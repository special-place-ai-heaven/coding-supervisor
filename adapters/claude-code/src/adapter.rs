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
use crate::header::{self, Header, Verdict};
use crate::journal::{self, Trace};
use crate::state::{self, Gate, Stored, obj, state_name};
use crate::{claim, guard, lifecycle, location, observation};

/// `output_ref` prefix of a receipt recorded through the MCP server: caller-asserted, never
/// observed by a hook, so the enforcing paths here do not count it.
pub const MCP_ASSERTED: &str = "mcp-asserted:";

/// True only when a hook completion journal backs every material receipt field.
pub fn receipt_is_observed(dir: &Path, task_id: &str, receipt: &VerificationReceipt) -> bool {
    receipt.claim_supported
        && observation::read_runs(dir, task_id).is_ok_and(|runs| {
            runs.iter().any(|run| {
                run.reference == receipt.output_ref
                    && run.command == receipt.command
                    && run.cwd == receipt.cwd
                    && run.candidate == receipt.candidate
                    && run.exit_code == receipt.exit_code
                    && run.t == receipt.started_at
                    && run.finished == receipt.finished_at
            })
        })
}

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
    let root = env.state_dir.clone();
    let event = text(payload, "hook_event_name").unwrap_or_default();
    if env.state_location != "test"
        && let Err(what) =
            location::prepare_state(&root, Path::new(text(payload, "cwd").unwrap_or(".")))
    {
        return match event {
            "PreToolUse" => Decision::Deny(format!("GATE: state unavailable: {what}")),
            "Stop" | "SubagentStop" => Decision::Block(format!("GATE: state unavailable: {what}")),
            _ => fail_open(&what),
        };
    }
    if event == "PreToolUse" {
        if let Some(denial) = guard::state_write_denial(payload, &root) {
            return denial;
        }
        let project = Path::new(text(payload, "cwd").unwrap_or("."));
        match crate::policy::action_policy_denial(payload, project, &root) {
            Ok(Some(denial)) => return denial,
            Err(_) => return Decision::Deny("GATE: invalid configured action policy".to_owned()),
            Ok(None) => {}
        }
    }
    let event_id = text(payload, "event_id").map(str::to_owned).or_else(|| {
        text(payload, "tool_use_id").map(|id| {
            format!(
                "{}:{event}:{id}",
                text(payload, "session_id").unwrap_or("legacy")
            )
        })
    });
    let candidate = env
        .candidate
        .observe()
        .map(|c| format!("{}:{}", c.id.base_commit, c.id.dirty_digest))
        .unwrap_or_else(|e| format!("unobserved:{e}"));
    let digest = crate::sha256::hex(format!("{payload}:{candidate}").as_bytes());
    if let Err(what) = crate::lifecycle::observe_connection(&root, "claude-hook") {
        return fail_open(&what);
    }
    let result = crate::lifecycle::with_task(
        &root,
        text(payload, "task_id"),
        text(payload, "session_id"),
        event_id.as_deref().map(|id| (id, digest.as_str())),
        |dir| {
            env.state_dir = dir.to_path_buf();
            let decision = handle_inner(payload, env);
            env.state_dir = root.clone();
            Ok(decision.to_json().unwrap_or(Value::Null))
        },
    );
    match result {
        Ok(Value::Null) => Decision::None,
        Ok(v) => decision_from_json(&v),
        Err(what) => match event {
            "PreToolUse" => Decision::Deny(format!("GATE: {what}")),
            "Stop" | "SubagentStop" => Decision::Block(format!("GATE: {what}")),
            _ => fail_open(&what),
        },
    }
}

