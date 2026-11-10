//! One hook payload in, at most one decision out. The core's rules are authoritative on every call.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use coding_supervisor::{
    Candidate, Criterion, CriterionId, GateError, LimitReached, ReceiptStatus, ReviewProvenance,
    ReviewRecord, Task, TaskContract, TaskState, VerificationReceipt,
};
use serde_json::Value;

use crate::candidate::CandidateProvider;
use crate::header::{self, Header, Verdict};
use crate::journal::{self, Trace};
use crate::level::{self, Level};
use crate::state::{self, Gate, Stored, StoredReview, obj, state_name};
use crate::{claim, guard, lifecycle, location, observation};

/// `output_ref` prefix of a receipt recorded through the MCP server: caller-asserted, never
/// observed by a hook, so the enforcing paths here do not count it.
pub const MCP_ASSERTED: &str = "mcp-asserted:";

/// True only when a hook completion journal backs every material receipt field. An unreadable
/// runs.jsonl or autonomy.json reads as "not observed".
pub fn receipt_is_observed(dir: &Path, task_id: &str, receipt: &VerificationReceipt) -> bool {
    receipt.claim_supported
        && current_dispatch_reference(dir, task_id, &receipt.output_ref).unwrap_or(false)
        && observation::read_runs(dir, task_id).is_ok_and(|runs| journaled(receipt, &runs))
}

/// [`receipt_is_observed`] for each of `receipts`, reading runs.jsonl and autonomy.json once;
/// either one unreadable is an error rather than "not observed".
pub fn receipts_observed(
    dir: &Path,
    task_id: &str,
    receipts: &[VerificationReceipt],
) -> Result<Vec<bool>, String> {
    let runs = observation::read_runs(dir, task_id)?;
    let invalidated = crate::autonomy::invalidated_dispatches(dir)?;
    Ok(receipts
        .iter()
        .map(|receipt| {
            receipt.claim_supported
                && cited_dispatch(task_id, &receipt.output_ref)
                    .is_some_and(|dispatch| !invalidated.iter().any(|id| id == dispatch))
                && journaled(receipt, &runs)
        })
        .collect())
}

/// A hook completion in `runs` matches every material field of `receipt`.
fn journaled(receipt: &VerificationReceipt, runs: &[observation::Run]) -> bool {
    runs.iter().any(|run| {
        run.cwd == run.execution_root
            && run.reference == receipt.output_ref
            && run.command == receipt.command
            && run.cwd == receipt.cwd
            && (run.candidate == receipt.candidate
                || run.transferred_to.contains(&receipt.candidate))
            && run.exit_code == receipt.exit_code
            && run.t == receipt.started_at
            && run.finished == receipt.finished_at
    })
}
/// Preserve dispatch IDs verbatim; they may themselves contain colons.
fn current_dispatch_reference(dir: &Path, task: &str, reference: &str) -> Result<bool, String> {
    match cited_dispatch(task, reference) {
        Some(dispatch) => crate::autonomy::dispatch_is_current(dir, dispatch),
        None => Ok(false),
    }
}

/// The dispatch an `observed:` or `tc-reconstructed:` reference of `task` cites.
pub(crate) fn cited_dispatch<'r>(task: &str, reference: &'r str) -> Option<&'r str> {
    [
        format!("observed:{task}:"),
        format!("tc-reconstructed:{task}:"),
    ]
    .iter()
    .find_map(|prefix| reference.strip_prefix(prefix.as_str()))
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
    pub level: level::Setting,
}

impl Env {
    pub fn real(cwd: &Path) -> Self {
        Self::build(cwd, cwd)
    }

    /// [`Env::real`] for a hook payload: a call from the linked worktree its session's task is
    /// bound to uses the main checkout's state, where that task lives.
    pub fn for_hook(cwd: &Path, session: &str) -> Self {
        Self::build(
            &crate::binding::redirect(cwd, session).unwrap_or_else(|| cwd.to_path_buf()),
            cwd,
        )
    }

