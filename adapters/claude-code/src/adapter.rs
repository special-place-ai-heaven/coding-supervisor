//! One hook payload in, at most one decision out. The core's rules are authoritative on every call.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use coding_supervisor::{
    Candidate, Criterion, CriterionId, GateError, ReceiptStatus, Task, TaskContract, TaskState,
    VerificationReceipt,
};
use serde_json::Value;

use crate::candidate::{CandidateProvider, GitCandidate};
use crate::header::{self, Verdict};
use crate::journal::{self, Trace};
use crate::state::{self, Gate, Stored, obj, state_name};
use crate::{claim, guard, location};

/// Everything a hook call needs besides its payload.
pub struct Env {
    /// contract.json, gate.json, runs.jsonl, events.jsonl; see `location::resolve_state_dir`.
    pub state_dir: PathBuf,
    /// Which rule chose `state_dir`; journaled on every events.jsonl line.
    pub state_location: &'static str,
    pub candidate: Box<dyn CandidateProvider>,
    /// Seconds since the Unix epoch.
    pub clock: fn() -> u64,
}

impl Env {
    pub fn real(cwd: &Path) -> Self {
        let (state_dir, state_location) =
            location::resolve_state_dir(cwd, &|name| std::env::var(name).ok());
        Self {
            state_dir,
            state_location,
            candidate: Box::new(GitCandidate {
                cwd: cwd.to_path_buf(),
            }),
            clock: now,
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    None,
    /// `{"decision":"block","reason":...}` (Stop, SubagentStop).
    Block(String),
    /// PreToolUse `permissionDecision: "deny"`.
    Deny(String),
    /// `hookSpecificOutput.additionalContext`.
    Context {
        event: &'static str,
        text: String,
    },
    /// `{"systemMessage":...}`.
    System(String),
}

impl Decision {
    pub fn to_json(&self) -> Option<Value> {
        Some(match self {
            Self::None => return None,
            Self::Block(reason) => obj([
                ("decision", "block".into()),
                ("reason", reason.clone().into()),
            ]),
            Self::Deny(reason) => obj([(
                "hookSpecificOutput",
                obj([
                    ("hookEventName", "PreToolUse".into()),
                    ("permissionDecision", "deny".into()),
                    ("permissionDecisionReason", reason.clone().into()),
                ]),
            )]),
            Self::Context { event, text } => obj([(
                "hookSpecificOutput",
                obj([
                    ("hookEventName", (*event).into()),
                    ("additionalContext", text.clone().into()),
                ]),
            )]),
            Self::System(message) => obj([("systemMessage", message.clone().into())]),
        })
    }
}

pub fn fail_open(what: &str) -> Decision {
    // shortcut: fails open on adapter error, upgrade to fail-closed once the adapter has a month of clean journals
    Decision::System(format!("GATE adapter error: {what}"))
}

/// The binary's whole job: payload text in, decision JSON text out.
pub fn run_hook(input: &str) -> Option<String> {
    let decision = match serde_json::from_str::<Value>(input) {
        Err(e) => fail_open(&format!("payload is not JSON ({e}); not journaled")),
        Ok(payload) => match payload.get("cwd").and_then(Value::as_str) {
            Some(cwd) => handle(&payload, &mut Env::real(Path::new(cwd))),
            None => fail_open("payload has no cwd; not journaled"),
        },
    };
    decision.to_json().map(|v| v.to_string())
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub fn handle(payload: &Value, env: &mut Env) -> Decision {
    let event = text(payload, "hook_event_name").unwrap_or_default();
    let agent = text(payload, "agent_id").unwrap_or("main");
    let mut reasons = Vec::new();
    let mut trace = Trace::default();
    let decision =
        process(payload, env, event, agent, &mut reasons, &mut trace).unwrap_or_else(|what| {
            reasons.push(format!("adapter error: {what}"));
            fail_open(&what)
        });
    let line = journal::line(
        payload,
        env.state_location,
        (env.clock)(),
        decision.to_json(),
        &reasons,
        &trace,
    );
    match state::append_line(&env.state_dir.join("events.jsonl"), &line) {
        Ok(()) => decision,
        Err(what) => fail_open(&format!("events journal not written: {what}")),
    }
}

fn process(
    payload: &Value,
    env: &mut Env,
    event: &str,
    agent: &str,
    reasons: &mut Vec<String>,
    trace: &mut Trace,
) -> Result<Decision, String> {
    if event == "PreToolUse"
        && let Some(denial) = guard::state_write_denial(payload, &env.state_dir)
    {
        reasons.push("gate state path protected".to_owned());
        return Ok(denial);
    }
    let now = (env.clock)();
    if let Some(run) = observed_run(payload, event, agent, now) {
        state::append_line(&env.state_dir.join("runs.jsonl"), &run)?;
        reasons.push("command run journaled".to_owned());
    }
    let (contract, contract_sha256) = state::read_contract(&env.state_dir)?;
    trace.task_id = contract.as_ref().map(|c| c.id.0.clone());
    // A removed contract with a gate.json still present fails the chain as well.
    let stored = match state::load_gate(&env.state_dir, &contract_sha256)? {
        Stored::Tampered(what) => {
            trace.tampered = true;
            reasons.push(format!("state tampered: {what}"));
            return Ok(tampered(payload, event, &what));
        }
        Stored::Valid(gate) => Some(gate),
        Stored::Absent => None,
    };
    let Some(contract) = contract else {
        reasons.push("no contract: advisory only".to_owned());
        return Ok(no_contract(payload, event));
    };
    trace.state_before = stored.as_ref().map(|g| g.checkpoint.state);
    let observed = env.candidate.observe();
    trace.candidate = observed.as_ref().ok().map(|c| c.id.clone());
    if let Err(why) = &observed {
        reasons.push(format!("candidate not observed: {why}"));
    }
    let mut session = Session::open(contract, stored, observed, reasons)?;
    let decision = match event {
        "SubagentStop" => session.subagent_stop(payload, &env.state_dir, agent, now, reasons)?,
        "Stop" => session.stop(payload, reasons),
        "PreToolUse" => session.pre_tool_use(payload),
        "PostToolUse" if text(payload, "tool_name") == Some("Agent") => {
            session.agent_result(payload)
        }
        "UserPromptSubmit" => {
            session.gate.continuations = 0;
            session.gate.prompt_id = text(payload, "prompt_id").map(str::to_owned);
            Decision::None
        }
        _ => Decision::None,
    };
    session.gate.checkpoint = session.task.checkpoint();
    trace.state_after = Some(session.task.state());
    state::save_gate(&env.state_dir, &session.gate, &contract_sha256)?;
    Ok(decision)
}

/// Refuse everywhere a refusal is possible; gate.json is left exactly as found.
fn tampered(payload: &Value, event: &str, what: &str) -> Decision {
    let refusal = |subject: &str| {
        format!(
            "GATE: {subject} refused: gate state tampered ({what}); restore the state from the journal or start a new task"
        )
    };
    let tool = text(payload, "tool_name").unwrap_or_default();
    // The refusal counter lives in gate.json, which must stay untouched here, so the cap is
    // the harness's own continuation flag: block once per prompt, then release visibly
    // (the M2b live run otherwise ran to the turn limit).
    let continuing = payload
        .get("stop_hook_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match event {
        "PreToolUse"
            if matches!(
                tool,
                "Write" | "Edit" | "NotebookEdit" | "Agent" | "Bash" | "PowerShell"
            ) =>
        {
            Decision::Deny(refusal(tool))
        }
        "Stop" | "SubagentStop" if continuing => Decision::System(refusal(if event == "Stop" {
            "completion"
        } else {
            "handback"
        })),
        "Stop" => Decision::Block(refusal("completion")),
        "SubagentStop" => Decision::Block(refusal("handback")),
        "PostToolUse" if tool == "Agent" => Decision::Context {
            event: "PostToolUse",
            text: refusal("Agent result"),
        },
        _ => Decision::None,
    }
}

fn no_contract(payload: &Value, event: &str) -> Decision {
    let status = payload
        .get("tool_response")
        .and_then(|r| r.get("status"))
        .and_then(Value::as_str);
    let seen = match (event, text(payload, "tool_name"), status) {
        ("PostToolUse", Some("Agent"), Some("async_launched")) => "background dispatch",
        ("PostToolUse", Some("Agent"), Some("completed")) => "worker result",
        _ => return Decision::None,
    };
    Decision::Context {
        event: "PostToolUse",
        text: format!(
            "GATE (no contract): {seen} observed; there is no contract.json in the gate's state directory, so nothing is judged."
        ),
    }
}

/// A command run worth journaling: Bash/PowerShell results and Terminal Commander calls.
fn observed_run(payload: &Value, event: &str, agent: &str, now: u64) -> Option<Value> {
    let tool = text(payload, "tool_name")?;
    let input = payload.get("tool_input");
    let shell = matches!(tool, "Bash" | "PowerShell");
    let command = || {
        input
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let (command, exit_code) = match event {
        // A non-zero exit goes to PostToolUseFailure, so PostToolUse success means exit 0.
        "PostToolUse" if shell => (command()?, Some(0)),
        "PostToolUseFailure" if shell => (command()?, failure_exit(payload)),
        "PostToolUse" if tool.starts_with("mcp__terminal_commander__") => {
            (tc_command(input?)?, tc_exit(payload.get("tool_response")?))
        }
        _ => return None,
    };
    Some(obj([
        ("agent_id", agent.into()),
        ("tool", tool.into()),
        ("command", command.into()),
        ("cwd", text(payload, "cwd").into()),
        ("exit_code", exit_code.into()),
        ("tool_use_id", text(payload, "tool_use_id").into()),
        ("t", now.into()),
    ]))
}

fn failure_exit(payload: &Value) -> Option<i32> {
    text(payload, "error")?
        .lines()
        .next()?
        .trim()
        .strip_prefix("Exit code ")?
        .trim()
        .parse()
        .ok()
}

fn tc_command(input: &Value) -> Option<String> {
    if let Some(argv) = input.get("argv").and_then(Value::as_array) {
        return Some(
            argv.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    input
        .get("command")
        .or_else(|| input.get("script"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The integer after the first `"exit_code":` in the serialized response; also matches the
/// escaped form `\"exit_code\":` that appears when the response is JSON text inside JSON.
fn tc_exit(response: &Value) -> Option<i32> {
    let serialized = match response {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (_, after) = serialized.split_once("exit_code")?;
    let after = after.trim_start_matches(['"', '\\', ':', ' ']);
    let end = after
        .find(|c: char| c != '-' && !c.is_ascii_digit())
        .unwrap_or(after.len());
    after.get(..end)?.parse().ok()
}

pub fn render(error: &GateError) -> String {
    match error {
        GateError::MissingReceipt(c) => format!("missing {}", c.0),
        GateError::StaleReceipt(c) => format!("stale {}", c.0),
        GateError::FailingReceipt {
            criterion,
            exit_code,
        } => format!("failing {} exit {exit_code}", criterion.0),
        GateError::OutOfScope(paths) => format!("out of scope {}", paths.join(", ")),
        GateError::OpenFinding(f) => format!("open finding {}", f.0),
        GateError::InvalidTransition { from, event } => {
            format!("{event} is not allowed in state {}", state_name(*from))
        }
        GateError::CandidateNotObserved => "candidate identity not observed".to_owned(),
        GateError::ReceiptMissingExitCode => "receipt has no exit code".to_owned(),
        GateError::ReceiptUnsupported => "receipt is not linked to an observed run".to_owned(),
        GateError::UnknownCriterion(c) => format!("unknown criterion {}", c.0),
        GateError::ReceiptForOtherCandidate { expected, got } => format!(
            "receipt is for candidate {} but the current one is {}",
            got.dirty_digest, expected.dirty_digest
        ),
        GateError::UnknownFinding(f) => format!("unknown finding {}", f.0),
        GateError::RepairBudgetExhausted { max } => {
            format!("repair budget exhausted after {max} rounds")
        }
    }
}

/// The core task and gate.json for one call, rebuilt from contract.json, gate.json and the observed candidate.
pub struct Session {
    pub task: Task,
    pub gate: Gate,
    criteria: Vec<Criterion>,
    observed: Result<Candidate, String>,
}

impl Session {
    pub fn open(
        contract: TaskContract,
        stored: Option<Gate>,
        observed: Result<Candidate, String>,
        reasons: &mut Vec<String>,
    ) -> Result<Self, String> {
        let criteria = contract.criteria.clone();
        let contract_id = contract.id.0.clone();
        let (task, gate) = match stored {
            None => {
                let mut task = Task::new(contract);
                task.start_preparing().map_err(|e| render(&e))?;
                if let Ok(current) = &observed {
                    task.observe_candidate(current.clone())
                        .map_err(|e| render(&e))?;
                }
                let gate = Gate::new(contract_id, task.checkpoint());
                (task, gate)
            }
            Some(gate) => {
                if gate.contract_id != contract_id {
                    return Err(format!(
                        "gate.json belongs to contract {} but contract.json is {contract_id}",
                        gate.contract_id
                    ));
                }
                let mut task = Task::resume(contract, gate.checkpoint.clone());
                if let Ok(current) = &observed {
                    restore(&mut task, &gate, current, reasons).map_err(|e| render(&e))?;
                }
                (task, gate)
            }
        };
        Ok(Self {
            task,
            gate,
            criteria,
            observed,
        })
    }

    fn subagent_stop(
        &mut self,
        payload: &Value,
        dir: &Path,
        agent: &str,
        now: u64,
        reasons: &mut Vec<String>,
    ) -> Result<Decision, String> {
        let message = text(payload, "last_assistant_message").unwrap_or_default();
        let active = payload
            .get("stop_hook_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let prior = self.gate.handback_refusals.get(agent).copied().unwrap_or(0);
        let Some(header) = header::parse(message) else {
            if prior > 0 {
                reasons.push("no header; released after one refusal".to_owned());
                return Ok(Decision::None);
            }
            self.gate.handback_refusals.insert(agent.to_owned(), 1);
            return Ok(Decision::Block(
                "GATE: handback refused: no header. Start the final message with the line VERDICT: PASS, FAIL or UNVERIFIED, then CHECKS RUN: <n>, then one line per check run in this session: CHECK <id> | <command> | exit <code>.".to_owned(),
            ));
        };
        let mut problems = Vec::new();
        if !header.consistent {
            problems.push(format!(
                "header says CHECKS RUN {} but has {} well-formed CHECK lines",
                header.checks_run,
                header.checks.len()
            ));
        }
        let runs = state::read_runs(dir)?;
        let mut used: HashSet<String> = self
            .gate
            .receipts
            .iter()
            .map(|r| r.output_ref.clone())
            .collect();
        let cwd = text(payload, "cwd").unwrap_or_default();
        for line in &header.checks {
            let cited = format!(
                "CHECK {} | {} | exit {}",
                line.criterion, line.command, line.exit_code
            );
            let run = runs.iter().find(|r| {
                r.agent_id == agent
                    && r.command.trim() == line.command.trim()
                    && r.exit_code == Some(line.exit_code)
                    && !used.contains(&r.reference)
            });
            let (output_ref, at) = match run {
                Some(r) => {
                    used.insert(r.reference.clone());
                    (r.reference.clone(), r.t)
                }
                None => {
                    problems.push(format!(
                        "unlinked {cited} (no run by agent {agent} with that command and exit code)"
                    ));
                    (String::new(), now)
                }
            };
            let Ok(current) = &self.observed else {
                reasons.push(format!("not recorded, candidate not observed: {cited}"));
                continue;
            };
            let receipt = VerificationReceipt {
                criterion: CriterionId(line.criterion.clone()),
                candidate: current.id.clone(),
                command: line.command.clone(),
                cwd: line.cwd.clone().unwrap_or_else(|| cwd.to_owned()),
                exit_code: Some(line.exit_code),
                output_ref,
                claim_supported: run.is_some(),
                started_at: at,
                finished_at: at,
            };
            match self.task.record_receipt(receipt.clone()) {
                Ok(()) => {
                    reasons.push(format!("recorded {cited}"));
                    self.gate.receipts.push(receipt);
                }
                Err(e) => reasons.push(format!("rejected {cited}: {}", render(&e))),
            }
        }
        if header.verdict != Verdict::Pass {
            reasons.push("verdict is not PASS; journaled, not judged".to_owned());
            return Ok(Decision::None);
        }
        match self.task.try_accept() {
            Ok(()) => reasons.push("accepted: ready_for_review".to_owned()),
            Err(
                e @ (GateError::MissingReceipt(_)
                | GateError::StaleReceipt(_)
                | GateError::FailingReceipt { .. }
                | GateError::OutOfScope(_)),
            ) => problems.push(render(&e)),
            Err(e) => reasons.push(format!("try_accept: {}", render(&e))),
        }
        if problems.is_empty() {
            return Ok(Decision::None);
        }
        reasons.extend(problems.iter().cloned());
        if active && prior >= 3 {
            // Silent release: additionalContext on SubagentStop keeps the worker running
            // (measured in the L1 live run), so the cap must answer with nothing. The
            // chair learns the outcome from the PostToolUse annotation.
            reasons.push(format!(
                "handback recorded as UNVERIFIED after 3 refusals: {}",
                problems.join("; ")
            ));
            return Ok(Decision::None);
        }
        let k = prior + 1;
        self.gate.handback_refusals.insert(agent.to_owned(), k);
        Ok(Decision::Block(format!(
            "GATE: handback refused (refusal {k} of 3): {}. Run the check in this session and cite it as CHECK <id> | <command> | exit <code>.",
            problems.join("; ")
        )))
    }

    fn stop(&mut self, payload: &Value, reasons: &mut Vec<String>) -> Decision {
        let message = text(payload, "last_assistant_message").unwrap_or_default();
        let active = payload
            .get("stop_hook_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !claim::claims_completion(message) {
            reasons.push("no completion claim".to_owned());
            return Decision::None;
        }
        let result = match self.task.state() {
            TaskState::Completed => Ok(()),
            TaskState::ReadyForReview => self.task.complete(),
            _ => self.task.try_accept().and_then(|()| self.task.complete()),
        };
        let state = state_name(self.task.state());
        match result {
            Ok(()) => {
                reasons.push("completed".to_owned());
                Decision::None
            }
            Err(e) => {
                reasons.push(render(&e));
                if active && self.gate.continuations >= 3 {
                    return Decision::System(format!(
                        "GATE: refusal cap reached; task remains {state}"
                    ));
                }
                self.gate.continuations += 1;
                Decision::Block(format!(
                    "GATE refuses completion (refusal {} of 3, harness cap 8): state {state}; {}; record the check with CHECK lines or report the task as not complete.",
                    self.gate.continuations,
                    render(&e)
                ))
            }
        }
    }

    fn pre_tool_use(&mut self, payload: &Value) -> Decision {
        self.gate.continuations = 0;
        let tool = text(payload, "tool_name").unwrap_or_default();
        if !matches!(tool, "Write" | "Edit" | "Agent") {
            return Decision::None;
        }
        let state = self.task.state();
        if state.is_terminal() {
            return Decision::Deny(format!(
                "GATE: {tool} refused: task is {}",
                state_name(state)
            ));
        }
        match &self.observed {
            Err(why) => Decision::Deny(format!(
                "GATE: {tool} refused: candidate identity could not be observed ({why})"
            )),
            Ok(_) => Decision::None,
        }
    }

    fn agent_result(&self, payload: &Value) -> Decision {
        let status = payload
            .get("tool_response")
            .and_then(|r| r.get("status"))
            .and_then(Value::as_str);
        let text = match status {
            Some("async_launched") => {
                "GATE: background dispatch; the result is judged at SubagentStop, not here."
                    .to_owned()
            }
            Some("completed") => self.summary(),
            _ => return Decision::None,
        };
        Decision::Context {
            event: "PostToolUse",
            text,
        }
    }

    /// Receipt status per criterion, read without any transition.
    fn summary(&self) -> String {
        let (mut current, mut stale, mut missing) = (Vec::new(), Vec::new(), Vec::new());
        for c in &self.criteria {
            match self.task.receipt_status(&c.id) {
                Some(ReceiptStatus::Current) => current.push(c.id.0.as_str()),
                Some(ReceiptStatus::Stale) => stale.push(c.id.0.as_str()),
                None if c.mandatory => missing.push(c.id.0.as_str()),
                None => {}
            }
        }
        let state = self.task.state();
        let decision = if matches!(state, TaskState::ReadyForReview | TaskState::Completed) {
            "accepted".to_owned()
        } else if !missing.is_empty() {
            format!("not accepted (missing {})", missing.join(", "))
        } else if !stale.is_empty() {
            format!("not accepted (stale {})", stale.join(", "))
        } else {
            format!("not accepted (state {})", state_name(state))
        };
        format!(
            "GATE: state {}; current receipts: [{}]; missing: [{}]; stale: [{}]; decision: {decision}",
            state_name(state),
            current.join(", "),
            missing.join(", "),
            stale.join(", ")
        )
    }
}

/// Rebuild the receipt set of a resumed task, then observe the current candidate.
///
/// Stored receipts are replayed in journal order, grouped by the candidate they were bound to:
/// each group's identity is observed first, then its receipts are recorded. Every identity
/// change marks the earlier receipts stale, so after the final observation of the current
/// candidate the core itself reports `StaleReceipt` for evidence from an older candidate
/// instead of `MissingReceipt` (the L4 live run showed the difference).
fn restore(
    task: &mut Task,
    gate: &Gate,
    current: &Candidate,
    reasons: &mut Vec<String>,
) -> Result<(), GateError> {
    let previous = gate.checkpoint.candidate.clone();
    if previous.is_some() && !task.state().is_terminal() {
        let mut groups: Vec<(coding_supervisor::CandidateId, Vec<&VerificationReceipt>)> =
            Vec::new();
        for receipt in &gate.receipts {
            match groups.iter_mut().find(|(id, _)| *id == receipt.candidate) {
                Some((_, list)) => list.push(receipt),
                None => groups.push((receipt.candidate.clone(), vec![receipt])),
            }
        }
        if groups.is_empty()
            && let Some(id) = previous.clone()
        {
            groups.push((id, Vec::new()));
        }
        let (mut restored, mut rejected) = (0, 0);
        for (id, list) in groups {
            task.observe_candidate(Candidate {
                id,
                provenance: "journal".to_owned(),
                changed_paths: current.changed_paths.clone(),
            })?;
            for receipt in list {
                match task.record_receipt(receipt.clone()) {
                    Ok(()) => restored += 1,
                    Err(e) => {
                        rejected += 1;
                        reasons.push(format!("replay rejected: {}", render(&e)));
                    }
                }
            }
        }
        reasons.push(format!(
            "replay: {restored} receipts restored, {rejected} rejected"
        ));
    }
    task.observe_candidate(current.clone())?;
    if previous.as_ref() != Some(&current.id) {
        reasons.push("candidate changed since the checkpoint".to_owned());
    } else if gate.checkpoint.state == TaskState::ReadyForReview
        && task.state() == TaskState::Verifying
    {
        // Replaying an older candidate's receipts moved review back to verifying (L4 live
        // run); the identity is unchanged, so re-evaluate on the evidence that is current.
        match task.try_accept() {
            Ok(()) => reasons.push("review state restored after replay".to_owned()),
            Err(e) => reasons.push(format!("review not restored: {}", render(&e))),
        }
    }
    Ok(())
}