fn decision_from_json(v: &Value) -> Decision {
    if let Some(message) = v.get("systemMessage").and_then(Value::as_str) {
        return Decision::System(message.to_owned());
    }
    if v.get("decision").and_then(Value::as_str) == Some("block") {
        return Decision::Block(text(v, "reason").unwrap_or_default().to_owned());
    }
    if let Some(output) = v.get("hookSpecificOutput") {
        if text(output, "permissionDecision") == Some("deny") {
            return Decision::Deny(
                text(output, "permissionDecisionReason")
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        return Decision::Context {
            event: "PostToolUse",
            text: text(output, "additionalContext")
                .unwrap_or_default()
                .to_owned(),
        };
    }
    fail_open("invalid cached hook response")
}

fn handle_inner(payload: &Value, env: &mut Env) -> Decision {
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
    let (contract, contract_sha256) = state::read_contract(&env.state_dir)?;
    trace.task_id = contract
        .as_ref()
        .map(|c| crate::lifecycle::task_identity(&env.state_dir, &c.id.0))
        .transpose()?;
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
    let task_id = lifecycle::task_identity(&env.state_dir, &contract.id.0)?;
    if observation::observe(
        &env.state_dir,
        payload,
        &task_id,
        agent,
        observed.as_ref().ok(),
        now,
        &contract.criteria,
    )? {
        reasons.push("command observation journaled".to_owned());
    }
    let mut session = Session::open_with_observed(
        contract,
        stored,
        observed,
        false,
        reasons,
        Some(&env.state_dir),
    )?;
    let decision = match event {
        "SubagentStop" => session.subagent_stop(payload, &env.state_dir, agent, now, reasons)?,
        "Stop" => session.stop(payload, &env.state_dir, agent, now, reasons)?,
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
        GateError::WrongCommand {
            criterion,
            cited: _,
            required,
        } => match required.split_first() {
            Some((first, [])) => format!(
                "{} cites an unapproved command but the contract requires `{first}`",
                criterion.0
            ),
            Some((first, rest)) => format!(
                "{} cites an unapproved command but the contract requires `{first}` or one of: {}",
                criterion.0,
                rest.iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => format!(
                "{} cites an unapproved command but the contract names no command for it",
                criterion.0
            ),
        },
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
    evidence_dir: Option<PathBuf>,
    task_id: String,
}

impl Session {
    /// `honour_asserted`: replay receipts whose `output_ref` starts with [`MCP_ASSERTED`].
    /// The hook adapter passes `false`; only the advisory MCP server passes `true`.
    pub fn open(
        contract: TaskContract,
        stored: Option<Gate>,
        observed: Result<Candidate, String>,
        honour_asserted: bool,
        reasons: &mut Vec<String>,
    ) -> Result<Self, String> {
        Self::open_with_observed(contract, stored, observed, honour_asserted, reasons, None)
    }

    fn open_with_observed(
        contract: TaskContract,
        stored: Option<Gate>,
        observed: Result<Candidate, String>,
        honour_asserted: bool,
        reasons: &mut Vec<String>,
        evidence_dir: Option<&Path>,
    ) -> Result<Self, String> {
        let criteria = contract.criteria.clone();
        let contract_id = contract.id.0.clone();
        let task_id = evidence_dir
            .map(|dir| crate::lifecycle::task_identity(dir, &contract_id))
            .transpose()?
            .unwrap_or_else(|| contract_id.clone());
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
                    restore(
                        &mut task,
                        &gate,
                        current,
                        honour_asserted,
                        reasons,
                        evidence_dir,
                        &task_id,
                    )
                    .map_err(|e| render(&e))?;
                }
                (task, gate)
            }
        };
        Ok(Self {
            task,
            gate,
            criteria,
            observed,
            evidence_dir: evidence_dir.map(Path::to_path_buf),
            task_id,
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
        let cwd = text(payload, "cwd").unwrap_or_default();
        let (mut problems, mut instructions) =
            self.link_checks(&header, dir, cwd, agent, now, reasons)?;
        if header.verdict != Verdict::Pass {
            reasons.push("verdict is not PASS; journaled, not judged".to_owned());
            return Ok(Decision::None);
        }
        self.note_asserted(reasons);
        let completed = self.task.state() == TaskState::Completed;
        let accepted = if completed {
            self.observed_evidence()
        } else {
            self.task.try_accept()
        };
        match accepted {
            Ok(()) if completed => reasons.push("completed: observed receipts current".to_owned()),
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
        if instructions.is_empty() {
            instructions.push(
                "Run the check in this session and cite it as CHECK <id> | <command> | exit <code>."
                    .to_owned(),
            );
        }
        Ok(Decision::Block(format!(
            "GATE: handback refused (refusal {k} of 3): {}. {}",
            problems.join("; "),
            instructions.join(" ")
        )))
    }

    /// Link each CHECK line to a journaled run by `agent` with the same command and exit code,
    /// and record it as a receipt. Shared by SubagentStop (a worker) and Stop (the chair, agent
    /// `main`). Returns the problems for a refusal and, per CHECK line citing a command the
    /// contract does not name, the instruction to run the right one.
    fn link_checks(
        &mut self,
        header: &Header,
        dir: &Path,
        cwd: &str,
        agent: &str,
        _now: u64,
        reasons: &mut Vec<String>,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let mut problems = Vec::new();
        let mut instructions = Vec::new();
        if !header.consistent {
            problems.push(format!(
                "header says CHECKS RUN {} but has {} well-formed CHECK lines",
                header.checks_run,
                header.checks.len()
            ));
        }
        let task_id = lifecycle::task_identity(dir, &self.gate.contract_id)?;
        let runs = observation::read_runs(dir, &task_id)?;
        let mut used: HashSet<String> = self
            .gate
            .receipts
            .iter()
            .map(|r| r.output_ref.clone())
            .collect();
        for line in &header.checks {
            let Some(criterion) = self.criteria.iter().find(|c| c.id.0 == line.criterion) else {
                let problem = "unlinked CHECK [unknown criterion]".to_owned();
                reasons.push(problem.clone());
                problems.push(problem);
                continue;
            };
            if !criterion.accepts(&line.command) {
                let problem = render(&GateError::WrongCommand {
                    criterion: criterion.id.clone(),
                    cited: String::new(),
                    required: criterion.command.clone(),
                });
                reasons.push(format!("unlinked CHECK {} | [unapproved command] | exit {} (claim_supported false: {problem})",criterion.id.0,line.exit_code));
                problems.push(problem);
                let first = criterion.command.first().map_or("", |s| s.trim());
                instructions.push(format!(
                    "Run that command in this session and cite it as CHECK {} | {first} | exit 0.",
                    criterion.id.0
                ));
                continue;
            }
            let cited = format!(
                "CHECK {} | {} | exit {}",
                line.criterion, line.command, line.exit_code
            );
            let run = runs.iter().find(|r| {
                r.agent_id == agent
                    && r.command.trim() == line.command.trim()
                    && r.exit_code == Some(line.exit_code)
                    && self.observed.as_ref().is_ok_and(|c| c.id == r.candidate)
                    && r.cwd == observation::cwd(line.cwd.as_deref().unwrap_or(cwd), cwd)
                    && !used.contains(&r.reference)
            });
            let (output_ref, at) = match run {
                Some(r) => {
                    used.insert(r.reference.clone());
                    (r.reference.clone(), r.t)
                }
                None => {
                    let problem = format!(
                        "unlinked {cited} (no run by agent {agent} with that command and exit code)"
                    );
                    reasons.push(problem.clone());
                    problems.push(problem);
                    continue;
                }
            };
            let Ok(current) = &self.observed else {
                reasons.push(format!("not recorded, candidate not observed: {cited}"));
                continue;
            };
            let receipt = VerificationReceipt {
                criterion: CriterionId(line.criterion.clone()),
                candidate: run.map_or_else(|| current.id.clone(), |r| r.candidate.clone()),
                command: line.command.clone(),
                cwd: run.map_or_else(
                    || line.cwd.clone().unwrap_or_else(|| cwd.to_owned()),
                    |r| r.cwd.clone(),
                ),
                exit_code: Some(line.exit_code),
                output_ref,
                claim_supported: run.is_some(),
                started_at: at,
                finished_at: run.map_or(at, |r| r.finished),
            };
            // The core takes no receipt once the task is completed (a chair may complete it
            // through MCP first). Keep a hook-linked one in gate.json, where
            // `observed_evidence` reads it; anything else is rejected as the core would.
            if self.task.state() == TaskState::Completed {
                if run.is_some() && self.criteria.iter().any(|c| c.id == receipt.criterion) {
                    reasons.push(format!(
                        "recorded {cited} (task completed; held as observed evidence)"
                    ));
                    self.gate.receipts.push(receipt);
                } else {
                    reasons.push(format!(
                        "rejected {cited}: {}",
                        render(&GateError::ReceiptUnsupported)
                    ));
                }
                continue;
            }
            match self.task.record_receipt(receipt.clone()) {
                Ok(()) => {
                    reasons.push(format!("recorded {cited}"));
                    self.gate.receipts.push(receipt);
                }
                Err(e) => reasons.push(format!("rejected {cited}: {}", render(&e))),
            }
        }
        Ok((problems, instructions))
    }

    fn stop(
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
        if !claim::claims_completion(message) {
            reasons.push("no completion claim".to_owned());
            return Ok(Decision::None);
        }
        // A chair working alone cites its own runs the way a worker does.
        let (problems, instructions) = match header::parse(message) {
            Some(header) => {
                let cwd = text(payload, "cwd").unwrap_or_default();
                self.link_checks(&header, dir, cwd, agent, now, reasons)?
            }
            None => (Vec::new(), Vec::new()),
        };
        self.note_asserted(reasons);
        let result = if problems.is_empty() {
            match self.task.state() {
                TaskState::Completed => self.observed_evidence(),
                TaskState::ReadyForReview => self.task.complete(),
                _ => self.task.try_accept().and_then(|()| self.task.complete()),
            }
            .map_err(|e| render(&e))
        } else {
            // An unlinked CHECK line refuses before any acceptance is tried.
            Err(problems.join("; "))
        };
        let state = state_name(self.task.state());
        match result {
            Ok(()) => {
                reasons.push("completed".to_owned());
                Ok(Decision::None)
            }
            Err(why) => {
                reasons.push(why.clone());
                if active && self.gate.continuations >= 3 {
                    return Ok(Decision::System(format!(
                        "GATE: refusal cap reached; task remains {state}"
                    )));
                }
                self.gate.continuations += 1;
                let run_instead: String = instructions.iter().map(|i| format!(" {i}")).collect();
                Ok(Decision::Block(format!(
                    "GATE refuses completion (refusal {} of 3, harness cap 8): state {state}; {why}; record the check with CHECK lines or report the task as not complete.{run_instead}",
                    self.gate.continuations
                )))
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

    /// Journal the caller-asserted receipts bound to the current candidate as unlinked CHECKs;
    /// called only where an acceptance decision is made, so `measure` counts each once there.
    fn note_asserted(&self, reasons: &mut Vec<String>) {
        let current = self.task.candidate();
        for r in &self.gate.receipts {
            if r.output_ref.starts_with(MCP_ASSERTED) && Some(&r.candidate) == current {
                let exit = r.exit_code.map_or("none".to_owned(), |c| c.to_string());
                reasons.push(format!(
                    "unlinked CHECK {} | {} | exit {exit} (mcp-asserted receipt; no hook observed the run)",
                    r.criterion.0, r.command
                ));
            }
        }
    }

    /// The core does not judge a task again once it is completed, and a task completed through
    /// MCP may rest on caller-asserted receipts. Check here, as `try_accept` would, that every
    /// mandatory criterion has a current, hook-observed receipt with exit 0.
    fn observed_evidence(&self) -> Result<(), GateError> {
        let current = self
            .task
            .candidate()
            .ok_or(GateError::CandidateNotObserved)?;
        for criterion in self.criteria.iter().filter(|c| c.mandatory) {
            let observed: Vec<&VerificationReceipt> = self
                .gate
                .receipts
                .iter()
                .filter(|r| {
                    r.criterion == criterion.id
                        && self
                            .evidence_dir
                            .as_ref()
                            .is_some_and(|dir| receipt_is_observed(dir, &self.task_id, r))
                })
                .collect();
            match observed.iter().rev().find(|r| &r.candidate == current) {
                Some(r) => match r.exit_code {
                    Some(0) => {}
                    Some(exit_code) => {
                        return Err(GateError::FailingReceipt {
                            criterion: criterion.id.clone(),
                            exit_code,
                        });
                    }
                    None => return Err(GateError::ReceiptMissingExitCode),
                },
                None if !observed.is_empty() => {
                    return Err(GateError::StaleReceipt(criterion.id.clone()));
                }
                None => return Err(GateError::MissingReceipt(criterion.id.clone())),
            }
        }
        Ok(())
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
///
/// Unless `honour_asserted`, a caller-asserted MCP receipt is left out of the replay: no hook
/// observed its command run. Stop and SubagentStop journal it (`Session::note_asserted`).
fn restore(
    task: &mut Task,
    gate: &Gate,
    current: &Candidate,
    honour_asserted: bool,
    reasons: &mut Vec<String>,
    evidence_dir: Option<&Path>,
    task_id: &str,
) -> Result<(), GateError> {
    let previous = gate.checkpoint.candidate.clone();
    if previous.is_some() && !task.state().is_terminal() {
        let mut groups: Vec<(coding_supervisor::CandidateId, Vec<&VerificationReceipt>)> =
            Vec::new();
        for receipt in &gate.receipts {
            if !honour_asserted
                && !evidence_dir.is_some_and(|dir| receipt_is_observed(dir, task_id, receipt))
            {
                continue;
            }
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