    /// State for `state_cwd`'s project, the candidate observed for `cwd`'s task.
    fn build(state_cwd: &Path, cwd: &Path) -> Self {
        let lookup = |name: &str| std::env::var(name).ok();
        let config_dir = level::config_dir(&lookup);
        let read_only = location::resolve_state_dir_read_only(state_cwd, &lookup);
        let (level, source, error) = match read_only
            .as_ref()
            .map_err(String::clone)
            .and_then(|(root, _)| level::effective(root, config_dir.as_deref()))
        {
            Ok((level, source)) => (level, source, None),
            Err(why) => (Level::Enforce, "unreadable", Some(why)),
        };
        // At `off` nothing is written, so the state root is located without being created.
        let (state_dir, state_location) = match read_only {
            Ok(selected) if level == Level::Off => selected,
            _ => location::resolve_state_dir(state_cwd, &lookup),
        };
        let candidate = crate::binding::provider_for_root(&state_dir, cwd);
        Self {
            state_dir,
            state_location,
            candidate,
            clock: now,
            level: level::Setting {
                level,
                source,
                config_dir,
                error,
            },
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

/// The advisory answer to an adapter error: the call goes ahead. Used where nothing is governed
/// (no payload or project known, level `off` or `audit`, a person's `failure open`) and for
/// read-only calls and events that cannot refuse; see [`adapter_error`].
pub fn fail_open(what: &str) -> Decision {
    Decision::System(format!("GATE adapter error: {what}"))
}

/// An adapter error in a project at `enforce` or `strict` (an unreadable level runs there too)
/// fails closed: a non-read-only PreToolUse is denied and a stop refused. A `failure` file
/// reading `open` in the state `root` is the person's per-project way out
/// (`supervisor level <level> --failure open`).
fn adapter_error(payload: &Value, env: &Env, root: &Path, what: &str) -> Decision {
    if env.level.level < Level::Enforce || level::failure_open(root) {
        return fail_open(what);
    }
    closed(
        text(payload, "hook_event_name").unwrap_or_default(),
        payload.get("stop_hook_active").and_then(Value::as_bool) == Some(true),
        exempt_from_failing_closed(payload),
        what,
    )
    .unwrap_or_else(|| fail_open(what))
}

/// The fail-closed answer for `event`, or `None` where the call goes ahead with the advisory
/// message. A stop Claude Code already continued once (`stop_hook_active`) goes ahead with a
/// warning, so a broken gate cannot loop the session (as `hooks/supervisor-hook.js` does).
fn closed(event: &str, stop_hook_active: bool, exempt: bool, what: &str) -> Option<Decision> {
    let why = || blocking_words(&format!("GATE: the gate fails closed: {what}"));
    match event {
        "PreToolUse" if !exempt => Some(Decision::Deny(why())),
        "Stop" | "SubagentStop" if stop_hook_active => Some(Decision::System(blocking_words(
            &format!("GATE: WARNING, this {event} went ahead NOT certified: {what}"),
        ))),
        "Stop" | "SubagentStop" => Some(Decision::Block(why())),
        _ => None,
    }
}

/// The read-only calls an adapter error still lets through, so a broken gate stays
/// diagnosable. Deliberately narrower than [`inspection`]: `intent_apply`, proposal commands
/// and memory saves change state.
fn exempt_from_failing_closed(payload: &Value) -> bool {
    (read_only_tool(payload) && text(payload, "tool_name") != Some("mcp__supervisor__intent_apply"))
        || read_only_command(payload)
}

/// Defensive: a fail-closed text never says "no such file", "not found" or "missing" (any case).
/// Claude Code reads such text as non-blocking on the plugin wrapper's exit-2 stderr path
/// (`hooks/supervisor-hook.js`); this binary's JSON `decision: block` blocks either way.
fn blocking_words(text: &str) -> String {
    let mut text = text.to_owned();
    for (word, with) in [
        ("no such file", "no file"),
        ("not found", "absent"),
        ("missing", "absent"),
    ] {
        while let Some(at) = text.to_ascii_lowercase().find(word) {
            text.replace_range(at..at + word.len(), with);
        }
    }
    text
}

/// serde_json refuses a `\uD800`-`\uDFFF` escape that is not half of a high+low pair, so such a
/// payload would never be judged. Each one becomes the U+FFFD escape, of the same length. A
/// backslash always consumes the byte after it, so an escaped backslash before `u` is kept.
fn lone_surrogates_replaced(input: &str) -> std::borrow::Cow<'_, str> {
    let bytes = input.as_bytes();
    let escape = |at: usize| -> Option<u16> {
        let six = bytes.get(at..at + 6)?;
        if &six[..2] != b"\\u" || !six[2..].iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u16::from_str_radix(std::str::from_utf8(&six[2..]).ok()?, 16).ok()
    };
    let mut out: Option<Vec<u8>> = None;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        match escape(at) {
            Some(0xD800..=0xDBFF) if matches!(escape(at + 6), Some(0xDC00..=0xDFFF)) => at += 12,
            Some(0xD800..=0xDFFF) => {
                out.get_or_insert_with(|| bytes.to_vec())[at..at + 6].copy_from_slice(b"\\uFFFD");
                at += 6;
            }
            Some(_) => at += 6,
            None => at += 2,
        }
    }
    match out.map(String::from_utf8) {
        Some(Ok(text)) => std::borrow::Cow::Owned(text),
        _ => std::borrow::Cow::Borrowed(input),
    }
}

/// Payload text that is not JSON: no project, so no level, is known, and a call that can refuse
/// counts as governed. The event is read from the raw text.
fn unparsed(input: &str, what: &str) -> Decision {
    let raw = |key: &str| {
        let at = input.find(&format!("\"{key}\""))? + key.len() + 2;
        input[at..]
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
    };
    let event = ["PreToolUse", "Stop", "SubagentStop"]
        .into_iter()
        .find(|e| raw("hook_event_name").is_some_and(|v| v.starts_with(&format!("\"{e}\""))));
    let active = raw("stop_hook_active").is_some_and(|v| v.starts_with("true"));
    event
        .and_then(|event| closed(event, active, false, what))
        .unwrap_or_else(|| fail_open(what))
}

/// The binary's whole job: payload text in, decision JSON text out.
pub fn run_hook(input: &str) -> Option<String> {
    let input = lone_surrogates_replaced(input);
    let decision = match serde_json::from_str::<Value>(&input) {
        Err(e) => unparsed(&input, &format!("payload is not JSON ({e}); not journaled")),
        // Without a cwd no project, so no level, is known: the call goes ahead.
        Ok(payload) => match payload.get("cwd").and_then(Value::as_str) {
            Some(cwd) => handle(
                &payload,
                &mut Env::for_hook(
                    Path::new(cwd),
                    text(&payload, "session_id").unwrap_or_default(),
                ),
            ),
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
        "Read"
            | "Glob"
            | "Grep"
            | "LS"
            | "TaskGet"
            | "TaskList"
            | "ToolSearch"
            | "StructuredOutput"
            // G40: asking the user changes nothing.
            | "AskUserQuestion"
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

/// The words of a Bash command with no shell syntax and no `--output` flag.
fn simple_words(payload: &Value) -> Option<Vec<&str>> {
    if text(payload, "tool_name") != Some("Bash") {
        return None;
    }
    let command = payload["tool_input"]["command"]
        .as_str()
        .unwrap_or_default();
    if command.contains([';', '&', '|', '<', '>', '$', '`', '(', ')', '\n', '\r']) {
        return None;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.iter().any(|w| w.starts_with("--output")) {
        return None;
    }
    Some(words)
}

/// The supervisor's own read-only status commands.
fn supervisor_read_only(words: &[&str]) -> bool {
    matches!(
        words,
        ["supervisor", "status", ..]
            | ["supervisor", "level"]
            | ["supervisor", "level", "--project", _]
            | ["supervisor", "level", "status", ..]
            | ["supervisor", "--help"]
    )
}

/// WP4: what still runs at a reached task limit. Narrower than [`inspection`]: a memory save,
/// the git reads and a `--version` query can run a program the agent prepared (git's diff
/// drivers and textconv, a program found on PATH).
fn limit_exempt(payload: &Value) -> bool {
    read_only_tool(payload)
        || proposal_command(payload)
        || simple_words(payload).is_some_and(|words| supervisor_read_only(&words))
}

/// One read-only shell command with no shell syntax, usable before the prompt is classified.
/// It still passes the gate-state guard and the action policy, and is observed like any run.
fn read_only_command(payload: &Value) -> bool {
    let Some(mut words) = simple_words(payload) else {
        return false;
    };
    if supervisor_read_only(&words) {
        return true;
    }
    // G40: `git -C <dir> <subcommand>` is the same subcommand run in another directory.
    if let ["git", "-C", _, ..] = words.as_slice() {
        words.drain(1..3);
    }
    // G38: a version query of a program found on PATH. Only a plain name: a path could name a
    // repo script, and shell expansion (`{node,x.js}`, globs, `~`, quotes) could build one.
    if let [program, "--version" | "-V"] = words.as_slice()
        && program
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+'))
        && !program.starts_with('-')
    {
        return true;
    }
    matches!(
        words.as_slice(),
        ["git", "status" | "log" | "diff" | "show" | "rev-parse", ..]
            | ["git", "branch", "--show-current"]
            | ["git", "worktree", "list", ..]
            | ["git", "remote"]
            | ["git", "remote", "-v"]
            // G40: read-only views of the forge and the npm registry.
            | ["gh", "pr" | "issue" | "release" | "run", "view" | "list" | "checks" | "diff" | "status", ..]
            | ["npm", "view" | "ls" | "list", ..]
            | ["ls", ..]
            | ["pwd"]
    ) || matches!(words.as_slice(), ["git", "branch", flags @ ..]
        if flags.iter().all(|f| matches!(*f, "-a" | "-r" | "-v" | "-vv" | "--all" | "--remotes" | "--list" | "--verbose")))
}

/// `supervisor intent apply ...` run alone from Bash: the MCP proposal for a session that has no
/// supervisor MCP server (F19). Shell syntax is allowed only inside single quotes, where Bash
/// expands nothing; double quotes and backslashes outside them are refused.
fn proposal_command(payload: &Value) -> bool {
    if text(payload, "tool_name") != Some("Bash") {
        return false;
    }
    let command = payload["tool_input"]["command"]
        .as_str()
        .unwrap_or_default()
        .trim();
    let Some(rest) = command.strip_prefix("supervisor intent apply ") else {
        return false;
    };
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '\'' => quoted = !quoted,
            _ if quoted => {}
            ';' | '&' | '|' | '<' | '>' | '$' | '`' | '(' | ')' | '"' | '\\' | '\n' | '\r' => {
                return false;
            }
            _ => {}
        }
    }
    !quoted
}

/// Ends the refusal for a Terminal Commander job whose exit is not recorded yet (G27).
const PENDING_JOB: &str = " after it ends to record its exit";

/// What the pre-intent gate lets through while the current prompt is unclassified.
fn inspection(payload: &Value) -> bool {
    read_only_tool(payload)
        || read_only_command(payload)
        || proposal_command(payload)
        || memory_save(payload)
}

/// G31: saving memory is not project work: EverOS, or a Write/Edit of a harness memory note
/// `<home>/.claude/projects/<project>/memory/<name>.md` (never settings or hooks). The path
/// must be absolute, and both it and the root are resolved through existing links first, so a
/// look-alike inside a project, a share or a linked `memory` directory does not count.
fn memory_save(payload: &Value) -> bool {
    let tool = text(payload, "tool_name").unwrap_or_default();
    if let Some(name) = tool.strip_prefix("mcp__everos__memory_") {
        return matches!(name, "add" | "flush" | "search" | "list");
    }
    if !matches!(tool, "Write" | "Edit") {
        return false;
    }
    let Some(path) = payload
        .pointer("/tool_input/file_path")
        .and_then(Value::as_str)
    else {
        return false;
    };
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return false;
    };
    // `normalize` resolves existing links on both sides, so a linked `memory` directory or
    // note leaves the root and fails the prefix.
    let root = location::normalize(&Path::new(&home).join(".claude").join("projects"));
    let full = location::normalize(path);
    let Some(rest) = full.strip_prefix(&format!("{root}/")) else {
        return false;
    };
    matches!(rest.split('/').collect::<Vec<_>>().as_slice(), [project, "memory", name]
        if !project.is_empty() && name.len() > 3 && name.to_lowercase().ends_with(".md"))
}

/// A pending prompt blocks a stop once; while the harness is already continuing, release it
/// visibly so the harness's own block cap is never reached.
fn unclassified_stop(payload: &Value, reason: &str) -> Decision {
    if payload
        .get("stop_hook_active")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Decision::System(
            "GATE: prompt left unclassified; this stop is released and nothing is certified"
                .to_owned(),
        )
    } else {
        Decision::Block(reason.to_owned())
    }
}

pub fn handle(payload: &Value, env: &mut Env) -> Decision {
    let mut decision = handle_at_level(payload, env);
    // G23: only a prompt hook says so, and only when the installed package moved on.
    if let Decision::Context {
        event: "UserPromptSubmit",
        text,
    } = &mut decision
        && let Some(warning) = crate::version::current_mismatch()
    {
        text.push(' ');
        text.push_str(&warning);
    }
    note_agent_effects(payload, env, &decision).unwrap_or(decision)
}

fn handle_at_level(payload: &Value, env: &mut Env) -> Decision {
    JOURNAL.set(Journal::StateUnproven);
    match env.level.level {
        Level::Off => protection(payload, env).unwrap_or(Decision::None),
        Level::Audit => match protection(payload, env) {
            Some(denied) => journal_refusal(payload, env, denied),
            None => {
                let decision = judge(payload, env);
                match journal_refusal(payload, env, decision) {
                    Decision::Block(text) | Decision::Deny(text) => Decision::System(text),
                    other => other,
                }
            }
        },
        Level::Enforce | Level::Strict => {
            let decision = judge(payload, env);
            journal_refusal(payload, env, decision)
        }
    }
}

/// G16/G21, from the decision the harness will actually see: a SubagentStop that does not
/// refuse the handback ends its worker, so its background delegation closes (on every route,
/// including the early ones in `judge`); a PreToolUse that lets a tool run, which is not
/// read-only, marks its agent as a mutator, whatever the level did to a denial. G24(b): a
/// mutation that cannot be journaled is refused (from `enforce` up), never treated as read-only.
fn note_agent_effects(payload: &Value, env: &Env, decision: &Decision) -> Option<Decision> {
    if matches!(decision, Decision::Block(_) | Decision::Deny(_))
        || env.level.level == Level::Off
        || JOURNAL.get() == Journal::StateUnproven
    {
        return None;
    }
    let now = (env.clock)();
    match text(payload, "hook_event_name") {
        Some("SubagentStop") => observation::close_delegation(&env.state_dir, payload, now),
        Some("PreToolUse") if !inspection(payload) => {
            let session = text(payload, "session_id").unwrap_or_default();
            let dir = match lifecycle::active_task(&env.state_dir) {
                Ok(Some(active)) => active.dir,
                _ => env.state_dir.clone(),
            };
            // Another session's task is not this agent's; an unreadable ownership is refused
            // like an unjournaled mutation (review of G24b), never read as "not ours".
            match crate::autonomy::owns_session(&dir, session) {
                Ok(true) => {}
                Ok(false) => return None,
                Err(e) => {
                    let why = format!("GATE: the gate could not journal this tool ({e}); retry");
                    return Some(match env.level.level {
                        Level::Audit => Decision::System(why),
                        _ => Decision::Deny(why),
                    });
                }
            }
            let task = state::read_contract(&dir)
                .ok()
                .and_then(|(contract, _)| contract)
                .and_then(|c| lifecycle::task_identity(&dir, &c.id.0).ok())
                .unwrap_or_default();
            let agent = text(payload, "agent_id").unwrap_or("main");
            if let Err(e) = observation::record_mutation(&dir, payload, &task, agent, now) {
                let why = format!("GATE: the gate could not journal this tool ({e}); retry");
                return Some(match env.level.level {
                    Level::Audit => Decision::System(why),
                    _ => Decision::Deny(why),
                });
            }
        }
        _ => {}
    }
    None
}

/// Where this hook call stands with its events.jsonl line.
#[derive(Clone, Copy, PartialEq)]
enum Journal {
    /// The state root is not yet proven to belong to this project: write nothing.
    StateUnproven,
    /// The state root was prepared for this project and has the current layout: a refusal
    /// may be journaled.
    StateReady,
    /// `handle_inner` already wrote this call's line.
    Written,
}

thread_local! {
    static JOURNAL: std::cell::Cell<Journal> = const { std::cell::Cell::new(Journal::StateUnproven) };
}

/// F11e: a refusal decided before the task gate (prompt gate, guard, policy) still gets its
/// events.jsonl line, written only under a free state lock: a busy state stays untouched, and a
/// lost line keeps the refusal, because it must not open the gate.
fn journal_refusal(payload: &Value, env: &Env, decision: Decision) -> Decision {
    if JOURNAL.get() != Journal::StateReady
        || !matches!(decision, Decision::Deny(_) | Decision::Block(_))
    {
        return decision;
    }
    let trace = Trace {
        level: Some(env.level.level),
        level_source: Some(env.level.source),
        ..Trace::default()
    };
    let line = journal::line(
        payload,
        env.state_location,
        (env.clock)(),
        decision.to_json(),
        &[],
        &trace,
    );
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(env.state_dir.join("state.lock"));
    if let Ok(lock) = lock
        && lock.try_lock().is_ok()
    {
        let _ = state::append_line(&env.state_dir.join("events.jsonl"), &line);
    }
    decision
}

/// The PreToolUse refusals that hold at every level: the gate's own state and the configured
/// action policy. Reads the state root, writes nothing.
fn protection(payload: &Value, env: &Env) -> Option<Decision> {
    if text(payload, "hook_event_name") != Some("PreToolUse") {
        return None;
    }
    let root = &env.state_dir;
    if let Some(denial) = guard::gate_write_denial(payload, root, env.level.config_dir.as_deref()) {
        return Some(denial);
    }
    let project = Path::new(text(payload, "cwd").unwrap_or("."));
    match crate::policy::action_policy_denial(payload, project, root) {
        Ok(denial) => denial,
        Err(_) => Some(Decision::Deny(
            "GATE: invalid configured action policy".to_owned(),
        )),
    }
}

/// Linked worktrees keep their own gate state, so a session whose prompt is registered in the
/// main checkout has no prompt in its worktree. That must not make the worktree ungated.
fn unregistered_linked_worktree(root: &Path, payload: &Value) -> Option<Decision> {
    let session = text(payload, "session_id").unwrap_or_default();
    if crate::autonomy::registered(root, session).unwrap_or(true) {
        return None;
    }
    let worktree = PathBuf::from(location::canonical_project(Path::new(
        text(payload, "cwd").unwrap_or("."),
    )));
    let main = location::linked_main_checkout(&worktree)?;
    let lookup = |name: &str| std::env::var(name).ok();
    let (main_root, _) = location::resolve_state_dir_read_only(&main, &lookup).ok()?;
    if !crate::autonomy::registered(&main_root, session).unwrap_or(true) {
        return None;
    }
    Some(Decision::Deny(format!(
        "GATE: this session's prompt is registered in the main checkout {}, not in linked worktree {}; its task does not observe edits here. Send the next prompt from this worktree so the gate registers it here, or work in the main checkout",
        location::canonical_project(&main),
        worktree.display()
    )))
}

/// WP4: the active task's limits are known not to be reached (or there is no active task).
/// Anything else, a reached limit or state that cannot be read, sends the call on to the task
/// gate, which refuses it or fails closed.
fn no_limit_reached(root: &Path, now: u64) -> bool {
    match lifecycle::active_task(root) {
        Ok(None) => true,
        Ok(Some(active)) => matches!(
            state::read_contract(&active.dir),
            Ok((Some(contract), _)) if matches!(
                crate::limits::usage(&active.dir, &contract, now),
                Ok(usage) if usage.as_ref().is_none_or(|u| u.reached.is_none())
            )
        ),
        Err(_) => false,
    }
}

/// The active task is bound to a different session than this prompt's. Unknown ownership is
/// not reported as foreign, so the ordinary advice stays.
fn foreign_task(root: &Path, payload: &Value) -> bool {
    let session = text(payload, "session_id").unwrap_or_default();
    lifecycle::active_task(root)
        .ok()
        .flatten()
        .is_some_and(|active| crate::autonomy::owns_session(&active.dir, session) == Ok(false))
}

fn judge(payload: &Value, env: &mut Env) -> Decision {
    let root = env.state_dir.clone();
    let event = text(payload, "hook_event_name").unwrap_or_default();
    if env.state_location != "test"
        && let Err(what) = location::prepare_state(
            &root,
            &crate::binding::state_project(&root, Path::new(text(payload, "cwd").unwrap_or("."))),
        )
    {
        return adapter_error(payload, env, &root, &format!("state unavailable: {what}"));
    }
    // Only a current-layout root takes refusal lines; legacy state stays untouched until migrated.
    if root.join("active.json").exists() || root.join("intents.json").exists() {
        JOURNAL.set(Journal::StateReady);
    }
    // Legacy/import fixtures with an explicit root contract keep their original hook
    // lifecycle. Any task pointer or intent state opts into prompt-bound activation.
    let legacy_contract = root.join("contract.json").is_file()
        && !root.join("active.json").exists()
        && !root.join("intents.json").exists();
    // G36 (AAP report): a background-task notice the harness delivers as a prompt is not a
    // request; the previous prompt's classification stands. A person typing this marker
    // first only keeps that classification, it never widens it.
    if event == "UserPromptSubmit"
        && !legacy_contract
        && text(payload, "prompt")
            .is_some_and(|p| p.trim_start().starts_with("<task-notification>"))
    {
        return Decision::None;
    }
    if event == "UserPromptSubmit" && text(payload, "prompt").is_some() && !legacy_contract {
        let project =
            crate::binding::state_project(&root, Path::new(text(payload, "cwd").unwrap_or(".")));
        return match crate::autonomy::record_prompt(&root, &project, payload) {
            Ok(intent) => {
                let mut call = serde_json::json!({"event_id":intent["event_id"],"nonce":intent["nonce"],"prompt_hash":intent["prompt_hash"],"task_id":intent["task_id"]});
                // G35 (AAP report): with no discoverable checks and no task, no proposal can
                // start one; say that once, briefly, with what blocks at this level.
                let checkless = intent["task_id"].is_null()
                    && intent["required_criteria"]
                        .as_array()
                        .is_some_and(|c| c.iter().all(|c| c["id"] == "AUTO_DIFF"));
                if checkless {
                    let level = env.level.level;
                    let blocks = if matches!(level, Level::Enforce | Level::Strict) {
                        "edits are refused until then"
                    } else {
                        "nothing is blocked meanwhile"
                    };
                    // F19: a question must still be classifiable, from the shell too.
                    call["action"] = "discussion".into();
                    return Decision::Context {
                        event: "UserPromptSubmit",
                        text: format!(
                            "SUPERVISOR: this repository has no checks the gate can discover, so no task can start until a person adds one outside the session with `supervisor task add-check --project <dir> --criterion <ID> --command <command> --reason <text>`; at level {}, {blocks}. Do not start a task here. To classify this prompt as a question: intent_apply {call}; without the supervisor MCP tools, run it alone from the Bash tool: supervisor intent apply --proposal '<the call as JSON>'.",
                            level.name()
                        ),
                    };
                }
                // G18: the session's own unfinished task already took this prompt as `continue`.
                if intent["default_action"] == "continue" {
                    call["action"] = "<action>".into();
                    return Decision::Context {
                        event: "UserPromptSubmit",
                        text: format!(
                            "SUPERVISOR: this prompt continues the active task {} by default (recorded as continue, source default); edits and checks proceed without an intent_apply call. Call intent_apply only for another action: start|revise|pause|resume|cancel, or discussion for a question that needs no work. Call shape: intent_apply {call}; without the supervisor MCP tools, run it alone from the Bash tool: supervisor intent apply --proposal '<the call as JSON>'. {intent}",
                            intent["task_id"].as_str().unwrap_or_default()
                        ),
                    };
                }
                let next = if intent["task_id"].is_null() {
                    call["action"] = "start".into();
                    format!(
                        "Next call for new work: intent_apply {call} plus \"contract\": a scoped contract, at least {}; for a question, the same fields with \"action\":\"discussion\".",
                        crate::autonomy::MINIMAL_CONTRACT
                    )
                } else if foreign_task(&root, payload) {
                    // `continue`, `start` and `cancel` are all refused for another session's
                    // task; only a direct resume prompt or a person's cancel ends the block.
                    call["action"] = "discussion".into();
                    format!(
                        "The active task {} belongs to another session; continue, start and cancel are refused from this one. If the user asked to resume that task, send \"action\":\"resume\". Otherwise call intent_apply {call} and tell the user that {}.",
                        intent["task_id"].as_str().unwrap_or_default(),
                        crate::autonomy::person_cancel(
                            &intent,
                            intent["task_id"].as_str().unwrap_or_default()
                        )
                    )
                } else {
                    call["action"] = "continue".into();
                    format!(
                        "Next call to keep working on the active task: intent_apply {call}; change the action only if the prompt asks for it."
                    )
                };
                Decision::Context {
                    event: "UserPromptSubmit",
                    text: format!(
                        "SUPERVISOR: current prompt requires an intent proposal before mutations. Read-only investigation is available. Call intent_apply with the event identity and a scoped contract or discussion/follow-up intent. Valid actions: discussion|start|continue|revise|pause|resume|cancel. {next} Without the supervisor MCP tools, run the same call from the Bash tool, alone: supervisor intent apply --proposal '<the call as JSON>' (a contract may also be named as \"contract_file\": a JSON file in the project). {intent}"
                    ),
                }
            }
            // Unlike other adapter errors at this event, an unrecorded prompt is refused.
            Err(_) => {
                let what =
                    "prompt intent could not be recorded; task activation remains unverified";
                if level::failure_open(&root) {
                    fail_open(what)
                } else {
                    Decision::Block(blocking_words(&format!(
                        "GATE: the gate fails closed: {what}"
                    )))
                }
            }
        };
    }
    if event == "PreToolUse"
        && !inspection(payload)
        && env.state_location != "test"
        && let Some(denial) = unregistered_linked_worktree(&root, payload)
    {
        return denial;
    }
    let guarded_intent =
        (event == "PreToolUse" && !inspection(payload)) || matches!(event, "Stop" | "SubagentStop");
    let intent_snapshot = if guarded_intent {
        match crate::autonomy::intent_authority(
            &root,
            text(payload, "session_id").unwrap_or_default(),
        ) {
            Ok(snapshot) => Some(snapshot),
            Err(_) => {
                return adapter_error(payload, env, &root, "prompt intent state unavailable");
            }
        }
    } else {
        None
    };
    if event == "PreToolUse" {
        if !inspection(payload) {
            match crate::autonomy::pending(&root, text(payload, "session_id").unwrap_or_default()) {
                Ok(Some(_)) => return Decision::Deny("GATE: propose the current prompt intent and scoped contract before mutation or delegation".to_owned()),
                Err(_) => return adapter_error(payload, env, &root, "prompt intent state unavailable"),
                Ok(None) => {}
            }
            match crate::autonomy::discussion_turn(&root, text(payload, "session_id").unwrap_or_default()) {
                Ok(true) => return Decision::Deny("GATE: this prompt is classified as discussion; to work on it, apply start, continue or revise with the same event before mutation or delegation".to_owned()),
                Err(_) => return adapter_error(payload, env, &root, "prompt intent state unavailable"),
                Ok(false) => {}
            }
            if root.join("intents.json").exists() {
                match lifecycle::active_task(&root) {
                    Ok(None) => return Decision::Deny("GATE: source mutation requires an active scoped task; discussion does not authorize edits".to_owned()),
                    Err(_) => return adapter_error(payload, env, &root, "task lifecycle state unavailable"),
                    Ok(Some(active)) => match crate::autonomy::owns_session(&active.dir, text(payload, "session_id").unwrap_or_default()) {
                        Ok(true) => {},
                        Ok(false) => return Decision::Deny("GATE: this task belongs to another session; explicit user resume is required before mutation".to_owned()),
                        Err(_) => return adapter_error(payload, env, &root, "task session ownership unavailable"),
                    }
                }
            }
        }
        if let Some(denial) =
            guard::gate_write_denial(payload, &root, env.level.config_dir.as_deref())
        {
            return denial;
        }
        let project = Path::new(text(payload, "cwd").unwrap_or("."));
        match crate::policy::action_policy_denial(payload, project, &root) {
            Ok(Some(denial)) => return denial,
            Err(_) => return Decision::Deny("GATE: invalid configured action policy".to_owned()),
            Ok(None) => {}
        }
        // Inspection and proposal routes do not need to open or advance the task gate.
        if root.join("intents.json").exists()
            && (read_only_tool(payload)
                || (memory_save(payload) && no_limit_reached(&root, (env.clock)()))
                || proposal_command(payload)
                || (read_only_command(payload)
                    && matches!(lifecycle::active_task(&root), Ok(None))))
        {
            return Decision::None;
        }
    }
    if event == "Stop" {
        match crate::autonomy::pending(&root, text(payload, "session_id").unwrap_or_default()) {
            Ok(Some(event)) => {
                // G17: nothing could have been changed under an unclassified prompt (every
                // mutation or delegation is refused until it is classified), so a stop with no
                // task to judge is a discussion turn; a control prompt (cancel, pause, resume)
                // is never silently dropped this way.
                let project = crate::binding::state_project(
                    &root,
                    Path::new(text(payload, "cwd").unwrap_or(".")),
                );
                let discussion = serde_json::json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":"discussion","task_id":event["task_id"]});
                if event["control"].is_null()
                    && event.get("control_hint").is_none()
                    && crate::autonomy::propose(&root, &project, &discussion).is_ok()
                {
                    return Decision::None;
                }
                return unclassified_stop(
                    payload,
                    "GATE: classify the current prompt with intent_apply before stopping; discussion needs no task",
                );
            }
            Err(_) => return adapter_error(payload, env, &root, "prompt intent state unavailable"),
            Ok(None) => {}
        }
    }
    if event == "Stop" {
        match crate::autonomy::discussion_turn(
            &root,
            text(payload, "session_id").unwrap_or_default(),
        ) {
            Ok(true) => return Decision::None,
            Err(_) => return adapter_error(payload, env, &root, "prompt intent state unavailable"),
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
                    return adapter_error(
                        payload,
                        env,
                        &root,
                        "task session ownership unavailable",
                    );
                }
            },
            Ok(None) => {}
            Err(_) => {
                return adapter_error(payload, env, &root, "task lifecycle state unavailable");
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
    // Observed once, before with_task takes state.lock: the candidate's git never runs under it.
    let observed = env.candidate.observe();
    let candidate = observed
        .as_ref()
        .map(|c| format!("{}:{}", c.id.base_commit, c.id.dirty_digest))
        .unwrap_or_else(|e| format!("unobserved:{e}"));
    let digest = crate::sha256::hex(format!("{payload}:{candidate}").as_bytes());
    let result = crate::lifecycle::observe_connection(&root, "claude-hook").and_then(|()| {
        crate::lifecycle::with_task(
            &root,
            text(payload, "task_id"),
            text(payload, "session_id"),
            event_id.as_deref().map(|id| (id, digest.as_str())),
            |dir| {
                // The fast ownership check above can race an explicit resume. Recheck the
                // staged task while with_task holds its OS lock, before Session can mutate it.
                if !crate::autonomy::owns_session(dir, text(payload, "session_id").unwrap_or_default())?
                {
                    return Ok(if event == "PreToolUse" && !inspection(payload) {
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
                            } else if event == "SubagentStop"
                                && !observation::made_mutation(
                                    dir,
                                    text(payload, "agent_id").unwrap_or("main"),
                                )?
                            {
                                // G21: a worker that mutated nothing hands back a read-only
                                // report; one that mutated under an earlier prompt keeps the
                                // verification rule (the Pending refusal below).
                                Decision::None
                            } else {
                                unclassified_stop(
                                    payload,
                                    "GATE: classify the current prompt before stopping",
                                )
                            })
                        }
                        IntentAuthority::Discussion => {
                            Some(if event == "PreToolUse" {
                                Decision::Deny("GATE: this prompt is classified as discussion; apply start, continue or revise with the same event to work on it".to_owned())
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
                let decision = handle_inner(payload, env, observed, &root);
                env.state_dir = root.clone();
                // An Err leaves the staged task uncommitted.
                Ok(decision?.to_json().unwrap_or(Value::Null))
            },
        )
    });
    match result {
        Ok(Value::Null) => Decision::None,
        Ok(v) => decision_from_json(&v)
            .unwrap_or_else(|| adapter_error(payload, env, &root, "invalid cached hook response")),
        // Not an adapter error but an operator requirement: every PreToolUse is refused until
        // a person migrates the legacy state, read-only calls and `failure open` included.
        Err(what) if lifecycle::legacy_migration_pending(&root) => match event {
            "PreToolUse" => Decision::Deny(format!("GATE: {what}")),
            "Stop" | "SubagentStop" => Decision::Block(format!("GATE: {what}")),
            _ => fail_open(&what),
        },
        Err(what) => adapter_error(payload, env, &root, &what),
    }
}

/// `None` when `v` holds no decision (a corrupt cached response).
fn decision_from_json(v: &Value) -> Option<Decision> {
    if let Some(message) = v.get("systemMessage").and_then(Value::as_str) {
        return Some(Decision::System(message.to_owned()));
    }
    if v.get("decision").and_then(Value::as_str) == Some("block") {
        return Some(Decision::Block(
            text(v, "reason").unwrap_or_default().to_owned(),
        ));
    }
    if let Some(output) = v.get("hookSpecificOutput") {
        if text(output, "permissionDecision") == Some("deny") {
            return Some(Decision::Deny(
                text(output, "permissionDecisionReason")
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        // The event name rides in the cached JSON; keep it (a PreToolUse advisory stays one).
        let event = match text(output, "hookEventName") {
            Some("PreToolUse") => "PreToolUse",
            Some("UserPromptSubmit") => "UserPromptSubmit",
            _ => "PostToolUse",
        };
        return Some(Decision::Context {
            event,
            text: text(output, "additionalContext")
                .unwrap_or_default()
                .to_owned(),
        });
    }
    None
}

/// `root` is the state root, which holds the project's `failure` setting; `env.state_dir` is the
/// staged task here.
fn handle_inner(
    payload: &Value,
    env: &mut Env,
    observed: Result<Candidate, String>,
    root: &Path,
) -> Result<Decision, String> {
    let event = text(payload, "hook_event_name").unwrap_or_default();
    let agent = text(payload, "agent_id").unwrap_or("main");
    let mut reasons: Vec<String> = env.level.error.iter().cloned().collect();
    let mut trace = Trace {
        level: Some(env.level.level),
        level_source: Some(env.level.source),
        ..Trace::default()
    };
    let decision = process(
        payload,
        env,
        observed,
        event,
        agent,
        &mut reasons,
        &mut trace,
    )
    .unwrap_or_else(|what| {
        reasons.push(format!("adapter error: {what}"));
        if env.level.level >= Level::Enforce && level::failure_open(root) {
            reasons.push("failure open (advisory)".to_owned());
        }
        let what = match event {
            "Stop" | "SubagentStop" => format!("evidence unavailable: {what}"),
            _ => what,
        };
        adapter_error(payload, env, root, &what)
    });
    // Last, so the first reason, which the journal also records as `reason`, is unchanged.
    if trace.contract_v1 {
        reasons.push(state::V1_MIGRATED_REASON.to_owned());
    }
    let line = journal::line(
        payload,
        env.state_location,
        (env.clock)(),
        decision.to_json(),
        &reasons,
        &trace,
    );
    JOURNAL.set(Journal::Written);
    match state::append_line(&env.state_dir.join("events.jsonl"), &line) {
        Ok(()) => Ok(decision),
        Err(what) if env.level.level < Level::Enforce => {
            Ok(fail_open(&format!("events journal not written: {what}")))
        }
        // A refusal stands whether or not its line was written.
        Err(_) if matches!(decision, Decision::Deny(_) | Decision::Block(_)) => Ok(decision),
        // Anything else is not committed: the caller answers through `adapter_error`.
        Err(what) => Err(format!("events journal not written: {what}")),
    }
}

fn process(
    payload: &Value,
    env: &mut Env,
    observed: Result<Candidate, String>,
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
    trace.contract_v1 = contract.as_ref().is_some_and(|c| c.migrated_from_v1);
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
        Stored::Valid(gate) => Some(*gate),
        Stored::Absent => None,
    };
    let Some(contract) = contract else {
        reasons.push("no contract: advisory only".to_owned());
        return Ok(no_contract(payload, event));
    };
    if event == "PreToolUse"
        && env.level.level == Level::Strict
        && let Some(denial) = out_of_scope(payload, &contract.scope, &env.state_dir)
    {
        reasons.push("level strict: write outside the contract scope".to_owned());
        return Ok(denial);
    }
    if event == "PreToolUse" {
        let tool = text(payload, "tool_name").unwrap_or_default();
        if let Some(effect) = forbidden_effect(payload, &default_forbidden_effects(payload)) {
            reasons.push("default forbidden effect".to_owned());
            return Ok(Decision::Deny(format!(
                "GATE: {tool} refused: {effect} is forbidden by default (level {}); a person can lower the level with `supervisor level audit`",
                env.level.level.name()
            )));
        }
        if env.level.level == Level::Strict
            && let Some(effect) = forbidden_effect(payload, &contract.forbidden_effects)
        {
            reasons.push("level strict: forbidden effect".to_owned());
            return Ok(Decision::Deny(format!(
                "GATE: {tool} refused: the task forbids {effect} (level strict)"
            )));
        }
    }
    trace.state_before = stored.as_ref().map(|g| g.checkpoint.state);
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
    // G16: background work is journaled when launched and closed at its SubagentStop; a chair
    // Stop while any is outstanding is not judged, consumes no refusal and accepts nothing.
    match event {
        "PostToolUse" if text(payload, "tool_name") == Some("Agent") => {
            observation::record_delegation(&env.state_dir, payload, &task_id, now)?;
        }
        "Stop" if agent == "main" => {
            let session = text(payload, "session_id").unwrap_or_default();
            let waiting = observation::outstanding_background(&env.state_dir, session, now)?;
            if !waiting.is_empty() {
                let waiting = waiting.join(", ");
                reasons.push(format!(
                    "not judged: background work outstanding: {waiting}"
                ));
                return Ok(Decision::System(format!(
                    "GATE: not judged: background work outstanding: {waiting}"
                )));
            }
        }
        _ => {}
    }
    // WP4: only the events a limit acts on read it; an unreadable journal or clock fails closed
    // through `adapter_error`, never as unlimited.
    let usage = if matches!(event, "PreToolUse" | "Stop" | "SubagentStop") {
        crate::limits::usage(&env.state_dir, &contract, now)?
    } else {
        None
    };
    let reached = usage.as_ref().and_then(|usage| usage.reached);
    let mut session = Session::open_with_observed(
        contract,
        stored,
        observed,
        false,
        reasons,
        Some(&env.state_dir),
        env.level.level == Level::Strict,
    )?;
    if let Some(cwd) = text(payload, "cwd") {
        let tree_at = crate::binding::bound_root(&env.state_dir).unwrap_or_else(|| cwd.to_owned());
        session.tree = CurrentTree::new(Path::new(&tree_at), &env.state_dir);
    }
    if matches!(event, "Stop" | "SubagentStop") {
        session.sync_observed_receipts(&env.state_dir, agent, now, reasons)?;
        session.transfer_reviews(reasons);
    }
    let refusals = (
        session.gate.continuations,
        session.gate.handback_refusals.clone(),
    );
    let refused_for_limit = match reached {
        Some(reached) if event == "PreToolUse" && !limit_exempt(payload) => {
            Some(session.limit_denial(reached, reasons))
        }
        _ => None,
    };
    let decision = match event {
        _ if let Some(denial) = refused_for_limit => denial,
        "SubagentStop" => session.subagent_stop(payload, &env.state_dir, agent, now, reasons)?,
        // WP4: at a reached limit the stop is released whatever the judgement, so it spends no
        // repair round on a refusal nobody sees.
        "Stop" => session.stop(
            payload,
            &env.state_dir,
            agent,
            now,
            reached.is_none(),
            reasons,
        )?,
        "PreToolUse" => session.pre_tool_use(payload)?,
        "PostToolUse" if text(payload, "tool_name") == Some("Agent") => {
            session.backfill_model(payload, reasons);
            session.agent_result(payload)
        }
        "UserPromptSubmit" => {
            session.gate.continuations = 0;
            session.gate.prompt_id = text(payload, "prompt_id").map(str::to_owned);
            Decision::None
        }
        _ => Decision::None,
    };
    // WP4: at a reached limit a stop the gate would refuse, or let through uncertified (the
    // refusal cap, a pending job), goes ahead NOT certified, and the refusal it never showed is
    // not counted; a completion, and a completed task's warning, are untouched.
    let uncertified = match &decision {
        Decision::Block(_) => true,
        // The refusal cap and a pending job let a stop through uncertified; a completed task's
        // warning that the tree moved on is not that.
        Decision::System(_) => session.task.state() != TaskState::Completed,
        _ => false,
    };
    let release = matches!(event, "Stop" | "SubagentStop") && uncertified;
    let decision = match (decision, reached, &usage) {
        (_, Some(reached), Some(usage)) if release => {
            (session.gate.continuations, session.gate.handback_refusals) = refusals.clone();
            session.limit_release(event, reached, usage, &env.state_dir, reasons)?
        }
        (decision, ..) => decision,
    };
    if env.level.level == Level::Audit && matches!(decision, Decision::Block(_) | Decision::Deny(_))
    {
        // Lite never blocks, so a stop refusal it only reports must not count toward the caps.
        if matches!(decision, Decision::Block(_)) {
            (session.gate.continuations, session.gate.handback_refusals) = refusals;
        }
        reasons.push("level audit: reported, not blocked".to_owned());
    }
    session.gate.checkpoint = session.task.checkpoint();
    session.gate.starting = session.task.starting_candidate().cloned().map(Box::new);
    trace.state_after = Some(session.task.state());
    state::save_gate(&env.state_dir, &session.gate, &contract_sha256)?;
    Ok(decision)
}

/// Ultra: a Write, Edit, MultiEdit or NotebookEdit outside `scope` is refused before it
/// happens, under the rule completion applies to changed paths (`scope_contains`).
fn out_of_scope(payload: &Value, scope: &[String], task_dir: &Path) -> Option<Decision> {
    let tool = text(payload, "tool_name")?;
    if !matches!(tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit") {
        return None;
    }
    let input = payload.get("tool_input")?;
    let path = text(input, "file_path").or_else(|| text(input, "notebook_path"))?;
    let cwd = Path::new(text(payload, "cwd").unwrap_or("."));
    let root = crate::binding::execution_root(task_dir, cwd);
    let inside = project_relative(&root, &cwd.join(path))
        .ok()
        .flatten()
        .is_some_and(|relative| {
            scope
                .iter()
                .any(|s| coding_supervisor::scope_contains(s, &relative))
        });
    (!inside).then(|| {
        Decision::Deny(format!(
            "GATE: {tool} refused: {path} is outside the task's contract scope (level strict)"
        ))
    })
}

/// Typed forbidden effects every task carries from `enforce` up, whatever its contract says;
/// the checkout's `origin/HEAD` branch is added when git recorded one.
/// ponytail: `main`/`master` plus `origin/HEAD`; a default branch known only to the server is
/// not seen.
const DEFAULT_FORBIDDEN_EFFECTS: &[&str] = &[
    "deny_git_push_target:main",
    "deny_git_push_target:master",
    "deny_git_force_push",
    "deny_argv_prefix:npm publish",
];

fn default_forbidden_effects(payload: &Value) -> Vec<String> {
    // The defaults only concern commands; an edit tool needs no lookup.
    let tool = text(payload, "tool_name").unwrap_or_default();
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    if crate::guard::command_text(tool, input).is_none() {
        return Vec::new();
    }
    let mut effects: Vec<String> = DEFAULT_FORBIDDEN_EFFECTS
        .iter()
        .map(|e| (*e).to_owned())
        .collect();
    let cwd = Path::new(text(payload, "cwd").unwrap_or("."));
    if let Some(branch) = location::origin_default_branch(cwd) {
        effects.push(format!("deny_git_push_target:{branch}"));
    }
    effects
}

/// One `git push`: whether it forces, whether it pushes every branch, and the branches it
/// updates or deletes (lowercased).
#[derive(Default)]
struct Push {
    force: bool,
    all: bool,
    targets: Vec<String>,
}

/// Words a command line may start with before the program it runs.
const COMMAND_WRAPPERS: &[&str] = &["command", "env", "sudo", "exec", "nohup", "time"];

/// The `git push` that `words` runs, if any: `git` must be the program, after optional
/// wrappers and `NAME=value` assignments, so prose such as `echo git push` is not a push. A
/// push without refspecs, or with `HEAD`, targets the checked-out branch, which `head` reads
/// for the `-C` directory when one was given.
/// ponytail: aliases, `git -c alias.x=push x`, variables and globbed refspecs are not resolved.
fn git_push(words: &[&str], head: &dyn Fn(Option<&str>) -> Option<String>) -> Option<Push> {
    let words: Vec<&str> = words.iter().map(|w| w.trim_matches(['"', '\''])).collect();
    let at = words
        .iter()
        .position(|w| !COMMAND_WRAPPERS.contains(w) && (w.starts_with('-') || !w.contains('=')))?;
    let program = words[at].rsplit(['/', '\\']).next()?;
    if !(program.eq_ignore_ascii_case("git") || program.eq_ignore_ascii_case("git.exe")) {
        return None;
    }
    let mut dir = None;
    let mut rest = words[at + 1..].iter();
    loop {
        match *rest.next()? {
            "-c" => {
                rest.next();
            }
            "-C" => dir = rest.next().copied(),
            "push" => break,
            word if word.starts_with('-') => {}
            _ => return None,
        }
    }
    let mut push = Push::default();
    let mut positional = Vec::new();
    let mut tags = false;
    while let Some(word) = rest.next() {
        match *word {
            "--all" | "--mirror" | "--branches" => push.all = true,
            "--tags" => tags = true,
            "-o" | "--push-option" | "--repo" | "--receive-pack" | "--exec" => {
                rest.next();
            }
            // --force, --force-with-lease[=...], --force-if-includes, and git's unambiguous
            // abbreviations of --force (from --forc); short flags combine, as in -fu.
            word if word.starts_with("--force") => push.force = true,
            word if word.len() >= 6 && "--force".starts_with(word) => push.force = true,
            word if !word.starts_with("--") && word.starts_with('-') && word.contains('f') => {
                push.force = true;
            }
            word if word.starts_with('-') => {}
            word => positional.push(word),
        }
    }
    let refspecs = positional.get(1..).unwrap_or_default();
    if refspecs.is_empty() && !push.all && !tags {
        push.targets.extend(head(dir));
    }
    for spec in refspecs {
        let spec = match spec.strip_prefix('+') {
            Some(spec) => {
                push.force = true;
                spec
            }
            None => spec,
        };
        let dst = spec.rsplit_once(':').map_or(spec, |(_, dst)| dst);
        let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
        if dst.eq_ignore_ascii_case("HEAD") {
            push.targets.extend(head(dir));
        } else if !dst.is_empty() {
            push.targets.push(dst.to_lowercase());
        }
    }
    Some(push)
}

/// M5: typed forbidden effects are refused before they happen; the effect found is returned.
/// `deny_path:<dir>` refuses an edit tool under that project path; `deny_git:<subcommand>` a git
/// command with that subcommand; `deny_argv_prefix:<words>` a command (or a part of one between
/// `;`, `&`, `|` or a newline) that starts with those words; `deny_git_push_target:<branch>` a
/// push that updates or deletes that branch, or pushes every branch; `deny_git_force_push` any
/// forced push. Other entries stay unenforced.
fn forbidden_effect(payload: &Value, effects: &[String]) -> Option<String> {
    let tool = text(payload, "tool_name")?;
    let input = payload.get("tool_input")?;
    let edit = matches!(tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit");
    // Resolving the project starts git, so only for an edit checked against a deny_path.
    let target = (edit
        && effects
            .iter()
            .any(|e| e.trim_start().starts_with("deny_path")))
    .then(|| text(input, "file_path").or_else(|| text(input, "notebook_path")))
    .flatten()
    .map(|path| {
        let cwd = Path::new(text(payload, "cwd").unwrap_or("."));
        project_relative(&location::canonical_project(cwd), &cwd.join(path))
    });
    let raw = crate::guard::command_text(tool, input);
    let command = raw.as_deref().map(str::to_lowercase);
    let split = |c: &'_ str| -> Vec<Vec<String>> {
        c.split([';', '&', '|', '\n'])
            .map(|part| part.split_whitespace().map(str::to_owned).collect())
            .collect()
    };
    let lowered = command.as_deref().map(split).unwrap_or_default();
    let segments: Vec<Vec<&str>> = lowered
        .iter()
        .map(|words| words.iter().map(String::as_str).collect())
        .collect();
    // Pushes keep their case: `-C <dir>` must not read as `-c`, and paths are case-sensitive.
    // ponytail: a bare push reads HEAD from the `-C` path or the command's cwd; a `cd x &&` in
    // an earlier segment is not followed, and an unreadable HEAD lets the bare push through.
    let base = PathBuf::from(
        text(input, "cwd")
            .filter(|_| tool.starts_with("mcp__terminal_commander__"))
            .or_else(|| text(payload, "cwd"))
            .unwrap_or("."),
    );
    let head = |dir: Option<&str>| location::current_branch(&base.join(dir.unwrap_or(".")));
    // Only the command itself: Terminal Commander's command text also carries cwd and path
    // fields, which would read as refspecs.
    let push_text = if tool.starts_with("mcp__terminal_commander__") {
        input
            .get("argv")
            .and_then(Value::as_array)
            .map(|argv| {
                argv.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .or_else(|| {
                ["shell_line", "line", "command"]
                    .iter()
                    .find_map(|field| text(input, field).map(str::to_owned))
            })
    } else {
        raw.clone()
    };
    let pushes: Vec<Push> = push_text
        .as_deref()
        .map(split)
        .unwrap_or_default()
        .iter()
        .filter_map(|words| {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            git_push(&words, &head)
        })
        .collect();
    let effect = effects.iter().find(|effect| {
        let (kind, value) = effect.split_once(':').unwrap_or((effect.as_str(), ""));
        let value = value.trim();
        match kind.trim() {
            // A target that cannot be placed is refused by every deny_path.
            "deny_path" => match &target {
                Some(Err(Unresolvable)) => true,
                Some(Ok(Some(path))) => coding_supervisor::scope_contains(value, path),
                _ => false,
            },
            "deny_git" => segments
                .iter()
                .any(|words| git_subcommand(words) == Some(value.to_lowercase().as_str())),
            "deny_argv_prefix" => {
                let prefix: Vec<String> = value.split_whitespace().map(str::to_lowercase).collect();
                !prefix.is_empty()
                    && segments.iter().any(|words| {
                        words.len() >= prefix.len()
                            && words.iter().zip(&prefix).all(|(w, p)| *w == p.as_str())
                    })
            }
            "deny_git_push_target" => {
                let branch = value.to_lowercase();
                pushes
                    .iter()
                    .any(|push| push.all || push.targets.contains(&branch))
            }
            "deny_git_force_push" => pushes.iter().any(|push| push.force),
            _ => false,
        }
    })?;
    Some(effect.clone())
}

/// The subcommand of a `git` invocation in `words`, skipping global options and the values of
/// `-C` and `-c`.
fn git_subcommand<'a>(words: &[&'a str]) -> Option<&'a str> {
    let at = words
        .iter()
        .position(|w| w.rsplit(['/', '\\']).next() == Some("git"))?;
    let mut rest = words[at + 1..].iter();
    while let Some(word) = rest.next() {
        if matches!(*word, "-c" | "-C") {
            rest.next();
        } else if !word.starts_with('-') {
            return Some(word);
        }
    }
    None
}

/// A path that exists but cannot be resolved: a symlink or junction whose target is missing.
/// Its name proves nothing, since a write through it lands wherever the target is.
struct Unresolvable;

/// `path` relative to the canonical `project` (see `location::canonical_project`), in git's
/// `/`-separated form; `Ok(None)` outside the project. The deepest existing ancestor is
/// resolved, so a new file under a symlinked or short-name spelling of the project still
/// matches.
fn project_relative(project: &str, path: &Path) -> Result<Option<String>, Unresolvable> {
    let mut existing = path;
    let mut missing = Vec::new();
    let resolved = loop {
        if let Ok(dir) = std::fs::canonicalize(existing) {
            break missing.iter().rev().fold(dir, |dir, name| dir.join(name));
        }
        if std::fs::symlink_metadata(existing).is_ok() {
            return Err(Unresolvable);
        }
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Ok(None);
        };
        missing.push(name);
        existing = parent;
    };
    let text = resolved.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let Some(head) = text.get(..project.len()) else {
        return Ok(None);
    };
    // `canonical_project` is already lower-case on Windows.
    let same = if cfg!(windows) {
        head.to_lowercase() == project
    } else {
        head == project
    };
    Ok(text[project.len()..]
        .strip_prefix('/')
        .filter(|_| same)
        .map(str::to_owned))
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
        GateError::NotReproduced(c) => format!(
            "{} did not fail on the task's starting candidate: run it before changing the code, and it must reproduce the reported failure",
            c.0
        ),
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
        GateError::ReviewRequired => {
            "review required: no approving review is recorded on this candidate".to_owned()
        }
        GateError::ReviewNotIndependent => "independent review required: every approval on this candidate came from the chair or from an agent that edited files or ran commands in this task".to_owned(),
        GateError::ReviewRejected { reviewer, findings } => format!(
            "review rejected by {reviewer} on this candidate ({findings} finding{})",
            if *findings == 1 { "" } else { "s" }
        ),
    }
}

/// WP5: what a Stop refused for its review asks for, with a person's way out.
fn review_protocol(rejected: bool, task_id: &str) -> String {
    let dispatch = "dispatch a reviewer agent that only reads (Read, Grep, Glob, git diff, log or show; a reviewer that edits or runs any other command counts as a builder) and whose final reply starts with the line `REVIEW: APPROVED` or `REVIEW: REJECTED`, with one `FINDING: <text>` line per finding below it";
    // `task cancel` ends a person's task and an agent's alike; `limits::way_out` names a
    // disposition that is only true at a task limit.
    let way_out = format!(
        "end the task with `supervisor task cancel --project <dir> --task {task_id} --reason <text>`"
    );
    if rejected {
        format!(
            "address the review findings and change the code, because a rejection stands until the content changes; then {dispatch}; a person can instead {way_out}"
        )
    } else {
        format!("{dispatch}; a person can instead {way_out}")
    }
}

/// G30: the criteria with a passing observed run on the current worktree content (the same
/// candidate, or another one with the same tree), as a Stop would count them. Read-only:
/// nothing is recorded, so the advisory `task_state` can show what the hook adapter accepts.
pub fn observed_criteria(dir: &Path, project: &Path) -> Result<Vec<String>, String> {
    let Some(contract) = state::read_contract(dir)?.0 else {
        return Ok(Vec::new());
    };
    let task = lifecycle::task_identity(dir, &contract.id.0)?;
    let root = crate::binding::bound_root(dir).map_or_else(|| project.to_owned(), PathBuf::from);
    let Ok(current) = (crate::candidate::GitCandidate { cwd: root.clone() }).observe() else {
        return Ok(Vec::new());
    };
    let tree = std::cell::OnceCell::new();
    // The latest completed run on this content decides, as at Stop: a later failure hides an
    // earlier pass. (A still-pending dispatch is reported separately as a pending job.)
    let mut latest: Vec<(String, bool)> = Vec::new();
    let mut runs = observation::read_runs(dir, &task)?;
    runs.sort_by_key(|run| run.finished);
    for run in runs {
        if run.cwd != run.execution_root {
            continue;
        }
        let same = run.candidate == current.id
            || run.transferred_to.contains(&current.id)
            || run.tree.as_deref().is_some_and(|t| {
                tree.get_or_init(|| crate::git::worktree_tree(&root, dir))
                    .as_deref()
                    == Some(t)
            });
        if !same || !current_dispatch_reference(dir, &task, &run.reference)? {
            continue;
        }
        for criterion in contract.criteria.iter().filter(|c| c.accepts(&run.command)) {
            latest.retain(|(id, _)| id != &criterion.id.0);
            latest.push((criterion.id.0.clone(), run.exit_code == Some(0)));
        }
    }
    Ok(latest
        .into_iter()
        .filter_map(|(id, passed)| passed.then_some(id))
        .collect())
}

/// The core task and gate.json for one call, rebuilt from contract.json, gate.json and the observed candidate.
pub struct Session {
    pub task: Task,
    pub gate: Gate,
    criteria: Vec<Criterion>,
    observed: Result<Candidate, String>,
    evidence_dir: Option<PathBuf>,
    task_id: String,
    tree: CurrentTree,
}

/// G9: how a run or dispatch counts for the current candidate.
#[derive(PartialEq)]
enum Match {
    Exact,
    /// Another candidate, same tree: the content is identical, only HEAD moved.
    Tree,
}

/// The current worktree tree, computed at most once per hook call and only when an exact
/// candidate match has failed.
#[derive(Default)]
struct CurrentTree {
    at: Option<(PathBuf, PathBuf)>,
    cell: std::cell::OnceCell<Option<String>>,
}

impl CurrentTree {
    fn new(project: &Path, state_dir: &Path) -> Self {
        Self {
            at: Some((project.to_owned(), state_dir.to_owned())),
            cell: Default::default(),
        }
    }

    fn get(&self) -> Option<&str> {
        self.cell
            .get_or_init(|| {
                let (project, state_dir) = self.at.as_ref()?;
                crate::git::worktree_tree(project, state_dir)
            })
            .as_deref()
    }
}

impl Session {
    /// Whether evidence made at `candidate` (in `tree`, when known; already `transferred` to
    /// the current candidate, when so recorded) counts for the current candidate. Every Stop
    /// comparison of past evidence with the current candidate goes through here; a worker's
    /// SubagentStop handback keeps the exact rule (fails closed after a commit).
    fn matches_current(
        &self,
        candidate: &coding_supervisor::CandidateId,
        tree: Option<&str>,
        transferred: bool,
    ) -> Option<Match> {
        let current = self.observed.as_ref().ok()?;
        if *candidate == current.id {
            Some(Match::Exact)
        } else if transferred || tree.is_some_and(|tree| self.tree.get() == Some(tree)) {
            Some(Match::Tree)
        } else {
            None
        }
    }

    /// `honour_asserted`: replay receipts whose `output_ref` starts with [`MCP_ASSERTED`].
    /// The hook adapter passes `false`; only the advisory MCP server passes `true`.
    pub fn open(
        contract: TaskContract,
        stored: Option<Gate>,
        observed: Result<Candidate, String>,
        honour_asserted: bool,
        reasons: &mut Vec<String>,
    ) -> Result<Self, String> {
        Self::open_with_observed(
            contract,
            stored,
            observed,
            honour_asserted,
            reasons,
            None,
            false,
        )
    }

    fn open_with_observed(
        contract: TaskContract,
        stored: Option<Gate>,
        observed: Result<Candidate, String>,
        honour_asserted: bool,
        reasons: &mut Vec<String>,
        evidence_dir: Option<&Path>,
        reproduction: bool,
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
                if reproduction {
                    task.require_reproduction();
                }
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
                // WP5: before the replay, which may try acceptance again.
                task.set_reviews(gate.reviews.iter().map(|s| s.review.clone()).collect());
                if reproduction {
                    task.require_reproduction();
                }
                if let Some(starting) = &gate.starting {
                    task.set_starting_candidate((**starting).clone());
                }
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
            tree: CurrentTree::default(),
        })
    }

    /// Receipts come from the protected observation journal, never from assistant prose.
    fn sync_observed_receipts(
        &mut self,
        dir: &Path,
        agent: &str,
        now: u64,
        reasons: &mut Vec<String>,
    ) -> Result<(), String> {
        let Ok(current) = &self.observed else {
            return Ok(());
        };
        if self.task.state().is_terminal() && self.task.state() != TaskState::Completed {
            return Ok(());
        }
        let runs = observation::read_runs(dir, &self.task_id)?;
        let starting = self.task.starting_candidate().cloned();
        for run in runs.iter().filter(|run| {
            (run.agent_id == agent || run.recovered_by.as_deref() == Some(agent))
                && run.cwd == run.execution_root
        }) {
            let matched = self.matches_current(
                &run.candidate,
                run.tree.as_deref(),
                run.transferred_to.contains(&current.id),
            );
            if matched.is_none() && Some(&run.candidate) != starting.as_ref() {
                continue;
            }
            if !current_dispatch_reference(dir, &self.task_id, &run.reference)? {
                continue;
            }
            // The receipt of a tree match is made on the current candidate; the transfer row
            // lets the journal check link it to the run made on another one.
            let candidate = if matched == Some(Match::Tree) {
                if let Some(tree) = self.tree.get() {
                    observation::record_transfer(dir, &self.task_id, run, &current.id, tree, now)?;
                }
                current.id.clone()
            } else {
                run.candidate.clone()
            };
            // K6: a run on the starting candidate evidences only a baseline criterion.
            for criterion in self
                .criteria
                .iter()
                .filter(|c| c.accepts(&run.command) && (matched.is_some() || c.is_baseline()))
            {
                if self.gate.receipts.iter().any(|r| {
                    r.criterion == criterion.id
                        && r.output_ref == run.reference
                        && r.candidate == candidate
                }) {
                    continue;
                }
                let receipt = VerificationReceipt {
                    criterion: criterion.id.clone(),
                    candidate: candidate.clone(),
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
        let execution_root = crate::binding::execution_root(dir, project);
        for criterion in self.criteria.iter().filter(|c| c.mandatory) {
            let latest = current_rows.iter().rev().find(|row| {
                text(row, "kind") == Some("dispatch")
                    && text(row, "cwd") == Some(execution_root.as_str())
                    && text(row, "task_id") == Some(self.task_id.as_str())
                    && text(row, "command").is_some_and(|s| criterion.accepts(s))
                    && row.get("candidate").is_some_and(|v| {
                        let id = coding_supervisor::CandidateId {
                            base_commit: text(v, "base_commit").unwrap_or_default().to_owned(),
                            dirty_digest: text(v, "dirty_digest").unwrap_or_default().to_owned(),
                        };
                        let dispatch = text(row, "dispatch_id").unwrap_or_default();
                        self.matches_current(
                            &id,
                            text(row, "tree"),
                            observation::transferred(
                                &current_rows,
                                &self.task_id,
                                dispatch,
                                &current.id,
                            ),
                        )
                        .is_some()
                    })
            });
            let Some(latest) = latest else { continue };
            if !observation::approved_context(latest) {
                return Err(format!(
                    "{} latest execution has unapproved or legacy execution context: rerun it without extra environment or shell; a CARGO_TARGET_DIR passed to the command counts only when the hook's own environment has the same value (set it in the harness settings env or cargo's [build] target-dir instead)",
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
                    // A Terminal Commander job is recorded only from its final command_status;
                    // an exit seen through bucket_wait carries no structured exit state.
                    let job = current_rows.iter().find_map(|row| {
                        (text(row, "kind") == Some("job")
                            && text(row, "dispatch_id") == Some(dispatch))
                        .then(|| text(row, "job_id"))
                        .flatten()
                    });
                    return Err(match job {
                        Some(job) => format!(
                            "{} latest execution has no observed completion: call command_status for Terminal Commander job {job}{PENDING_JOB}",
                            criterion.id.0
                        ),
                        None => format!(
                            "{} latest execution has no observed completion",
                            criterion.id.0
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// WP4: a PreToolUse at a reached limit that is not [`limit_exempt`] is refused whatever the
    /// task state, and changes no task state: both limits are recomputed at every call, so a
    /// task whose evidence is complete still completes at its next Stop.
    fn limit_denial(&self, reached: LimitReached, reasons: &mut Vec<String>) -> Decision {
        let what = crate::limits::reached_text(&reached);
        reasons.push(format!("task limit reached: {what}"));
        Decision::Deny(format!(
            "GATE: task limit reached: {what}; new work is refused; still allowed: read-only tools (Read, Grep, Glob, LS and read-only lookups), `supervisor status`, `supervisor level` and intent proposals; stop and report"
        ))
    }

    /// WP4: a Stop or SubagentStop the gate refused or let through uncertified, at a reached
    /// limit, goes ahead. A SubagentStop is released silently (any output there keeps the worker
    /// running) and changes no task state, so the chair's own Stop still decides. A Stop says so
    /// in a report and moves a task that is not terminal to `budget_exhausted`; a completed task
    /// stays completed.
    fn limit_release(
        &mut self,
        event: &str,
        reached: LimitReached,
        usage: &crate::limits::Usage,
        dir: &Path,
        reasons: &mut Vec<String>,
    ) -> Result<Decision, String> {
        let completed = self.task.state() == TaskState::Completed;
        let what = crate::limits::reached_text(&reached);
        let released = if completed {
            "the task stays completed on the candidate it completed on"
        } else {
            "NOT certified"
        };
        reasons.push(format!("task limit reached: {what}; released, {released}"));
        if event == "SubagentStop" {
            return Ok(Decision::None);
        }
        let _ = self.task.exhaust_limit(reached);
        let remaining = self.remaining_criteria();
        let stop = if completed {
            "this stop goes ahead; the task stays completed on the candidate it completed on, and the current work is NOT certified"
        } else {
            "this stop goes ahead NOT certified"
        };
        Ok(Decision::System(blocking_words(&format!(
            "GATE: task limit reached ({what}); {stop}; elapsed {}; {}; remaining criteria: {}; the work tree and logs are kept; for new work, {}",
            usage
                .elapsed_s
                .map_or("unknown (no approval time)".to_owned(), |s| format!(
                    "{s} s"
                )),
            usage
                .turns_used
                .map_or("turns not counted (no turn limit)".to_owned(), |n| format!(
                    "turns used {n}"
                )),
            if remaining.is_empty() {
                "none".to_owned()
            } else {
                remaining.join(", ")
            },
            crate::limits::way_out(dir, &self.task_id)?
        ))))
    }

    /// Mandatory criteria with no passing, hook-linked receipt on the task's candidate. When
    /// reproduction is required, a baseline is met by its failure on the starting candidate, as
    /// in [`Self::observed_failure`].
    fn remaining_criteria(&self) -> Vec<&str> {
        let reproduction = self.task.requires_reproduction();
        self.criteria
            .iter()
            .filter(|c| {
                c.mandatory
                    && !if reproduction && c.is_baseline() {
                        self.reproduced(c)
                    } else {
                        self.passing_on_checkpoint(c)
                    }
            })
            .map(|c| c.id.0.as_str())
            .collect()
    }

    /// Baseline `c` reproduced: its latest hook-observed receipt failed on the starting
    /// candidate, the rule [`Self::observed_evidence`] applies in strict mode.
    fn reproduced(&self, c: &Criterion) -> bool {
        let Some(dir) = self.evidence_dir.as_ref() else {
            return false;
        };
        self.gate
            .receipts
            .iter()
            .rev()
            .find(|r| r.criterion == c.id && receipt_is_observed(dir, &self.task_id, r))
            .is_some_and(|r| {
                Some(&r.candidate) == self.task.starting_candidate()
                    && r.exit_code.is_some_and(|code| code != 0)
            })
    }

    /// The rule of [`Self::observed_evidence`]: the latest hook-observed receipt for `c` on the
    /// task's candidate exited 0, so a check that passed and then failed is not met.
    fn passing_on_checkpoint(&self, c: &Criterion) -> bool {
        let (Some(candidate), Some(dir)) = (self.task.candidate(), self.evidence_dir.as_ref())
        else {
            return false;
        };
        self.gate
            .receipts
            .iter()
            .rev()
            .find(|r| {
                r.criterion == c.id
                    && &r.candidate == candidate
                    && receipt_is_observed(dir, &self.task_id, r)
            })
            .is_some_and(|r| r.exit_code == Some(0))
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
        // WP5: a review is recorded before the read-only return, so a pure reviewer is heard.
        let message = self.take_review(message, dir, agent, now, reasons)?;
        // G21: an agent that changed nothing hands back a read-only report. Nothing is
        // consumed or accepted; the line is journaled, not printed, because any output on
        // SubagentStop keeps the worker running.
        if !observation::made_mutation(dir, agent)? {
            reasons.push("read-only handback: not judged".to_owned());
            return Ok(Decision::None);
        }
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
        repair: bool,
        reasons: &mut Vec<String>,
    ) -> Result<Decision, String> {
        let message = text(payload, "last_assistant_message").unwrap_or_default();
        if crate::autonomy::is_paused(dir)? {
            reasons.push("task paused; no completion attempted".to_owned());
            return Ok(Decision::None);
        }
        // WP5: the chair's own review is caller-asserted; "REVIEW: APPROVED" is no claim.
        let message = self.take_review(message, dir, agent, now, reasons)?;
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
        let project = Path::new(text(payload, "cwd").unwrap_or("."));
        // A repair round is spent only on a candidate judged wrong, never on absent evidence.
        // A redefined check entrypoint is a defect in the candidate.
        let mut judged_wrong = crate::autonomy::validate_entrypoints(dir, project).is_err();
        if let Err(why) = self.latest_attempts(dir, project) {
            problems.push(why);
        }
        // M6 rides on strict, which is when reproduction is required.
        if self.task.requires_reproduction() {
            problems.extend(self.too_few_tests(dir)?);
        }
        // G27: the only gap is a job still running; refuse it once per turn, then wait.
        let only_pending_job = problems.len() == 1 && problems[0].ends_with(PENDING_JOB);
        // WP5: `Some(rejected)` when the review rule refused; the refusal then teaches it.
        let mut review_gap = None;
        let result = if problems.is_empty() {
            self.observed_evidence()
                .map_err(|e| render(&e))
                .and_then(|()| {
                    match self.task.state() {
                        TaskState::Completed => self.observed_evidence(),
                        TaskState::ReadyForReview => self.task.complete(),
                        _ => self.task.try_accept().and_then(|()| self.task.complete()),
                    }
                    .map_err(|e| {
                        // A rejection is the candidate judged wrong; an absent review is not.
                        judged_wrong |= matches!(
                            e,
                            GateError::OutOfScope(_)
                                | GateError::OpenFinding(_)
                                | GateError::ReviewRejected { .. }
                        );
                        review_gap = match e {
                            GateError::ReviewRejected { .. } => Some(true),
                            GateError::ReviewRequired | GateError::ReviewNotIndependent => {
                                Some(false)
                            }
                            _ => None,
                        };
                        render(&e)
                    })
                })
        } else {
            // An unlinked CHECK line refuses before any acceptance is tried.
            Err(problems.join("; "))
        };
        match result {
            Ok(()) => {
                reasons.push("completed".to_owned());
                // G15: the completed task keeps the candidate it completed on; say so when the
                // tree has since moved on. A warning, never a refusal.
                if self.task.state() == TaskState::Completed
                    && self.tree_changed_since_completion(dir)?
                {
                    reasons.push("tree changed since completion".to_owned());
                    return Ok(Decision::System(
                        "GATE: tree changed since completion; the completed task still stands on the candidate it completed on".to_owned(),
                    ));
                }
                Ok(Decision::None)
            }
            Err(why) => {
                reasons.push(why.clone());
                let mut round = "";
                if autonomous
                    && repair
                    && matches!(
                        self.task.state(),
                        TaskState::Verifying | TaskState::ReadyForReview
                    )
                {
                    let current = self.task.candidate().cloned();
                    if !(judged_wrong || self.observed_failure()) {
                        round = " (no repair round spent: no required check has been observed failing on this candidate)";
                    } else if current.is_some()
                        && self.gate.repair_charged.as_deref() == current.as_ref()
                    {
                        round = " (no repair round spent: a round was already charged on this version of the code; change the code before the next repair)";
                    } else {
                        match self.task.request_repair() {
                            Ok(()) => self.gate.repair_charged = current.map(Box::new),
                            Err(error) => reasons.push(render(&error)),
                        }
                    }
                }
                let state = state_name(self.task.state());
                if only_pending_job && self.gate.continuations >= 1 {
                    return Ok(Decision::System(format!(
                        "GATE: waiting, task remains {state}; {why}; nothing is certified"
                    )));
                }
                if self.gate.continuations >= 3 {
                    return Ok(Decision::System(format!(
                        "GATE: refusal cap reached; task remains {state}"
                    )));
                }
                self.gate.continuations += 1;
                let run_instead: String = instructions.iter().map(|i| format!(" {i}")).collect();
                let next = match review_gap {
                    Some(rejected) => review_protocol(rejected, &self.task_id),
                    None if !autonomous && header::parse(message).is_some() => {
                        "record the check with CHECK lines".to_owned()
                    }
                    None => "run the required check commands".to_owned(),
                };
                Ok(Decision::Block(format!(
                    "GATE refuses completion (refusal {} of 3 this turn): state {state}{round}; {why}; {next}.{run_instead}",
                    self.gate.continuations
                )))
            }
        }
    }

    /// WP5: record the review a final message opens with, and return the rest of the message for
    /// the header. Not a decision, a duplicate, or one the core refuses (a terminal task, an
    /// unobserved candidate) is journaled and stores nothing; the journal gets identifiers, the
    /// decision, the provenance and the finding count, never finding text.
    fn take_review<'m>(
        &mut self,
        message: &'m str,
        dir: &Path,
        agent: &str,
        now: u64,
        reasons: &mut Vec<String>,
    ) -> Result<&'m str, String> {
        let Some(parsed) = crate::review::parse(message) else {
            return Ok(message);
        };
        let Some(decision) = parsed.decision else {
            reasons.push(
                "REVIEW line is not a decision (APPROVED or REJECTED): nothing recorded".to_owned(),
            );
            return Ok(parsed.rest);
        };
        let Ok(current) = &self.observed else {
            reasons
                .push("review not recorded: candidate identity could not be observed".to_owned());
            return Ok(parsed.rest);
        };
        // The chair and any agent that changed something in this task review their own work.
        let provenance = if agent != "main" && !observation::made_mutation(dir, agent)? {
            ReviewProvenance::Observed
        } else {
            ReviewProvenance::CallerAsserted
        };
        let review = ReviewRecord {
            reviewer: agent.to_owned(),
            model: observation::delegated_model(dir, agent)?,
            decision,
            findings: parsed.findings,
            at: now,
            candidate: current.id.clone(),
            provenance,
        };
        let what = format!(
            "review {} by {agent} ({}, {} findings)",
            state::decision_name(decision),
            state::provenance_name(provenance),
            review.findings.len()
        );
        if self.gate.reviews.iter().any(|s| {
            s.review.reviewer == review.reviewer
                && s.review.candidate == review.candidate
                && s.review.decision == review.decision
        }) {
            reasons.push(format!("{what} already recorded"));
            return Ok(parsed.rest);
        }
        match self.task.record_review(review.clone()) {
            Ok(()) => {
                let tree = self.tree.get().map(str::to_owned);
                self.gate.reviews.push(StoredReview { review, tree });
                reasons.push(format!("recorded {what}"));
            }
            Err(e) => reasons.push(format!("{what} not recorded: {}", render(&e))),
        }
        Ok(parsed.rest)
    }

    /// WP5, as G9 does for receipts: a review made on another candidate with the current tree is
    /// copied onto the current candidate, rejections included, so a plain commit neither sheds a
    /// rejection nor loses an approval. A review stored with no tree is not copied.
    fn transfer_reviews(&mut self, reasons: &mut Vec<String>) {
        let Ok(current) = &self.observed else {
            return;
        };
        if self.task.state().is_terminal() {
            return;
        }
        let mut copies: Vec<StoredReview> = Vec::new();
        for stored in &self.gate.reviews {
            let r = &stored.review;
            let same = |o: &StoredReview| {
                o.review.candidate == current.id
                    && o.review.reviewer == r.reviewer
                    && o.review.decision == r.decision
            };
            if r.candidate == current.id
                || stored.tree.is_none()
                || stored.tree.as_deref() != self.tree.get()
                || self.gate.reviews.iter().any(same)
                || copies.iter().any(same)
            {
                continue;
            }
            copies.push(StoredReview {
                review: ReviewRecord {
                    candidate: current.id.clone(),
                    ..r.clone()
                },
                tree: stored.tree.clone(),
            });
        }
        for copy in copies {
            let who = copy.review.reviewer.clone();
            match self.task.record_review(copy.review.clone()) {
                Ok(()) => {
                    reasons.push(format!("review by {who} transferred: same tree"));
                    self.gate.reviews.push(copy);
                }
                Err(e) => reasons.push(format!("review by {who} not transferred: {}", render(&e))),
            }
        }
    }

    /// WP5: a foreground agent's SubagentStop arrives before its Agent tool's PostToolUse, which
    /// is the first to name the model; fill it in on the reviews that agent recorded.
    fn backfill_model(&mut self, payload: &Value, reasons: &mut Vec<String>) {
        let response = &payload["tool_response"];
        let (Some("completed"), Some(agent), Some(model)) = (
            text(response, "status"),
            text(response, "agentId"),
            text(response, "resolvedModel"),
        ) else {
            return;
        };
        let mut filled = false;
        for stored in &mut self.gate.reviews {
            if stored.review.reviewer == agent && stored.review.model.is_none() {
                stored.review.model = Some(model.to_owned());
                filled = true;
            }
        }
        if filled {
            self.task
                .set_reviews(self.gate.reviews.iter().map(|s| s.review.clone()).collect());
            reasons.push(format!("review model of {agent} recorded"));
        }
    }

    /// `Err` is an adapter error, answered through `adapter_error` by the caller.
    fn pre_tool_use(&mut self, payload: &Value) -> Result<Decision, String> {
        self.gate.continuations = 0;
        if !read_only_tool(payload) {
            match self
                .evidence_dir
                .as_ref()
                .map(|dir| crate::autonomy::is_paused(dir))
            {
                Some(Ok(true)) => {
                    return Ok(Decision::Deny(
                        "GATE: task is paused; resume through a new user prompt before mutation"
                            .to_owned(),
                    ));
                }
                Some(Err(_)) => return Err("task lifecycle state unavailable".to_owned()),
                _ => {}
            }
        }
        // G14: the same read-only tools and commands an unclassified prompt may use stay open,
        // and the refusal names the way on.
        if self.task.state().is_terminal()
            && !inspection(payload)
            && self
                .evidence_dir
                .as_ref()
                .is_some_and(|dir| crate::autonomy::is_autonomous_task(dir).unwrap_or(true))
        {
            return Ok(Decision::Deny(format!(
                "GATE: task is {}; read-only tools and commands still run, and a new work prompt starts the next task (intent_apply start)",
                state_name(self.task.state())
            )));
        }
        let tool = text(payload, "tool_name").unwrap_or_default();
        let input = &payload["tool_input"];
        if matches!(tool, "Bash" | "PowerShell")
            && input["run_in_background"].as_bool() == Some(true)
            && let Some(command) = text(input, "command")
            && let Some(criterion) = self.criteria.iter().find(|c| c.accepts(command))
        {
            return Ok(Decision::Deny(format!(
                "GATE: {} is a required check; run it in the foreground. Background runs are never counted.",
                criterion.id.0
            )));
        }
        // G19: a required check behind a pipe runs, but its exit code is the pipe's, so the
        // gate never counts it; say so before the run, not after.
        if matches!(tool, "Bash" | "PowerShell")
            && let Some(command) = text(input, "command")
            && let Some(head) = observation::before_pipe(command)
            && let Some(criterion) = self.criteria.iter().find(|c| c.accepts(head))
        {
            return Ok(Decision::Context {
                event: "PreToolUse",
                text: format!(
                    "GATE: {} will not count: run `{head}` without a pipe",
                    criterion.id.0
                ),
            });
        }
        if !matches!(tool, "Write" | "Edit" | "Agent") {
            return Ok(Decision::None);
        }
        let state = self.task.state();
        if state.is_terminal() {
            return Ok(Decision::Deny(format!(
                "GATE: {tool} refused: task is {}",
                state_name(state)
            )));
        }
        Ok(match &self.observed {
            Err(why) => Decision::Deny(format!(
                "GATE: {tool} refused: candidate identity could not be observed ({why})"
            )),
            Ok(_) => Decision::None,
        })
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

    /// Whether the worktree content differs from the tree the task completed on. Unknown trees (no
    /// run recorded one) never count as a change.
    fn tree_changed_since_completion(&self, dir: &Path) -> Result<bool, String> {
        let (Some(done), Ok(now)) = (self.task.candidate(), &self.observed) else {
            return Ok(false);
        };
        if *done == now.id {
            return Ok(false);
        }
        let runs = observation::read_runs(dir, &self.task_id)?;
        let trees: Vec<&str> = runs
            .iter()
            .filter(|run| &run.candidate == done)
            .filter_map(|run| run.tree.as_deref())
            .collect();
        Ok(!trees.is_empty() && !trees.iter().any(|t| self.tree.get() == Some(*t)))
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
            // M7: in strict, the baseline must have failed on the starting candidate.
            if self.task.requires_reproduction() && criterion.is_baseline() {
                let starting = self.task.starting_candidate();
                match observed.last() {
                    Some(r)
                        if Some(&r.candidate) == starting
                            && r.exit_code.is_some_and(|code| code != 0) => {}
                    Some(_) => return Err(GateError::NotReproduced(criterion.id.clone())),
                    None => return Err(GateError::MissingReceipt(criterion.id.clone())),
                }
                continue;
            }
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

    /// M6, strict: each criterion with `min_tests` whose latest observed receipt on the current
    /// candidate came from a run that showed fewer passing tests, or no test summary.
    fn too_few_tests(&self, dir: &Path) -> Result<Vec<String>, String> {
        let minimums = state::min_tests(dir)?;
        let Some(current) = self.task.candidate() else {
            return Ok(Vec::new());
        };
        let runs = observation::read_runs(dir, &self.task_id)?;
        let mut problems = Vec::new();
        for (id, min) in &minimums {
            let Some(receipt) = self
                .gate
                .receipts
                .iter()
                .rev()
                .find(|r| &r.criterion.0 == id && &r.candidate == current)
            else {
                continue;
            };
            let seen = runs
                .iter()
                .find(|run| run.reference == receipt.output_ref)
                .and_then(|run| run.passed_tests);
            if seen.is_none_or(|n| n < *min) {
                problems.push(format!(
                    "{id} needs at least {min} passing tests; its run showed {} (level strict)",
                    seen.map_or("no test summary".to_owned(), |n| n.to_string())
                ));
            }
        }
        Ok(problems)
    }

    /// True when some mandatory criterion's latest hook-observed run on the current candidate
    /// exited non-zero: the candidate was judged and found wrong.
    fn observed_failure(&self) -> bool {
        let (Some(current), Some(dir)) = (self.task.candidate(), self.evidence_dir.as_ref()) else {
            return false;
        };
        // A failing baseline is what strict asks for, not a sign the candidate is wrong.
        let reproduction = self.task.requires_reproduction();
        self.criteria
            .iter()
            .filter(|c| c.mandatory && !(reproduction && c.is_baseline()))
            .any(|criterion| {
                self.gate
                    .receipts
                    .iter()
                    .rev()
                    .find(|r| {
                        r.criterion == criterion.id
                            && &r.candidate == current
                            && receipt_is_observed(dir, &self.task_id, r)
                    })
                    .is_some_and(|r| r.exit_code.is_some_and(|code| code != 0))
            })
    }

    /// Receipt status per criterion, read without any transition.
    fn summary(&self) -> String {
        let (mut current, mut stale, mut missing) = (Vec::new(), Vec::new(), Vec::new());
        let cp = self.task.checkpoint();
        // Repair: restore() skips receipt replay for terminal states, so after a resume the
        // persisted gate receipts are the only evidence the summary can still report.
        let recorded: Vec<&str> = if cp.state.is_terminal() {
            // The acceptance path refuses mcp-asserted receipts here and a failing run
            // satisfies nothing; the display must agree with it.
            self.criteria
                .iter()
                .filter(|c| self.passing_on_checkpoint(c))
                .map(|c| c.id.0.as_str())
                .collect()
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
    // G15: a terminal task keeps the candidate it ended on; moving it made the completed
    // evidence look stale and refused the next task's start after any new commit.
    match previous.clone().filter(|_| task.state().is_terminal()) {
        Some(id) => task.observe_candidate(Candidate {
            id,
            provenance: "journal".to_owned(),
            changed_paths: current.changed_paths.clone(),
        })?,
        None => task.observe_candidate(current.clone())?,
    }
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

#[cfg(test)]
mod fail_closed_tests {
    use super::{blocking_words, lone_surrogates_replaced};

    /// The JSON escape `\u` followed by `hex`, built so no escape appears in this source.
    fn esc(hex: &str) -> String {
        format!("{}u{hex}", '\\')
    }

    #[test]
    fn only_lone_surrogate_escapes_are_replaced() {
        let (high, low, fffd) = (esc("d800"), esc("DC00"), esc("FFFD"));
        let pair = format!("{}{}", esc("d83d"), esc("de00"));
        let backslash = '\\';
        for (raw, cleaned) in [
            (format!("\"a{high}b\""), format!("\"a{fffd}b\"")),
            (format!("\"a{low}b\""), format!("\"a{fffd}b\"")),
            (format!("\"{high}{pair}\""), format!("\"{fffd}{pair}\"")),
            (format!("\"pair {pair}\""), format!("\"pair {pair}\"")),
            (
                format!("\"escaped {backslash}{high}\""),
                format!("\"escaped {backslash}{high}\""),
            ),
            (
                format!("\"short {}\"", esc("d80")),
                format!("\"short {}\"", esc("d80")),
            ),
            (
                format!("\"{} n\"", esc("00e9")),
                format!("\"{} n\"", esc("00e9")),
            ),
        ] {
            assert_eq!(lone_surrogates_replaced(&raw), cleaned, "{raw}");
        }
        let parsed: String =
            serde_json::from_str(&lone_surrogates_replaced(&format!("\"a{high}b\""))).unwrap();
        assert_eq!(parsed, "a\u{fffd}b");
    }

    #[test]
    fn fail_closed_texts_never_name_something_absent() {
        assert_eq!(
            blocking_words("x: No such file or directory; Missing field; NOT FOUND"),
            "x: no file or directory; absent field; absent"
        );
    }
}

#[cfg(test)]
mod deny_path_tests {
    use super::forbidden_effect;
    use serde_json::json;
    use std::fs;

    /// A link whose target is missing cannot be placed, so a `deny_path` refuses an edit
    /// through it on its own, without relying on the scope check running first.
    #[test]
    fn deny_path_refuses_an_edit_through_an_unresolvable_link() {
        let root =
            std::env::temp_dir().join(format!("supervisor-deny-path-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (project, outside) = (root.join("project"), root.join("outside"));
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let link = project.join("src").join("out");
        #[cfg(unix)]
        let target = {
            std::os::unix::fs::symlink(outside.join("new.rs"), &link).unwrap();
            link.clone()
        };
        #[cfg(windows)]
        let target = {
            let gone = outside.join("gone");
            fs::create_dir_all(&gone).unwrap();
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&gone)
                .output()
                .unwrap();
            assert!(made.status.success(), "mklink /J failed");
            fs::remove_dir(&gone).unwrap();
            link.join("x.rs")
        };
        let payload = json!({"tool_name":"Write","cwd":project,
            "tool_input":{"file_path":target,"content":"x"}});
        let effect = forbidden_effect(&payload, &["deny_path:src/".to_owned()]);
        #[cfg(windows)]
        let _ = fs::remove_dir(&link);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(effect.as_deref(), Some("deny_path:src/"));
    }
}
