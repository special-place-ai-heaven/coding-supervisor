//! One hook payload in, at most one decision out. The core's rules are authoritative on every call.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use coding_supervisor::{
    Candidate, CandidateId, Criterion, CriterionId, GateError, ReceiptStatus, Task, TaskContract,
    TaskState, VerificationReceipt,
};
use serde_json::Value;

use crate::candidate::{CandidateProvider, GitCandidate};
use crate::header::{self, CheckLine, Header, Verdict};
use crate::journal::{self, Trace};
use crate::level::{self, Level};
use crate::state::{self, Gate, Stored, obj, state_name};
use crate::{claim, guard, location};

/// `output_ref` prefix of a receipt recorded through the MCP server: caller-asserted, never
/// observed by a hook, so the enforcing paths here do not count it.
pub const MCP_ASSERTED: &str = "mcp-asserted:";

/// Everything a hook call needs besides its payload.
pub struct Env {
    /// contract.json, gate.json, runs.jsonl, events.jsonl; see `location::resolve_state_dir`.
    pub state_dir: PathBuf,
    /// Which rule chose `state_dir`; journaled on every events.jsonl line.
    pub state_location: &'static str,
    pub candidate: Box<dyn CandidateProvider>,
    /// Seconds since the Unix epoch.
    pub clock: fn() -> u64,
    /// From the level files (`level::effective`); a degraded session lowers it to lite.
    pub level: Level,
    /// Why the level files could not be read; the call then runs at full and journals this.
    pub level_error: Option<String>,
    /// The user config dir holding the global level file (`level::config_dir`); agents may not
    /// write it.
    pub config_dir: Option<PathBuf>,
}

