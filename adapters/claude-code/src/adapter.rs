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
        && current_dispatch_reference(dir, task_id, &receipt.output_ref).unwrap_or(false)
        && observation::read_runs(dir, task_id).is_ok_and(|runs| {
            runs.iter().any(|run| {
                run.cwd == run.execution_root
                    && run.reference == receipt.output_ref
                    && run.command == receipt.command
                    && run.cwd == receipt.cwd
                    && run.candidate == receipt.candidate
                    && run.exit_code == receipt.exit_code
                    && run.t == receipt.started_at
                    && run.finished == receipt.finished_at
            })
        })
}
/// Preserve dispatch IDs verbatim; they may themselves contain colons.
fn current_dispatch_reference(dir: &Path, task: &str, reference: &str) -> Result<bool, String> {
    for prefix in [
        format!("observed:{task}:"),
        format!("tc-reconstructed:{task}:"),
    ] {
        if let Some(dispatch) = reference.strip_prefix(&prefix) {
            return crate::autonomy::dispatch_is_current(dir, dispatch);
        }
    }
    Ok(false)
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

/// Conservative read-only routes remain usable while the current prompt awaits a contract.
fn read_only_tool(payload: &Value) -> bool {
    let tool = text(payload, "tool_name").unwrap_or_default();
    if matches!(
        tool,
        "Read" | "Glob" | "Grep" | "LS" | "TaskGet" | "TaskList"
    ) {
        return true;
    }
    if let Some(name) = tool.strip_prefix("mcp__symforge__") {
        return name.starts_with("get_")
            || name.starts_with("search_")
            || name.starts_with("find_")
            || matches!(
                name,
                "status"
                    | "health"
                    | "health_compact"
                    | "conventions"
                    | "edit_plan"
                    | "what_changed"
                    | "inspect_match"
                    | "symforge_retrieve"
            );
    }
    if let Some(name) = tool.strip_prefix("mcp__terminal_commander__") {
        return matches!(
            name,
            "command_status"
                | "bucket_wait"
                | "bucket_summary"
                | "bucket_events_since"
                | "command_output_tail"
                | "file_read_window"
                | "file_search"
                | "shell_session_list"
                | "shell_session_status"
                | "runtime_state"
                | "system_discover"
                | "health"
                | "policy_status"
                | "event_context"
        );
    }
    matches!(
        tool,
        "mcp__supervisor__intent_apply" | "mcp__supervisor__task_state"
    )
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
    // Legacy/import fixtures with an explicit root contract keep their original hook
    // lifecycle. Any task pointer or intent state opts into prompt-bound activation.
    let legacy_contract = root.join("contract.json").is_file()
        && !root.join("active.json").exists()
        && !root.join("intents.json").exists();
    if event == "UserPromptSubmit" && text(payload, "prompt").is_some() && !legacy_contract {
        let project = Path::new(text(payload, "cwd").unwrap_or("."));
        return match crate::autonomy::record_prompt(&root, project, payload) {
            Ok(intent) => Decision::Context {
                event: "UserPromptSubmit",
                text: format!(
                    "SUPERVISOR: current prompt requires an intent proposal before mutations. Read-only investigation is available. Call intent_apply with the event identity and a scoped contract or discussion/follow-up intent. {}",
                    intent
                ),
            },
            Err(_) => Decision::Block(
                "GATE: prompt intent could not be recorded; task activation remains unverified"
                    .to_owned(),
            ),
        };
    }
    let guarded_intent = (event == "PreToolUse" && !read_only_tool(payload))
        || matches!(event, "Stop" | "SubagentStop");
    let intent_snapshot = if guarded_intent {
        match crate::autonomy::intent_authority(
            &root,
            text(payload, "session_id").unwrap_or_default(),
        ) {
            Ok(snapshot) => Some(snapshot),
            Err(_) => {
                return if event == "PreToolUse" {
                    Decision::Deny("GATE: prompt intent state unavailable".to_owned())
                } else {
                    Decision::Block("GATE: prompt intent state unavailable".to_owned())
                };
            }
        }
    } else {
        None
    };
    if event == "PreToolUse" {
        if !read_only_tool(payload) {
            match crate::autonomy::pending(&root, text(payload, "session_id").unwrap_or_default()) {
                Ok(Some(_)) => return Decision::Deny("GATE: propose the current prompt intent and scoped contract before mutation or delegation".to_owned()),
                Err(_) => return Decision::Deny("GATE: prompt intent state unavailable".to_owned()),
                Ok(None) => {}
            }
            match crate::autonomy::discussion_turn(&root, text(payload, "session_id").unwrap_or_default()) {
                Ok(true) => return Decision::Deny("GATE: this prompt is discussion only; a new user work prompt is required before mutation or delegation".to_owned()),
                Err(_) => return Decision::Deny("GATE: prompt intent state unavailable".to_owned()),
                Ok(false) => {}
            }
            if root.join("intents.json").exists() {
                match lifecycle::active_task(&root) {
                    Ok(None) => return Decision::Deny("GATE: source mutation requires an active scoped task; discussion does not authorize edits".to_owned()),
                    Err(_) => return Decision::Deny("GATE: task lifecycle state unavailable".to_owned()),
                    Ok(Some(active)) => match crate::autonomy::owns_session(&active.dir, text(payload, "session_id").unwrap_or_default()) {
                        Ok(true) => {},
                        Ok(false) => return Decision::Deny("GATE: this task belongs to another session; explicit user resume is required before mutation".to_owned()),
                        Err(_) => return Decision::Deny("GATE: task session ownership unavailable".to_owned()),
                    }
                }
            }
        }
        if let Some(denial) = guard::state_write_denial(payload, &root) {
            return denial;
        }
        let project = Path::new(text(payload, "cwd").unwrap_or("."));
        match crate::policy::action_policy_denial(payload, project, &root) {
            Ok(Some(denial)) => return denial,
            Err(_) => return Decision::Deny("GATE: invalid configured action policy".to_owned()),
            Ok(None) => {}
        }
        // Inspection and proposal routes do not need to open or advance the task gate.
        if root.join("intents.json").exists() && read_only_tool(payload) {
            return Decision::None;
        }
    }
    if event == "Stop" {
        match crate::autonomy::pending(&root, text(payload, "session_id").unwrap_or_default()) {
            Ok(Some(_)) => return Decision::Block("GATE: classify the current prompt with intent_apply before stopping; discussion needs no task".to_owned()),
            Err(_) => return Decision::Block("GATE: prompt intent state unavailable".to_owned()),
            Ok(None) => {}
        }
    }
    if event == "Stop" {
        match crate::autonomy::discussion_turn(
            &root,
            text(payload, "session_id").unwrap_or_default(),
        ) {
            Ok(true) => return Decision::None,
            Err(_) => return Decision::Block("GATE: prompt intent state unavailable".to_owned()),
            Ok(false) => {}
        }
    }
    // Historical session bindings remain for provenance, but obsolete sessions must not
    // open a task session even for informational hooks: opening can advance repair state.
    {
        match lifecycle::active_task(&root) {
            Ok(Some(active)) => match crate::autonomy::owns_session(
                &active.dir,
                text(payload, "session_id").unwrap_or_default(),
            ) {
                Ok(false) => return Decision::None,
                Ok(true) => {}
                Err(_) => {
                    return match event {
                        "PreToolUse" => {
                            Decision::Deny("GATE: task session ownership unavailable".to_owned())
                        }
                        "Stop" | "SubagentStop" => {
                            Decision::Block("GATE: task session ownership unavailable".to_owned())
                        }
                        _ => Decision::None,
                    };
                }
            },
            Ok(None) => {}
            Err(_) => {
                return match event {
                    "PreToolUse" => {
                        Decision::Deny("GATE: task lifecycle state unavailable".to_owned())
                    }
                    "Stop" | "SubagentStop" => {
                        Decision::Block("GATE: task lifecycle state unavailable".to_owned())
                    }
                    _ => Decision::None,
                };
            }
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
            // The fast ownership check above can race an explicit resume. Recheck the
            // staged task while with_task holds its OS lock, before Session can mutate it.
            if !crate::autonomy::owns_session(dir, text(payload, "session_id").unwrap_or_default())?
            {
                return Ok(if event == "PreToolUse" && !read_only_tool(payload) {
                    Decision::Deny("GATE: task ownership changed; explicit user resume is required before mutation".to_owned()).to_json().unwrap_or(Value::Null)
                } else {
                    Value::Null
                });
            }
            if let Some((_, fingerprint)) = &intent_snapshot {
                use crate::autonomy::IntentAuthority;
                let (authority, current_fingerprint) = crate::autonomy::locked_intent_authority(
                    &root,
                    text(payload, "session_id").unwrap_or_default(),
                )?;
                let refusal = match authority {
                    IntentAuthority::Pending => {
                        Some(if event == "PreToolUse" {
                            Decision::Deny("GATE: propose the current prompt intent before mutation or delegation".to_owned())
                        } else {
                            Decision::Block(
                                "GATE: classify the current prompt before stopping".to_owned(),
                            )
                        })
                    }
                    IntentAuthority::Discussion => {
                        Some(if event == "PreToolUse" {
                            Decision::Deny("GATE: this prompt is discussion only; a new work prompt is required".to_owned())
                        } else {
                            Decision::None
                        })
                    }
                    IntentAuthority::Actionable if &current_fingerprint != fingerprint => {
                        Some(if event == "PreToolUse" {
                            Decision::Deny("GATE: current prompt intent changed; retry under its approved scope".to_owned())
                        } else {
                            Decision::Block(
                                "GATE: current prompt intent changed; retry before stopping"
                                    .to_owned(),
                            )
                        })
                    }
                    IntentAuthority::Actionable => None,
                };
                if let Some(refusal) = refusal {
                    return Ok(refusal.to_json().unwrap_or(Value::Null));
                }
                if event == "PreToolUse"
                    && root.join("intents.json").exists()
                    && lifecycle::active_task(&root)?.is_none()
                {
                    return Ok(Decision::Deny(
                        "GATE: source mutation requires an active scoped task".to_owned(),
                    )
                    .to_json()
                    .unwrap_or(Value::Null));
                }
            }
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
            match event {
                "Stop" | "SubagentStop" => {
                    Decision::Block(format!("GATE: evidence unavailable: {what}"))
                }
                _ => fail_open(&what),
            }
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
    if matches!(event, "Stop" | "SubagentStop") {
        session.sync_observed_receipts(&env.state_dir, agent, reasons)?;
    }
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
                    if task.state() == TaskState::Repairing {
                        task.observe_candidate(current.clone())
                            .map_err(|e| render(&e))?;
                        task.finish_repair().map_err(|e| render(&e))?;
                    }
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

    /// Receipts come from the protected observation journal, never from assistant prose.
    fn sync_observed_receipts(
        &mut self,
        dir: &Path,
        agent: &str,
        reasons: &mut Vec<String>,
    ) -> Result<(), String> {
        let Ok(current) = &self.observed else {
            return Ok(());
        };
        if self.task.state().is_terminal() && self.task.state() != TaskState::Completed {
            return Ok(());
        }
        let runs = observation::read_runs(dir, &self.task_id)?;
        for run in runs.iter().filter(|run| {
            (run.agent_id == agent || run.recovered_by.as_deref() == Some(agent))
                && run.candidate == current.id
                && run.cwd == run.execution_root
        }) {
            if !current_dispatch_reference(dir, &self.task_id, &run.reference)? {
                continue;
            }
            for criterion in self.criteria.iter().filter(|c| c.accepts(&run.command)) {
                if self
                    .gate
                    .receipts
                    .iter()
                    .any(|r| r.criterion == criterion.id && r.output_ref == run.reference)
                {
                    continue;
                }
                let receipt = VerificationReceipt {
                    criterion: criterion.id.clone(),
                    candidate: run.candidate.clone(),
                    command: run.command.clone(),
                    cwd: run.cwd.clone(),
                    exit_code: run.exit_code,
                    output_ref: run.reference.clone(),
                    claim_supported: true,
                    started_at: run.t,
                    finished_at: run.finished,
                };
                if self.task.state() != TaskState::Completed {
                    self.task
                        .record_receipt(receipt.clone())
                        .map_err(|e| render(&e))?;
                }
                reasons.push(format!(
                    "recorded CHECK {} | {} | exit {}",
                    criterion.id.0,
                    run.command,
                    run.exit_code
                        .map_or("unknown".to_owned(), |n| n.to_string())
                ));
                self.gate.receipts.push(receipt);
            }
        }
        Ok(())
    }

    /// The latest dispatch wins even when it has no valid completion. An older success
    /// must not hide a newer failed, interrupted, pending, or invalidated execution.
    fn latest_attempts(&self, dir: &Path, project: &Path) -> Result<(), String> {
        crate::autonomy::validate_entrypoints(dir, project)?;
        let Ok(current) = &self.observed else {
            return Err("candidate identity could not be observed".to_owned());
        };
        let rows = match std::fs::read_to_string(dir.join("runs.jsonl")) {
            Ok(text) => text
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid command observation journal".to_owned())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err("command observation journal unavailable".to_owned()),
        };
        let mut current_rows = Vec::new();
        for row in rows {
            if text(&row, "kind") == Some("dispatch") {
                let dispatch =
                    text(&row, "dispatch_id").ok_or("command dispatch has no identity")?;
                if !crate::autonomy::dispatch_is_current(dir, dispatch)? {
                    continue;
                }
            }
            current_rows.push(row);
        }
        // Current criteria are project-root checks. Nested observations retain their
        // actual cwd, but explicit nested criterion contexts need a separate contract API.
        let execution_root = location::canonical_project(project);
        for criterion in self.criteria.iter().filter(|c| c.mandatory) {
            let latest = current_rows.iter().rev().find(|row| {
                text(row, "kind") == Some("dispatch")
                    && text(row, "cwd") == Some(execution_root.as_str())
                    && text(row, "task_id") == Some(self.task_id.as_str())
                    && text(row, "command").is_some_and(|s| criterion.accepts(s))
                    && row.get("candidate").is_some_and(|v| {
                        text(v, "base_commit") == Some(current.id.base_commit.as_str())
                            && text(v, "dirty_digest") == Some(current.id.dirty_digest.as_str())
                    })
            });
            let Some(latest) = latest else { continue };
            if !observation::approved_context(latest) {
                return Err(format!(
                    "{} latest execution has unapproved or legacy execution context",
                    criterion.id.0
                ));
            }
            let dispatch = text(latest, "dispatch_id").ok_or("command dispatch has no identity")?;
            let observed = format!("observed:{}:{dispatch}", self.task_id);
            let reconstructed = format!("tc-reconstructed:{}:{dispatch}", self.task_id);
            let receipt = self.gate.receipts.iter().rev().find(|r| {
                r.criterion == criterion.id
                    && r.candidate == current.id
                    && (r.output_ref == observed || r.output_ref == reconstructed)
                    && receipt_is_observed(dir, &self.task_id, r)
            });
            match receipt.and_then(|r| r.exit_code) {
                Some(0) => {}
                Some(exit) => {
                    return Err(render(&GateError::FailingReceipt {
                        criterion: criterion.id.clone(),
                        exit_code: exit,
                    }));
                }
                None => {
                    return Err(format!(
                        "{} latest execution has no observed completion",
                        criterion.id.0
                    ));
                }
            }
        }
        Ok(())
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
        let observed = observation::read_runs(dir, &self.task_id)?
            .iter()
            .any(|run| {
                run.agent_id == agent && self.observed.as_ref().is_ok_and(|c| run.candidate == c.id)
            });
        let parsed = header::parse(message).or_else(|| {
            observed.then_some(Header {
                verdict: Verdict::Pass,
                checks_run: 0,
                checks: Vec::new(),
                consistent: true,
            })
        });
        let Some(header) = parsed else {
            if prior > 0 {
                reasons.push("no header; released after one refusal".to_owned());
                return Ok(Decision::None);
            }
            self.gate.handback_refusals.insert(agent.to_owned(), 1);
            return Ok(Decision::Block(
                    "GATE: handback refused: no observed verification. Run the required check commands, or report this handback as unverified.".to_owned(),
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
        if let Err(why) = self.latest_attempts(dir, Path::new(text(payload, "cwd").unwrap_or(".")))
        {
            problems.push(why);
        }
        let completed = self.task.state() == TaskState::Completed;
        let accepted = if completed || self.task.state() == TaskState::ReadyForReview {
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
        let mut used: HashSet<String> = HashSet::new();
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
                    && r.cwd == r.execution_root
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
            if self
                .gate
                .receipts
                .iter()
                .any(|r| r.criterion == receipt.criterion && r.output_ref == receipt.output_ref)
            {
                continue;
            }
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
        if crate::autonomy::is_paused(dir)? {
            reasons.push("task paused; no completion attempted".to_owned());
            return Ok(Decision::None);
        }
        let autonomous = crate::autonomy::is_autonomous_task(dir)?;
        if !autonomous && !claim::claims_completion(message) {
            reasons.push("no completion claim".to_owned());
            return Ok(Decision::None);
        }
        // A chair working alone cites its own runs the way a worker does.
        let (mut problems, instructions) = match header::parse(message) {
            Some(header) => {
                let cwd = text(payload, "cwd").unwrap_or_default();
                self.link_checks(&header, dir, cwd, agent, now, reasons)?
            }
            None => (Vec::new(), Vec::new()),
        };
        self.note_asserted(reasons);
        if let Err(why) = self.latest_attempts(dir, Path::new(text(payload, "cwd").unwrap_or(".")))
        {
            problems.push(why);
        }
        let result = if problems.is_empty() {
            self.observed_evidence()
                .map_err(|e| render(&e))
                .and_then(|()| {
                    match self.task.state() {
                        TaskState::Completed => self.observed_evidence(),
                        TaskState::ReadyForReview => self.task.complete(),
                        _ => self.task.try_accept().and_then(|()| self.task.complete()),
                    }
                    .map_err(|e| render(&e))
                })
        } else {
            // An unlinked CHECK line refuses before any acceptance is tried.
            Err(problems.join("; "))
        };
        match result {
            Ok(()) => {
                reasons.push("completed".to_owned());
                Ok(Decision::None)
            }
            Err(why) => {
                reasons.push(why.clone());
                if autonomous
                    && matches!(
                        self.task.state(),
                        TaskState::Verifying | TaskState::ReadyForReview
                    )
                    && let Err(error) = self.task.request_repair()
                {
                    reasons.push(render(&error));
                }
                let state = state_name(self.task.state());
                if active && self.gate.continuations >= 3 {
                    return Ok(Decision::System(format!(
                        "GATE: refusal cap reached; task remains {state}"
                    )));
                }
                self.gate.continuations += 1;
                let run_instead: String = instructions.iter().map(|i| format!(" {i}")).collect();
                let next = if !autonomous && header::parse(message).is_some() {
                    "record the check with CHECK lines"
                } else {
                    "run the required check commands"
                };
                Ok(Decision::Block(format!(
                    "GATE refuses completion (refusal {} of 3, harness cap 8): state {state}; {why}; {next} or report the task as not complete.{run_instead}",
                    self.gate.continuations
                )))
            }
        }
    }

    fn pre_tool_use(&mut self, payload: &Value) -> Decision {
        self.gate.continuations = 0;
        if !read_only_tool(payload) {
            match self
                .evidence_dir
                .as_ref()
                .map(|dir| crate::autonomy::is_paused(dir))
            {
                Some(Ok(true)) => {
                    return Decision::Deny(
                        "GATE: task is paused; resume through a new user prompt before mutation"
                            .to_owned(),
                    );
                }
                Some(Err(_)) => {
                    return Decision::Deny("GATE: task lifecycle state unavailable".to_owned());
                }
                _ => {}
            }
        }
        if self.task.state().is_terminal()
            && !read_only_tool(payload)
            && self
                .evidence_dir
                .as_ref()
                .is_some_and(|dir| crate::autonomy::is_autonomous_task(dir).unwrap_or(true))
        {
            return Decision::Deny(format!("GATE: task is {}", state_name(self.task.state())));
        }
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
        let cp = self.task.checkpoint();
        // Repair: restore() skips receipt replay for terminal states, so after a resume the
        // persisted gate receipts are the only evidence the summary can still report.
        let recorded: Vec<&str> = if cp.state.is_terminal() {
            cp.candidate
                .map(|candidate| {
                    self.criteria
                        .iter()
                        .filter(|c| {
                            self.gate.receipts.iter().any(|r| {
                                r.criterion == c.id
                                    && r.candidate == candidate
                                    // The acceptance path refuses mcp-asserted receipts
                                    // here and a failing run satisfies nothing; the
                                    // display must agree with it.
                                    && !r.output_ref.starts_with(MCP_ASSERTED)
                                    && r.exit_code == Some(0)
                                    && r.claim_supported
                            })
                        })
                        .map(|c| c.id.0.as_str())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for c in &self.criteria {
            if recorded.contains(&c.id.0.as_str()) {
                continue;
            }
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
            "GATE: state {}; current receipts: [{}]; missing: [{}]; stale: [{}]; recorded: [{}]; decision: {decision}",
            state_name(state),
            current.join(", "),
            missing.join(", "),
            stale.join(", "),
            recorded.join(", ")
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