impl Env {
    pub fn real(cwd: &Path) -> Self {
        let lookup = |name: &str| std::env::var(name).ok();
        // Located without creating anything: at level off nothing is written.
        let (dir, location) = location::state_dir_path(cwd, &lookup);
        let config_dir = level::config_dir(&lookup);
        let (level, level_error) = match level::effective(&dir, config_dir.as_deref()) {
            Ok((level, _)) => (level, None),
            Err(e) => (
                Level::Full,
                Some(format!("level not read, running at full: {e}")),
            ),
        };
        let (state_dir, state_location) = if level == Level::Off {
            (dir, location)
        } else {
            location::resolve_state_dir(cwd, &lookup)
        };
        Self {
            state_dir,
            state_location,
            candidate: Box::new(GitCandidate {
                cwd: cwd.to_path_buf(),
            }),
            clock: now,
            level,
            level_error,
            config_dir,
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
    if env.level == Level::Off {
        // Nothing judged, nothing written; only the state-path guard still answers.
        return match event {
            "PreToolUse" => {
                guard::gate_write_denial(payload, &env.state_dir, env.config_dir.as_deref())
                    .unwrap_or(Decision::None)
            }
            _ => Decision::None,
        };
    }
    let agent = text(payload, "agent_id").unwrap_or("main");
    let mut reasons = Vec::new();
    if let Some(error) = &env.level_error {
        reasons.push(error.clone());
    }
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
        && let Some(denial) =
            guard::gate_write_denial(payload, &env.state_dir, env.config_dir.as_deref())
    {
        reasons.push("gate state path protected".to_owned());
        return Ok(denial);
    }
    let now = (env.clock)();
    // Observed before journaling so each run record carries the candidate it ran against.
    let observed = env.candidate.observe();
    let ran_on = observed.as_ref().ok().map(|c| &c.id);
    let run = observed_run(payload, event, agent, ran_on, now);
    if let Some(run) = &run {
        state::append_line(&env.state_dir.join("runs.jsonl"), run)?;
        reasons.push("command run journaled".to_owned());
    }
    let (contract, contract_sha256) = state::read_contract_tiers(&env.state_dir)?;
    trace.task_id = contract.as_ref().map(|(c, _)| c.id.0.clone());
    // A removed contract with a gate.json still present fails the chain as well.
    let stored = match state::load_gate(&env.state_dir, &contract_sha256)? {
        Stored::Tampered(what) => {
            trace.tampered = true;
            reasons.push(format!("state tampered: {what}"));
            let refusal = tampered(payload, event, &what);
            return Ok(if env.level == Level::Lite {
                soften(refusal)
            } else {
                refusal
            });
        }
        Stored::Valid(gate) => Some(gate),
        Stored::Absent => None,
    };
    let Some((mut contract, full_tier)) = contract else {
        reasons.push("no contract: advisory only".to_owned());
        return Ok(no_contract(payload, event));
    };
    let sid = text(payload, "session_id").unwrap_or_default();
    let degraded = stored
        .as_ref()
        .and_then(|g| g.sessions.get(sid))
        .and_then(|s| s.degraded.as_deref());
    let level = match degraded {
        Some(why) => {
            reasons.push(format!("session degraded, running at lite at most: {why}"));
            env.level.min(Level::Lite)
        }
        None => env.level,
    };
    if level < Level::Ultra {
        // A full-tier criterion is required at ultra only.
        for c in &mut contract.criteria {
            if full_tier.contains(&c.id.0) {
                c.mandatory = false;
            }
        }
    }
    trace.state_before = stored.as_ref().map(|g| g.checkpoint.state);
    trace.candidate = observed.as_ref().ok().map(|c| c.id.clone());
    if let Err(why) = &observed {
        reasons.push(format!("candidate not observed: {why}"));
    }
    let mut session = Session::open(contract, stored, observed, false, reasons)?;
    session.track_session(sid, event, run.as_ref(), reasons);
    let counters = (
        session.gate.continuations,
        session.gate.handback_refusals.clone(),
    );
    let decision = match event {
        "SubagentStop" => session.subagent_stop(payload, &env.state_dir, agent, now, reasons)?,
        "Stop" => {
            let judge_unclaimed = level == Level::Ultra && session.changed_in_session(sid);
            session.stop(
                payload,
                &env.state_dir,
                agent,
                now,
                judge_unclaimed,
                reasons,
            )?
        }
        "PreToolUse" => session.pre_tool_use(payload, level == Level::Ultra),
        "PostToolUse" if text(payload, "tool_name") == Some("Agent") => {
            session.agent_result(payload)
        }
        "UserPromptSubmit" => {
            session.gate.continuations = 0;
            session.gate.prompt_id = text(payload, "prompt_id").map(str::to_owned);
            match session
                .gate
                .sessions
                .get_mut(sid)
                .and_then(|s| s.notice.take())
            {
                Some(text) => Decision::Context {
                    event: "UserPromptSubmit",
                    text,
                },
                None => Decision::None,
            }
        }
        _ => Decision::None,
    };
    let decision = match decision {
        Decision::Block(text) | Decision::Deny(text) if level == Level::Lite => {
            reasons.push("level lite: reported, not blocked".to_owned());
            if matches!(event, "Stop" | "SubagentStop") {
                // Lite counts no refusal; the model reads the notice at its next prompt.
                (session.gate.continuations, session.gate.handback_refusals) = counters;
                session
                    .gate
                    .sessions
                    .entry(sid.to_owned())
                    .or_default()
                    .notice = Some(format!(
                    "GATE (lite, not blocking) at the last stop: {text}"
                ));
            }
            Decision::System(text)
        }
        other => other,
    };
    if level != Level::Full {
        reasons.push(format!("level {}", level.name()));
    }
    session.gate.checkpoint = session.task.checkpoint();
    trace.state_after = Some(session.task.state());
    state::save_gate(&env.state_dir, &session.gate, &contract_sha256)?;
    Ok(decision)
}

/// Lite reports what full would refuse, without blocking.
fn soften(decision: Decision) -> Decision {
    match decision {
        Decision::Block(text) | Decision::Deny(text) => Decision::System(text),
        other => other,
    }
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
/// `candidate` is the candidate observed at the run (null when it could not be observed).
fn observed_run(
    payload: &Value,
    event: &str,
    agent: &str,
    candidate: Option<&CandidateId>,
    now: u64,
) -> Option<Value> {
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
        ("candidate", candidate.map(state::candidate_json).into()),
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
        GateError::WrongCommand {
            criterion,
            cited,
            required,
        } => match required.split_first() {
            Some((first, [])) => format!(
                "{} cites `{cited}` but the contract requires `{first}`",
                criterion.0
            ),
            Some((first, rest)) => format!(
                "{} cites `{cited}` but the contract requires `{first}` or one of: {}",
                criterion.0,
                rest.iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => format!(
                "{} cites `{cited}` but the contract names no command for it",
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
    scope: Vec<String>,
    observed: Result<Candidate, String>,
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
        let criteria = contract.criteria.clone();
        let scope = contract.scope.clone();
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
                    restore(&mut task, &gate, current, honour_asserted, reasons)
                        .map_err(|e| render(&e))?;
                }
                (task, gate)
            }
        };
        Ok(Self {
            task,
            gate,
            criteria,
            scope,
            observed,
        })
    }

    /// The session's baseline is the candidate at its first UserPromptSubmit (Claude Code sends
    /// it before any tool call). A run of a mandatory criterion's command that exits nonzero on
    /// that baseline marks the session degraded: it then runs at lite at most.
    fn track_session(
        &mut self,
        sid: &str,
        event: &str,
        run: Option<&Value>,
        reasons: &mut Vec<String>,
    ) {
        let current = self.observed.as_ref().ok().map(|c| c.id.clone());
        let record = self.gate.sessions.entry(sid.to_owned()).or_default();
        if event == "UserPromptSubmit" && record.baseline.is_none() {
            record.baseline.clone_from(&current);
        }
        let Some(run) = run else { return };
        if record.degraded.is_some() || record.baseline.is_none() || current != record.baseline {
            return;
        }
        let command = run
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let exit = run.get("exit_code").and_then(Value::as_i64);
        if let Some(code) = exit.filter(|c| *c != 0)
            && self
                .criteria
                .iter()
                .any(|c| c.mandatory && c.accepts(command))
        {
            let why = format!(
                "`{}` exited {code} on the untouched tree at session start",
                command.trim()
            );
            reasons.push(format!("session degraded: {why}"));
            record.degraded = Some(why);
        }
    }

    /// The current candidate differs from the session's baseline and from the last acceptance.
    fn changed_in_session(&self, sid: &str) -> bool {
        let current = self.observed.as_ref().ok().map(|c| &c.id);
        let baseline = self
            .gate
            .sessions
            .get(sid)
            .and_then(|s| s.baseline.as_ref());
        current.is_some()
            && baseline.is_some()
            && current != baseline
            && current != self.gate.accepted_candidate.as_ref()
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
        let (header, to_run) = match header::parse(message) {
            Some(header) => (header, None),
            // A completion claim without a header is judged on the agent's observed runs.
            None if claim::claims_completion(message) => {
                let (header, to_run) = self.derive(dir, agent, reasons)?;
                (header, Some(to_run))
            }
            None if prior > 0 => {
                reasons.push("no header; released after one refusal".to_owned());
                return Ok(Decision::None);
            }
            None => {
                self.gate.handback_refusals.insert(agent.to_owned(), 1);
                return Ok(Decision::Block(
                    "GATE: handback refused: no header. Start the final message with the line VERDICT: PASS, FAIL or UNVERIFIED, then CHECKS RUN: <n>, then one line per check run in this session: CHECK <id> | <command> | exit <code>.".to_owned(),
                ));
            }
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
        match to_run {
            Some(to_run) => instructions = to_run.into_iter().map(|r| format!("{r}.")).collect(),
            None if instructions.is_empty() => instructions.push(
                "Run the check in this session and cite it as CHECK <id> | <command> | exit <code>."
                    .to_owned(),
            ),
            None => {}
        }
        Ok(Decision::Block(
            format!(
                "GATE: handback refused (refusal {k} of 3): {}. {}",
                problems.join("; "),
                instructions.join(" ")
            )
            .trim_end()
            .to_owned(),
        ))
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
        now: u64,
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
        let runs = state::read_runs(dir)?;
        let mut used: HashSet<String> = self
            .gate
            .receipts
            .iter()
            .map(|r| r.output_ref.clone())
            .collect();
        for line in &header.checks {
            let cited = format!(
                "CHECK {} | {} | exit {}",
                line.criterion, line.command, line.exit_code
            );
            // The core enforces the same rule; checking it here keeps the run unlinked and names the command.
            let wrong = self
                .criteria
                .iter()
                .find(|c| c.id.0 == line.criterion && !c.accepts(&line.command));
            let current_id = self.observed.as_ref().ok().map(|c| &c.id);
            let matches = |r: &&state::Run| {
                wrong.is_none()
                    && r.agent_id == agent
                    && r.command.trim() == line.command.trim()
                    && r.exit_code == Some(line.exit_code)
                    && !used.contains(&r.reference)
            };
            // A run records the candidate it ran against; one from an older candidate (or an
            // old record without one) never links.
            let run = runs
                .iter()
                .find(|r| matches(r) && current_id.is_some() && r.candidate.as_ref() == current_id);
            let earlier = run.is_none() && runs.iter().any(|r| matches(&r));
            let (output_ref, at) = match (run, wrong) {
                (Some(r), _) => {
                    used.insert(r.reference.clone());
                    (r.reference.clone(), r.t)
                }
                (None, Some(c)) => {
                    let problem = render(&GateError::WrongCommand {
                        criterion: c.id.clone(),
                        cited: line.command.trim().to_owned(),
                        required: c.command.clone(),
                    });
                    reasons.push(format!(
                        "unlinked {cited} (claim_supported false: {problem})"
                    ));
                    problems.push(problem);
                    let first = c.command.first().map_or("", |s| s.trim());
                    instructions.push(format!(
                        "Run that command in this session and cite it as CHECK {} | {first} | exit 0.",
                        c.id.0
                    ));
                    (String::new(), now)
                }
                (None, None) if earlier => {
                    let command = line.command.trim();
                    problems.push(format!(
                        "{} `{command}` ran before your last change; run it again",
                        line.criterion
                    ));
                    instructions.push(format!(
                        "Run `{command}` again in this session, then cite the new run as CHECK {} | {command} | exit <code>.",
                        line.criterion
                    ));
                    (String::new(), now)
                }
                (None, None) => {
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

    /// `judge_unclaimed` (ultra, after a change in this session): judge the stop as a
    /// completion claim whatever the message says.
    fn stop(
        &mut self,
        payload: &Value,
        dir: &Path,
        agent: &str,
        now: u64,
        judge_unclaimed: bool,
        reasons: &mut Vec<String>,
    ) -> Result<Decision, String> {
        let message = text(payload, "last_assistant_message").unwrap_or_default();
        let active = payload
            .get("stop_hook_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !claim::claims_completion(message) {
            if !judge_unclaimed {
                reasons.push("no completion claim".to_owned());
                return Ok(Decision::None);
            }
            reasons.push("no completion claim; judged at level ultra after a change".to_owned());
        }
        // A chair working alone cites its own runs the way a worker does; without a header
        // the citation is derived from those runs.
        let (header, to_run) = match header::parse(message) {
            Some(header) => (header, None),
            None => {
                let (header, to_run) = self.derive(dir, agent, reasons)?;
                (header, Some(to_run))
            }
        };
        let cwd = text(payload, "cwd").unwrap_or_default();
        let (problems, instructions) = self.link_checks(&header, dir, cwd, agent, now, reasons)?;
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
                self.gate.accepted_candidate = self.task.candidate().cloned();
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
                let tail = match to_run {
                    Some(to_run) => to_run
                        .iter()
                        .map(|r| format!("{r}; "))
                        .chain(["or report the task as not complete.".to_owned()])
                        .collect::<String>(),
                    None => {
                        let run_instead: String =
                            instructions.iter().map(|i| format!(" {i}")).collect();
                        format!(
                            "record the check with CHECK lines or report the task as not complete.{run_instead}"
                        )
                    }
                };
                Ok(Decision::Block(format!(
                    "GATE refuses completion (refusal {} of 3, harness cap 8): state {state}; {why}; {tail}",
                    self.gate.continuations
                )))
            }
        }
    }

    /// For a completion claim without a header: per criterion, a CHECK line for the latest run by
    /// `agent` of one of its commands that exited 0 on the current candidate and backs no receipt
    /// yet. Also, per mandatory criterion left without a current passing hook-observed receipt,
    /// what to run (the criterion's first command) and why.
    fn derive(
        &self,
        dir: &Path,
        agent: &str,
        reasons: &mut Vec<String>,
    ) -> Result<(Header, Vec<String>), String> {
        let runs = state::read_runs(dir)?;
        let current = self.observed.as_ref().ok().map(|c| &c.id);
        // `link_checks` links one run to one CHECK line; derive the same way.
        let mut used: HashSet<&str> = self
            .gate
            .receipts
            .iter()
            .map(|r| r.output_ref.as_str())
            .collect();
        let (mut checks, mut to_run) = (Vec::new(), Vec::new());
        for c in &self.criteria {
            // The latest run on the current candidate decides, whatever its exit code.
            let latest = runs.iter().rev().find(|r| {
                r.agent_id == agent
                    && c.accepts(&r.command)
                    && current.is_some()
                    && r.candidate.as_ref() == current
            });
            if let Some(r) = latest
                && r.exit_code == Some(0)
                && !used.contains(r.reference.as_str())
            {
                used.insert(&r.reference);
                checks.push(CheckLine {
                    criterion: c.id.0.clone(),
                    command: r.command.clone(),
                    cwd: None,
                    exit_code: 0,
                });
                continue;
            }
            let held = self
                .gate
                .receipts
                .iter()
                .rev()
                .find(|r| {
                    r.criterion == c.id
                        && !r.output_ref.starts_with(MCP_ASSERTED)
                        && Some(&r.candidate) == current
                })
                .is_some_and(|r| r.exit_code == Some(0));
            if !c.mandatory {
                continue;
            }
            if held {
                // A stored pass on this tree yields to a later failing run of the same command:
                // cite that run, so the core records the failure and refuses.
                match latest {
                    Some(r) if !used.contains(r.reference.as_str()) => {
                        if let Some(n) = r.exit_code.filter(|n| *n != 0) {
                            used.insert(&r.reference);
                            checks.push(CheckLine {
                                criterion: c.id.0.clone(),
                                command: r.command.clone(),
                                cwd: None,
                                exit_code: n,
                            });
                        } else {
                            continue;
                        }
                    }
                    _ => continue,
                }
            }
            let first = c.command.first().map_or("", |s| s.trim());
            let why = match latest {
                Some(r) => match r.exit_code {
                    Some(0) => "its run already backs another check; run it again".to_owned(),
                    Some(n) => format!("exited {n} on your latest run"),
                    None => "its exit code was not observed".to_owned(),
                },
                // No run on the current candidate: one of the first command on another candidate is stale.
                None if runs
                    .iter()
                    .any(|r| r.agent_id == agent && r.command.trim() == first) =>
                {
                    "ran before your last change".to_owned()
                }
                None => "never ran".to_owned(),
            };
            to_run.push(format!("run `{first}`: {why}"));
        }
        reasons.push(format!(
            "header derived from observed runs of agent {agent}: {} CHECK lines",
            checks.len()
        ));
        Ok((
            Header {
                verdict: Verdict::Pass,
                checks_run: checks.len(),
                checks,
                consistent: true,
            },
            to_run,
        ))
    }

    fn pre_tool_use(&mut self, payload: &Value, ultra: bool) -> Decision {
        self.gate.continuations = 0;
        let tool = text(payload, "tool_name").unwrap_or_default();
        if matches!(tool, "Write" | "Edit" | "Agent") {
            let state = self.task.state();
            if state.is_terminal() {
                return Decision::Deny(format!(
                    "GATE: {tool} refused: task is {}",
                    state_name(state)
                ));
            }
            if let Err(why) = &self.observed {
                return Decision::Deny(format!(
                    "GATE: {tool} refused: candidate identity could not be observed ({why})"
                ));
            }
        }
        if ultra && matches!(tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit") {
            return self.scope_denial(payload, tool).unwrap_or(Decision::None);
        }
        Decision::None
    }

    /// Ultra: a write outside the contract scope, matched as the core matches changed paths
    /// (project-relative prefix). An empty scope, or one naming the whole project, allows all.
    fn scope_denial(&self, payload: &Value, tool: &str) -> Option<Decision> {
        if self.scope.is_empty()
            || self
                .scope
                .iter()
                .any(|p| matches!(p.trim(), "" | "." | "./" | "/"))
        {
            return None;
        }
        let input = payload.get("tool_input")?;
        let path = input
            .get("file_path")
            .or_else(|| input.get("notebook_path"))
            .and_then(Value::as_str)?;
        let relative = project_relative(path, text(payload, "cwd").unwrap_or_default());
        if relative.is_some_and(|r| self.scope.iter().any(|p| r.starts_with(p.as_str()))) {
            return None;
        }
        Some(Decision::Deny(format!(
            "GATE: {tool} refused: {path} is out of scope; the contract scope is {} (level ultra)",
            self.scope.join(", ")
        )))
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
                .filter(|r| r.criterion == criterion.id && !r.output_ref.starts_with(MCP_ASSERTED))
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

/// `path` relative to the project `cwd`, with `/` separators; `None` when it lies outside.
/// The cwd prefix is compared ASCII-case-insensitively (Windows drive and folder casing).
fn project_relative(path: &str, cwd: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/') || path.get(1..2) == Some(":");
    if !absolute {
        return Some(path.trim_start_matches("./").to_owned());
    }
    let cwd = cwd.replace('\\', "/");
    let cwd = cwd.trim_end_matches('/');
    let head = path.get(..cwd.len())?;
    let rest = path.get(cwd.len()..)?.strip_prefix('/')?;
    (!cwd.is_empty() && head.eq_ignore_ascii_case(cwd)).then(|| rest.to_owned())
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
) -> Result<(), GateError> {
    let previous = gate.checkpoint.candidate.clone();
    if previous.is_some() && !task.state().is_terminal() {
        let mut groups: Vec<(coding_supervisor::CandidateId, Vec<&VerificationReceipt>)> =
            Vec::new();
        for receipt in &gate.receipts {
            if !honour_asserted && receipt.output_ref.starts_with(MCP_ASSERTED) {
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
