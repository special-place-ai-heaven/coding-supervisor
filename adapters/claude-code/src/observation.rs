//! Hook-observed dispatches and completions. No command output is searched for status.

use std::path::{Path, PathBuf};

use coding_supervisor::{Candidate, CandidateId, Criterion};
use serde_json::{Value, json};

use crate::state;

const TC: &str = "mcp__terminal_commander__";

pub(crate) struct Run {
    pub agent_id: String,
    pub command: String,
    pub cwd: String,
    pub execution_root: String,
    pub candidate: CandidateId,
    /// Git's id of the worktree content when the candidate was observed (schema 4, G9).
    pub tree: Option<String>,
    /// Candidates this run's evidence was transferred to because their tree is identical.
    pub transferred_to: Vec<CandidateId>,
    pub exit_code: Option<i32>,
    pub reference: String,
    pub t: u64,
    pub finished: u64,
    pub recovered_by: Option<String>,
    /// Passing tests in the run's test summary, when its output had one (M6); never the output.
    pub passed_tests: Option<u64>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().filter(|s| !s.is_empty())
}

pub(crate) fn cwd(path: &str, base: &str) -> String {
    let path = Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        PathBuf::from(base).join(path)
    };
    let resolved = absolute.canonicalize().unwrap_or(absolute);
    let value = resolved.to_string_lossy().replace('\\', "/");
    let value = value.strip_prefix("//?/").unwrap_or(&value);
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}

fn identity(input: &Value, tool: &str) -> Option<(Value, String)> {
    if matches!(
        tool.strip_prefix(TC),
        Some("run_and_watch" | "command_start_combed")
    ) {
        let argv = input.get("argv")?.as_array()?;
        let args: Vec<&str> = argv.iter().map(Value::as_str).collect::<Option<_>>()?;
        if args.is_empty() || args[0].is_empty() {
            return None;
        }
        let rendered = args
            .iter()
            .map(|arg| {
                if !arg.is_empty()
                    && arg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_./\\:-=".contains(c))
                {
                    (*arg).to_owned()
                } else {
                    Value::from(*arg).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        return Some((json!({"argv": args}), rendered));
    }
    let line = match tool {
        "Bash" | "PowerShell" => text(input, "command")?,
        _ if tool.strip_prefix(TC) == Some("shell_exec") => text(input, "shell_line")?,
        _ => return None,
    };
    Some((
        json!({"shell_line": line, "shell_sha256": crate::sha256::hex(input.get("shell").unwrap_or(&Value::Null).to_string().as_bytes())}),
        line.to_owned(),
    ))
}

// Only the inherited local execution profile is approved. Per-call runner changes
// need a separately approved profile, not a fingerprint of caller-chosen settings.
fn execution_profile(input: &Value) -> Value {
    let mut approved = input.get("shell").is_none_or(Value::is_null);
    let mut target_digest = None;
    match input.get("env") {
        None | Some(Value::Null) => {}
        Some(Value::Array(values)) if values.is_empty() => {}
        Some(Value::Array(values)) if values.len() == 1 => {
            let value = &values[0];
            let inherited = std::env::var("CARGO_TARGET_DIR")
                .ok()
                .filter(|v| !v.is_empty());
            if value.as_object().is_some_and(|fields| fields.len() == 2)
                && text(value, "key") == Some("CARGO_TARGET_DIR")
                && inherited
                    .as_deref()
                    .is_some_and(|target| text(value, "value") == Some(target))
            {
                // This setting is nonsecret and already approved by the host. Its
                // digest binds dispatch to completion without storing its value.
                target_digest = inherited.map(|target| crate::sha256::hex(target.as_bytes()));
            } else {
                approved = false;
            }
        }
        _ => approved = false,
    }
    json!({"name":"local-default-v1", "approved":approved, "cargo_target_sha256":target_digest})
}

/// Validate stored approval without reapproving old records from today's environment.
pub(crate) fn approved_context(record: &Value) -> bool {
    let Some(profile) = record.get("execution_profile") else {
        return false;
    };
    valid_identity(record)
        && profile.as_object().is_some_and(|fields| fields.len() == 3)
        && text(profile, "name") == Some("local-default-v1")
        && profile.get("approved").and_then(Value::as_bool) == Some(true)
        && profile.get("cargo_target_sha256").is_some_and(|value| {
            value.is_null()
                || value.as_str().is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        })
        && text(record, "execution_root").is_some()
        && text(record, "execution_root") == text(record, "execution_project")
        && text(record, "execution_project")
            == text(record, "cwd")
                .map(|cwd| crate::location::canonical_project(Path::new(cwd)))
                .as_deref()
}

// Schema 3 binds command identity to an approved execution profile and root.
// Older records stay on disk, but cannot authorize completion without a rerun.
fn valid_identity(record: &Value) -> bool {
    // Schema 4 adds the optional `tree` (G9); a row without it keeps the exact candidate rule.
    if !matches!(
        record.get("identity_schema").and_then(Value::as_u64),
        Some(3 | 4)
    ) {
        return false;
    }
    let (Some(tool), Some(stored), Some(command)) = (
        text(record, "tool"),
        record.get("identity"),
        text(record, "command"),
    ) else {
        return false;
    };
    if matches!(tool, "Bash" | "PowerShell") || tool.strip_prefix(TC) == Some("shell_exec") {
        stored.as_object().is_some_and(|fields| fields.len() == 2)
            && text(stored, "shell_line") == Some(command)
            && text(stored, "shell_sha256").is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    } else {
        identity(stored, tool)
            .is_some_and(|(expected, rendered)| expected == *stored && rendered == command)
    }
}

fn records(dir: &Path) -> Result<Vec<Value>, String> {
    let path = dir.join("runs.jsonl");
    match std::fs::read_to_string(&path) {
        Ok(s) => s
            .lines()
            .enumerate()
            .map(|(i, s)| {
                serde_json::from_str(s).map_err(|_| {
                    format!(
                        "{}:{}: invalid command journal record",
                        path.display(),
                        i + 1
                    )
                })
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

// MCP may return a structured object, JSON text, or one text content block. Claude
// can pass that content array directly, without its enclosing MCP object. Unwrap
// only those envelopes; stdout/tail/signals are never recursively interpreted.
fn response(value: &Value) -> Option<Value> {
    let value = if let Some(s) = value.as_str() {
        serde_json::from_str(s).ok()?
    } else {
        value.clone()
    };
    if value.get("isError").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if let Some(v) = value.get("structuredContent") {
        return v.is_object().then(|| v.clone());
    }
    if let Some(content) = value
        .as_array()
        .or_else(|| value.get("content").and_then(Value::as_array))
    {
        if content.len() != 1 || text(&content[0], "type") != Some("text") {
            return None;
        }
        return serde_json::from_str(text(&content[0], "text")?).ok();
    }
    value.is_object().then_some(value)
}

fn same_scope(record: &Value, payload: &Value, task: &str, agent: &str) -> bool {
    text(record, "task_id") == Some(task)
        && text(record, "agent_id") == Some(agent)
        && record.get("session_id") == Some(&json!(payload.get("session_id")))
}

fn terminal(v: &Value) -> Option<i32> {
    let healthy = v.get("degraded").and_then(Value::as_bool).map_or_else(
        || {
            v.pointer("/process_observation/stdout/state/state")
                .and_then(Value::as_str)
                == Some("complete")
                && v.pointer("/process_observation/stderr/state/state")
                    .and_then(Value::as_str)
                    == Some("complete")
                && v.pointer("/process_cleanup/state").and_then(Value::as_str) == Some("complete")
        },
        |degraded| !degraded,
    );
    // Repair: TC "failed" jobs now journal their exit code instead of staying pending.
    if !matches!(text(v, "state"), Some("exited" | "failed"))
        || v.get("complete").and_then(Value::as_bool) == Some(false)
        || !healthy
        || v.get("pipeline_exit_masked").and_then(Value::as_bool) != Some(false)
    {
        return None;
    }
    let exit = i32::try_from(v.get("exit_code")?.as_i64()?).ok()?;
    // A failed status with a zero exit is contradictory daemon output; journaling
    // it would mint a passing receipt from a job the daemon says failed.
    if text(v, "state") == Some("failed") && exit == 0 {
        return None;
    }
    Some(exit)
}

/// The caller serializes this operation with every other task mutation.
pub(crate) fn observe(
    dir: &Path,
    payload: &Value,
    task: &str,
    agent: &str,
    candidate: Option<&Candidate>,
    now: u64,
    criteria: &[Criterion],
) -> Result<bool, String> {
    let session = text(payload, "session_id").unwrap_or_default();
    if !crate::autonomy::owns_session(dir, session)? {
        return Ok(false);
    }
    let event = text(payload, "hook_event_name").unwrap_or_default();
    let Some(tool) = text(payload, "tool_name") else {
        return Ok(false);
    };
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    let shell = matches!(tool, "Bash" | "PowerShell");
    let tc = tool.strip_prefix(TC);
    let starts = shell
        || matches!(
            tc,
            Some("run_and_watch" | "command_start_combed" | "shell_exec")
        );
    // A remote daemon's cwd does not prove it ran against this local candidate.
    if tc.is_some() && text(input, "target_id").is_some() {
        return Ok(false);
    }
    // A successful shell launch is not a successful background command completion.
    if shell
        && (input.get("run_in_background").and_then(Value::as_bool) == Some(true)
            || payload.get("tool_response").is_some_and(|r| {
                [
                    "backgroundTaskId",
                    "background_task_id",
                    "task_id",
                    "taskId",
                ]
                .iter()
                .any(|key| r.get(*key).is_some_and(|v| !v.is_null()))
                    || r.get("is_async").and_then(Value::as_bool) == Some(true)
                    || r.get("background").and_then(Value::as_bool) == Some(true)
                    || matches!(
                        text(r, "status"),
                        Some("running" | "started" | "background")
                    )
            }))
    {
        return Ok(false);
    }
    let previous = records(dir)?;
    let append = |v: &Value| state::append_line(&dir.join("runs.jsonl"), v);
    if event == "PreToolUse" && starts {
        let (Some(candidate), Some(id), Some(base), Some((identity, command))) = (
            candidate,
            text(payload, "tool_use_id"),
            text(payload, "cwd"),
            identity(input, tool),
        ) else {
            return Ok(false);
        };
        // Only deliberately approved contract commands may carry raw text on disk.
        // Unrelated routine commands execute normally but produce no receipt evidence.
        if !criteria.iter().any(|criterion| criterion.accepts(&command)) {
            return Ok(false);
        }
        // TC's omitted cwd is daemon-owned, not necessarily the hook's project cwd.
        let Some(workdir) = (if shell {
            Some(base)
        } else {
            text(input, "cwd")
        }) else {
            return Ok(false);
        };
        if !shell && !Path::new(workdir).is_absolute() {
            return Ok(false);
        }
        // G5: a task bound to a worktree counts only runs in it, wherever the hook runs.
        let bound = crate::binding::bound_root(dir);
        let project = crate::location::canonical_project(Path::new(workdir));
        let hook_project = crate::location::canonical_project(Path::new(base));
        if bound
            .as_deref()
            .map_or(project != hook_project, |b| project != b)
        {
            return Ok(false);
        }
        if previous
            .iter()
            .any(|v| same_scope(v, payload, task, agent) && text(v, "dispatch_id") == Some(id))
        {
            return Ok(false);
        }
        let tree =
            crate::git::worktree_tree(Path::new(if bound.is_some() { workdir } else { base }), dir);
        append(&json!({"kind":"dispatch", "task_id":task, "agent_id":agent,
            "session_id":payload.get("session_id"), "dispatch_id":id, "tool":tool,
            "identity_schema":4, "tree":tree, "identity":identity, "command":command, "cwd":cwd(workdir, base),
            "execution_project":project, "execution_root":project, "bound_root":bound,
            "execution_profile":execution_profile(input),
            "candidate":{"base_commit":candidate.id.base_commit,"dirty_digest":candidate.id.dirty_digest},
            "target_id":input.get("target_id"), "t":now}))?;
        return Ok(true);
    }
    if !matches!(event, "PostToolUse" | "PostToolUseFailure") {
        return Ok(false);
    }
    // G13: a job its owner stopped is no longer pending. The row records no exit, so it can
    // never count as a completion.
    if tc == Some("command_stop") {
        if event != "PostToolUse" {
            return Ok(false);
        }
        let Some(job) = previous.iter().find(|v| {
            text(v, "kind") == Some("job")
                && same_scope(v, payload, task, agent)
                && text(v, "job_id").is_some()
                && text(v, "job_id") == text(input, "job_id")
        }) else {
            return Ok(false);
        };
        let mut stopped = job.clone();
        stopped["kind"] = "stopped".into();
        stopped["t"] = now.into();
        append(&stopped)?;
        return Ok(true);
    }
    let dispatch = if starts {
        previous.iter().find(|v| {
            text(v, "kind") == Some("dispatch")
                && same_scope(v, payload, task, agent)
                && text(v, "dispatch_id") == text(payload, "tool_use_id")
                && text(v, "tool") == Some(tool)
                && identity(input, tool)
                    .is_some_and(|(identity, _)| v.get("identity") == Some(&identity))
                && v.get("execution_profile") == Some(&execution_profile(input))
        })
    } else if matches!(tc, Some("command_status" | "bucket_wait")) {
        let mut found = None;
        for v in &previous {
            // G25: in its own session the chair may record the exit of a worker's job.
            let same_owner = same_scope(v, payload, task, agent)
                || (text(v, "task_id") == Some(task) && text(v, "session_id") == Some(session));
            let recovered = text(v, "task_id") == Some(task)
                && crate::autonomy::recovers_session(
                    dir,
                    task,
                    text(v, "session_id").unwrap_or_default(),
                    session,
                )?;
            if text(v, "kind") == Some("job")
                && (same_owner || recovered)
                && (if tc == Some("command_status") {
                    text(v, "job_id") == text(input, "job_id")
                        && v.get("target_id") == Some(&json!(input.get("target_id")))
                } else {
                    text(v, "bucket_id") == text(input, "bucket_id")
                })
            {
                found = Some(v);
                break;
            }
        }
        found
    } else {
        None
    };
    let Some(dispatch) = dispatch else {
        return Ok(false);
    };
    if !starts
        && text(dispatch, "session_id") != Some(session)
        && text(payload, "cwd")
            .map(|cwd| crate::location::canonical_project(Path::new(cwd)))
            .as_deref()
            != text(dispatch, "execution_project")
    {
        return Ok(false);
    }
    if starts && !shell {
        let Some(base) = text(payload, "cwd") else {
            return Ok(false);
        };
        let Some(workdir) = text(input, "cwd") else {
            return Ok(false);
        };
        if text(dispatch, "cwd") != Some(cwd(workdir, base).as_str()) {
            return Ok(false);
        }
    }
    if !approved_context(dispatch) {
        return Ok(false);
    }
    // Require the binding written at dispatch, including for pre-upgrade journals.
    // A nested repository/worktree is a different candidate even below the same path.
    let Some(workdir) = text(dispatch, "cwd") else {
        return Ok(false);
    };
    if text(dispatch, "execution_project")
        != Some(crate::location::canonical_project(Path::new(workdir)).as_str())
    {
        return Ok(false);
    }
    let mut run = dispatch.clone();
    let mut completion_trust = "observed";
    let id = text(dispatch, "dispatch_id").unwrap_or_default();
    if previous.iter().any(|v| {
        matches!(text(v, "kind"), Some("completed" | "invalidated"))
            && text(v, "task_id") == text(dispatch, "task_id")
            && text(v, "agent_id") == text(dispatch, "agent_id")
            && v.get("session_id") == dispatch.get("session_id")
            && text(v, "dispatch_id") == Some(id)
    }) {
        return Ok(false);
    }
    let current =
        candidate.map(|c| json!({"base_commit":c.id.base_commit,"dirty_digest":c.id.dirty_digest}));
    if current.as_ref() != dispatch.get("candidate") {
        // An observed mismatch permanently invalidates this dispatch, including
        // a later status replay after the checkout has reverted to its old bytes.
        run["kind"] = "invalidated".into();
        run["invalidation"] = "candidate changed or not observed".into();
        append(&run)?;
        return Ok(false);
    }
    let exit = if shell {
        if dispatch
            .pointer("/identity/shell_line")
            .and_then(Value::as_str)
            .is_some_and(unquoted_pipe)
        {
            return Ok(false);
        }
        if event == "PostToolUse" {
            if payload
                .pointer("/tool_response/interrupted")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Ok(false);
            }
            Some(0)
        } else {
            text(payload, "error")
                .and_then(|s| s.split_once("Exit code "))
                .and_then(|(_, s)| s.split_whitespace().next())
                .and_then(|s| s.parse::<i32>().ok())
        }
    } else {
        let Some(v) = payload.get("tool_response").and_then(response) else {
            return Ok(false);
        };
        let Some(job) = text(&v, "job_id") else {
            return Ok(false);
        };
        if text(dispatch, "kind") == Some("job") && text(dispatch, "job_id") != Some(job) {
            return Ok(false);
        }
        run["job_id"] = job.into();
        if let Some(bucket) = v.get("bucket_id") {
            run["bucket_id"] = bucket.clone();
        }
        let masked = v.get("pipeline_exit_masked").and_then(Value::as_bool) == Some(true)
            || dispatch
                .get("pipeline_exit_masked")
                .and_then(Value::as_bool)
                == Some(true)
            || dispatch
                .pointer("/identity/shell_line")
                .and_then(Value::as_str)
                .is_some_and(unquoted_pipe)
            || dispatch
                .pointer("/identity/argv")
                .and_then(Value::as_array)
                .is_some_and(|args| shell_argv_pipe(args));
        run["pipeline_exit_masked"] = masked.into();
        if starts
            && !previous
                .iter()
                .any(|p| text(p, "kind") == Some("job") && text(p, "dispatch_id") == Some(id))
        {
            run["kind"] = "job".into();
            append(&run)?;
        }
        if masked {
            return Ok(false);
        }
        match text(&v, "outcome_trust") {
            Some("observed") => {}
            Some("reconstructed")
                if tc == Some("command_status")
                    && text(dispatch, "kind") == Some("job")
                    && text(dispatch, "bucket_id").is_some()
                    && text(dispatch, "bucket_id") == text(&v, "bucket_id")
                    && v.get("restarted").and_then(Value::as_bool) == Some(true)
                    && v.pointer("/process_observation/stdout/state/state")
                        .and_then(Value::as_str)
                        == Some("complete")
                    && v.pointer("/process_observation/stderr/state/state")
                        .and_then(Value::as_str)
                        == Some("complete")
                    && v.pointer("/process_cleanup/state").and_then(Value::as_str)
                        == Some("complete") =>
            {
                // TC 0.3.15 reconstructs these terminal facts from its durable
                // receipt. The protected dispatch/job journal supplies original
                // task, session, argv, cwd and candidate identity; no completion
                // had to reach supervisor before the TC restart. TC reconstructs
                // pipeline_exit_masked as false, so dispatch-side masking above
                // must still reject every pipeline we cannot attest.
                completion_trust = "tc-reconstructed";
            }
            _ => return Ok(false),
        }
        terminal(&v)
    };
    let Some(exit) = exit else { return Ok(false) };
    run["kind"] = "completed".into();
    run["completion_trust"] = completion_trust.into();
    run["exit_code"] = exit.into();
    if shell {
        let output = ["/tool_response/stdout", "/tool_response/stderr", "/error"]
            .iter()
            .filter_map(|p| payload.pointer(p).and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(passed) = passed_tests(&output) {
            run["passed_tests"] = passed.into();
        }
    }
    run["finished"] = now.into();
    if text(dispatch, "session_id") != Some(session) || text(dispatch, "agent_id") != Some(agent) {
        run["recovered_by_session"] = session.into();
        run["recovered_by_agent"] = agent.into();
    }
    append(&run)?;
    Ok(true)
}

/// Sanitized pending job context. It reports linkage, never a successful command outcome.
pub(crate) fn pending_jobs(dir: &Path, task: &str) -> Result<Value, String> {
    let (contract, _) = state::read_contract(dir)?;
    let Some(contract) = contract else {
        return Ok(json!([]));
    };
    let rows = records(dir)?;
    let mut jobs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in rows.iter().rev() {
        let (Some(dispatch), Some(job), Some(command), Some(workdir)) = (
            text(row, "dispatch_id"),
            text(row, "job_id"),
            text(row, "command"),
            text(row, "cwd"),
        ) else {
            continue;
        };
        if text(row, "kind") != Some("job")
            || text(row, "task_id") != Some(task)
            || !approved_context(row)
            || text(row, "execution_project")
                != Some(crate::location::canonical_project(Path::new(workdir)).as_str())
        {
            continue;
        }
        let key = json!([row.get("session_id"), row.get("agent_id"), dispatch]).to_string();
        if !seen.insert(key) {
            continue;
        }
        let same_dispatch = |other: &&Value| {
            text(other, "dispatch_id") == Some(dispatch)
                && other.get("task_id") == row.get("task_id")
                && other.get("session_id") == row.get("session_id")
                && other.get("agent_id") == row.get("agent_id")
        };
        if rows.iter().filter(same_dispatch).any(|other| {
            text(other, "kind") == Some("stopped")
                || text(other, "kind") == Some("completed")
                    && approved_context(other)
                    && matches!(
                        text(other, "completion_trust"),
                        Some("observed" | "tc-reconstructed")
                    )
        }) {
            continue;
        }
        let criteria: Vec<_> = contract
            .criteria
            .iter()
            .filter(|criterion| criterion.accepts(command))
            .map(|criterion| criterion.id.0.clone())
            .collect();
        if criteria.is_empty() {
            continue;
        }
        let invalidated = !crate::autonomy::dispatch_is_current(dir, dispatch)?
            || row.get("pipeline_exit_masked").and_then(Value::as_bool) == Some(true)
            || rows
                .iter()
                .filter(same_dispatch)
                .any(|other| text(other, "kind") == Some("invalidated"));
        let candidate = row.get("candidate");
        jobs.push(json!({"job_id":job,"bucket_id":text(row,"bucket_id"),"dispatch_id":dispatch,"candidate":{"base_commit":candidate.and_then(|c| text(c,"base_commit")),"dirty_digest":candidate.and_then(|c| text(c,"dirty_digest"))},"session_id":text(row,"session_id"),"agent_id":text(row,"agent_id"),"task_id":task,"t":row.get("t"),"cwd":workdir,"command":command,"criteria":criteria,"status":"pending","validity":if invalidated { "invalidated" } else { "current" }}));
    }
    jobs.reverse();
    Ok(jobs.into())
}

/// G16: a delegation older than this no longer counts as outstanding, so a lost SubagentStop
/// cannot hold Stop back forever.
/// ponytail: fixed two hours; make it configurable if a real worker runs longer.
const OUTSTANDING_TTL_SECS: u64 = 2 * 3600;

/// The parseable rows of `delegations.jsonl`, and whether any line was not parseable. A bad
/// line is skipped, never an error: it counts as "this agent mutated" for handbacks and as
/// "not outstanding" for Stop (`made_mutation`, `outstanding_background`).
fn delegation_rows(dir: &Path) -> Result<(Vec<Value>, bool), String> {
    match std::fs::read_to_string(dir.join("delegations.jsonl")) {
        Ok(rows) => {
            let mut malformed = false;
            let parsed = rows
                .lines()
                .filter_map(|line| {
                    let row = serde_json::from_str(line).ok();
                    malformed |= row.is_none();
                    row
                })
                .collect();
            Ok((parsed, malformed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Vec::new(), false)),
        Err(e) => Err(e.to_string()),
    }
}

/// G16: journal a background Agent launch (PostToolUse `async_launched` names its `agentId`), with
/// the model the harness reported, which the agent's SubagentStop has no field for (WP5).
pub(crate) fn record_delegation(
    dir: &Path,
    payload: &Value,
    task: &str,
    now: u64,
    content: Option<&str>,
) -> Result<(), String> {
    let launched = payload
        .pointer("/tool_response/status")
        .and_then(Value::as_str)
        == Some("async_launched");
    let Some(agent) = payload
        .pointer("/tool_response/agentId")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if !launched {
        return Ok(());
    }
    state::append_line(
        &dir.join("delegations.jsonl"),
        &json!({"kind":"delegation","task_id":task,"session_id":text(payload,"session_id"),
            "agent_id":agent,"tool_use_id":text(payload,"tool_use_id"),
            "model":payload.pointer("/tool_response/resolvedModel").and_then(Value::as_str),
            "content":content,"t":now}),
    )
}

/// WP5: the full-content tree recorded at the latest background launch of `agent`: `None` with
/// no launch row (a foreground agent), `Some(None)` when the launch content is unknown or the
/// records hold a line that cannot be read. A new launch row for the same agent replaces it.
pub(crate) fn delegated_content(dir: &Path, agent: &str) -> Result<Option<Option<String>>, String> {
    let (rows, malformed) = delegation_rows(dir)?;
    // A line that cannot be read may be this agent's latest launch: a launch, content unknown.
    if malformed {
        return Ok(Some(None));
    }
    Ok(rows
        .iter()
        .rev()
        .find(|r| text(r, "kind") == Some("delegation") && text(r, "agent_id") == Some(agent))
        .map(|r| text(r, "content").map(str::to_owned)))
}

/// WP5: the model the launch of background agent `agent` reported, if any.
pub(crate) fn delegated_model(dir: &Path, agent: &str) -> Result<Option<String>, String> {
    let (rows, _) = delegation_rows(dir)?;
    Ok(rows
        .iter()
        .rev()
        .filter(|r| text(r, "kind") == Some("delegation") && text(r, "agent_id") == Some(agent))
        .find_map(|r| text(r, "model").map(str::to_owned)))
}

/// G16: the worker named by this SubagentStop has ended, so close its open delegation (the
/// caller only asks when the handback was not refused). Best effort: a write failure leaves the
/// delegation to expire. `root` is the state root; the delegation lives in the active task.
pub(crate) fn close_delegation(root: &Path, payload: &Value, now: u64) {
    let (Some(agent), Some(session)) = (text(payload, "agent_id"), text(payload, "session_id"))
    else {
        return;
    };
    let dir = match crate::lifecycle::active_task(root) {
        Ok(Some(active)) => active.dir,
        _ => root.to_path_buf(),
    };
    let Ok((rows, _)) = delegation_rows(&dir) else {
        return;
    };
    let ours =
        |r: &Value| text(r, "agent_id") == Some(agent) && text(r, "session_id") == Some(session);
    let open = rows
        .iter()
        .any(|r| text(r, "kind") == Some("delegation") && ours(r))
        && !rows
            .iter()
            .any(|r| text(r, "kind") == Some("delegation_closed") && ours(r));
    if open {
        let _ = state::append_line(
            &dir.join("delegations.jsonl"),
            &json!({"kind":"delegation_closed","agent_id":agent,"session_id":session,"t":now}),
        );
    }
}

/// G21: journal, once per agent, that `agent` was allowed a tool that is not read-only. G44:
/// the row keeps the worktree `tree` as it was before that first tool ran (asked for only
/// when the row is written), so its handback can tell whether anything changed.
pub(crate) fn record_mutation(
    dir: &Path,
    payload: &Value,
    task: &str,
    agent: &str,
    now: u64,
    tree: impl FnOnce() -> Option<String>,
) -> Result<(), String> {
    let (rows, _) = delegation_rows(dir)?;
    if rows
        .iter()
        .any(|r| text(r, "kind") == Some("mutation") && text(r, "agent_id") == Some(agent))
    {
        return Ok(());
    }
    state::append_line(
        &dir.join("delegations.jsonl"),
        &json!({"kind":"mutation","task_id":task,"session_id":text(payload,"session_id"),
            "agent_id":agent,"tool":text(payload,"tool_name"),"t":now,"tree":tree()}),
    )
}

/// G44: what `agent` may have changed: `None` when it was never allowed a tool that is not
/// read-only; otherwise the worktree tree from before its first such tool, when one was
/// recorded. A malformed journal counts as a mutation with no known tree.
pub(crate) fn mutation_tree(dir: &Path, agent: &str) -> Result<Option<Option<String>>, String> {
    let (rows, malformed) = delegation_rows(dir)?;
    let row = rows
        .iter()
        .find(|r| text(r, "kind") == Some("mutation") && text(r, "agent_id") == Some(agent));
    Ok(match row {
        _ if malformed => Some(None),
        Some(row) => Some(text(row, "tree").map(str::to_owned)),
        None => None,
    })
}

/// G21: whether `agent` was ever allowed a tool that is not read-only.
pub(crate) fn made_mutation(dir: &Path, agent: &str) -> Result<bool, String> {
    let (rows, malformed) = delegation_rows(dir)?;
    Ok(malformed
        || rows
            .iter()
            .any(|r| text(r, "kind") == Some("mutation") && text(r, "agent_id") == Some(agent)))
}

/// G16: agent ids of `session`'s background Agents that have not reached SubagentStop. Pending
/// Terminal Commander jobs are not included: an unfetched job keeps the G7 refusal.
pub(crate) fn outstanding_background(
    dir: &Path,
    session: &str,
    now: u64,
) -> Result<Vec<String>, String> {
    let (rows, _) = delegation_rows(dir)?;
    let mut waiting: Vec<String> = Vec::new();
    for row in rows
        .iter()
        .filter(|r| text(r, "kind") == Some("delegation") && text(r, "session_id") == Some(session))
    {
        let Some(agent) = text(row, "agent_id") else {
            continue;
        };
        let closed = rows.iter().any(|r| {
            text(r, "kind") == Some("delegation_closed")
                && text(r, "agent_id") == Some(agent)
                && text(r, "session_id") == Some(session)
        });
        let fresh = row["t"]
            .as_u64()
            .is_none_or(|t| t.saturating_add(OUTSTANDING_TTL_SECS) > now);
        if !closed && fresh && !waiting.iter().any(|w| w == agent) {
            waiting.push(agent.to_owned());
        }
    }
    Ok(waiting)
}

/// The candidates the adapter recorded as having the same tree as `run`'s dispatch (G9).
fn transfers(rows: &[Value], task: &str, run: &Value) -> Vec<CandidateId> {
    rows.iter()
        .filter(|t| {
            text(t, "kind") == Some("transfer")
                && text(t, "task_id") == Some(task)
                && text(t, "dispatch_id") == text(run, "dispatch_id")
                && t.get("from") == run.get("candidate")
        })
        .filter_map(|t| {
            let to = t.get("to")?;
            Some(CandidateId {
                base_commit: text(to, "base_commit")?.to_owned(),
                dirty_digest: text(to, "dirty_digest")?.to_owned(),
            })
        })
        .collect()
}

/// True when the journal holds a tree transfer of `dispatch` to `candidate` (G9).
pub(crate) fn transferred(
    rows: &[Value],
    task: &str,
    dispatch: &str,
    candidate: &CandidateId,
) -> bool {
    rows.iter().any(|t| {
        text(t, "kind") == Some("transfer")
            && text(t, "task_id") == Some(task)
            && text(t, "dispatch_id") == Some(dispatch)
            && t.pointer("/to/base_commit").and_then(Value::as_str) == Some(&candidate.base_commit)
            && t.pointer("/to/dirty_digest").and_then(Value::as_str)
                == Some(&candidate.dirty_digest)
    })
}

/// Append the transfer of `run`'s evidence to `to`, once per dispatch and target (G9).
pub(crate) fn record_transfer(
    dir: &Path,
    task: &str,
    run: &Run,
    to: &CandidateId,
    tree: &str,
    now: u64,
) -> Result<(), String> {
    let Some(dispatch) = crate::adapter::cited_dispatch(task, &run.reference) else {
        return Ok(());
    };
    if run.transferred_to.contains(to) {
        return Ok(());
    }
    let id = |c: &CandidateId| json!({"base_commit":c.base_commit,"dirty_digest":c.dirty_digest});
    state::append_line(
        &dir.join("runs.jsonl"),
        &json!({"kind":"transfer","matched_by":"tree","task_id":task,"dispatch_id":dispatch,
            "from":id(&run.candidate),"to":id(to),"tree":tree,"t":now}),
    )
}

pub(crate) fn read_runs(dir: &Path, task: &str) -> Result<Vec<Run>, String> {
    let rows = records(dir)?;
    let mut recovered = std::collections::BTreeMap::new();
    for row in &rows {
        if let (Some(session), Some(agent), Some(dispatch)) = (
            text(row, "recovered_by_session"),
            text(row, "recovered_by_agent"),
            text(row, "dispatch_id"),
        ) && (text(row, "session_id") == Some(session)
            || crate::autonomy::recovers_session(
                dir,
                task,
                text(row, "session_id").unwrap_or_default(),
                session,
            )?)
        {
            recovered.insert(
                json!([row.get("session_id"), row.get("agent_id"), dispatch]).to_string(),
                agent.to_owned(),
            );
        }
    }
    Ok(rows
        .iter()
        .filter_map(|v| {
            if !approved_context(v)
                || text(v, "kind") != Some("completed")
                || !matches!(
                    text(v, "completion_trust"),
                    Some("observed" | "tc-reconstructed")
                )
                || text(v, "execution_project")
                    != text(v, "cwd")
                        .map(|p| crate::location::canonical_project(Path::new(p)))
                        .as_deref()
                || text(v, "task_id") != Some(task)
            {
                return None;
            }
            Some(Run {
                agent_id: text(v, "agent_id")?.to_owned(),
                command: text(v, "command")?.to_owned(),
                cwd: text(v, "cwd")?.to_owned(),
                execution_root: text(v, "execution_root")?.to_owned(),
                candidate: CandidateId {
                    base_commit: text(v.get("candidate")?, "base_commit")?.to_owned(),
                    dirty_digest: text(v.get("candidate")?, "dirty_digest")?.to_owned(),
                },
                tree: text(v, "tree").map(str::to_owned),
                transferred_to: transfers(&rows, task, v),
                exit_code: v
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .and_then(|n| i32::try_from(n).ok()),
                reference: format!(
                    "{}:{task}:{}",
                    text(v, "completion_trust")?,
                    text(v, "dispatch_id")?
                ),
                t: v.get("t")?.as_u64()?,
                finished: v.get("finished")?.as_u64()?,
                passed_tests: v.get("passed_tests").and_then(Value::as_u64),
                recovered_by: text(v, "dispatch_id")
                    .and_then(|id| {
                        recovered
                            .get(&json!([v.get("session_id"), v.get("agent_id"), id]).to_string())
                    })
                    .cloned(),
            })
        })
        .collect())
}

/// M6: passing tests in a test runner's summary lines: cargo (`test result: ok. N passed`, one
/// line per test binary, summed), jest (`Tests: ... N passed`), vitest (`Tests  N passed`) and
/// pytest (`== N passed ... ==`). `None` when no summary line names a passed count.
pub(crate) fn passed_tests(output: &str) -> Option<u64> {
    let mut total = None;
    for line in output.lines() {
        let line = line.trim();
        let summary = line.starts_with("test result:")
            || line.starts_with("Tests:")
            || line.starts_with("Tests ")
            || (line.starts_with('=') && line.contains(" passed"));
        if !summary {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(n) = words
            .windows(2)
            .find(|w| w[1].trim_end_matches([',', ';']) == "passed")
            .and_then(|w| w[0].parse::<u64>().ok())
        {
            *total.get_or_insert(0) += n;
        }
    }
    total
}

/// F20: a `|` the shell would read as a pipe, i.e. outside single and double quotes. A `|` inside
/// a quoted search pattern is text, and the command's exit code is its own. Bash and PowerShell
/// escape differently (`\` and a backtick), so a line with an escaped quote or a backtick keeps
/// the old rule: any `|` is a pipe.
fn unquoted_pipe(line: &str) -> bool {
    unquoted_pipe_at(line).is_some()
}

/// Byte offset of the first pipe `unquoted_pipe` recognises.
fn unquoted_pipe_at(line: &str) -> Option<usize> {
    if line.contains("\\'") || line.contains("\\\"") || line.contains('`') {
        return line.find('|');
    }
    let (mut single, mut double) = (false, false);
    for (at, c) in line.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '|' if !single && !double => return Some(at),
            _ => {}
        }
    }
    None
}

/// G19: what a shell line runs before its first pipe, when it has one.
pub(crate) fn before_pipe(line: &str) -> Option<&str> {
    unquoted_pipe_at(line).map(|at| line[..at].trim())
}

/// An argv element reaches a program literally, so its `|` pipes only when the program is a
/// shell that runs that element as a script.
fn shell_argv_pipe(args: &[Value]) -> bool {
    let mut words = args.iter().filter_map(Value::as_str);
    let program = words.next().unwrap_or_default();
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    matches!(
        name,
        "sh" | "bash" | "zsh" | "dash" | "pwsh" | "powershell" | "cmd"
    ) && words.any(unquoted_pipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (Dir, Value, Candidate) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = Dir(std::env::temp_dir().join(format!(
            "supervisor-observation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir_all(&dir.0).unwrap();
        let p = json!({"tool_name":format!("{TC}run_and_watch"),"tool_use_id":"dispatch-1",
            "session_id":"session-1","cwd":dir.0,"tool_input":{"argv":["cargo","test"],"cwd":dir.0}});
        let c = Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: "one".into(),
            },
            provenance: "test".into(),
            changed_paths: vec![],
        };
        (dir, p, c)
    }
    fn event(dir: &Dir, p: &Value, e: &str, c: &Candidate) -> bool {
        let mut p = p.clone();
        p["hook_event_name"] = e.into();
        observe(&dir.0, &p, "task-1", "main", Some(c), 100, &criteria()).unwrap()
    }
    fn criteria() -> Vec<Criterion> {
        vec![Criterion {
            id: coding_supervisor::CriterionId("C1".into()),
            check: coding_supervisor::CheckName("test".into()),
            mandatory: true,
            command: vec!["cargo test".into(), "echo \"a b\"".into()],
        }]
    }
    fn final_response() -> Value {
        json!({"job_id":"job-1","bucket_id":"bucket-1","state":"exited","complete":true,
            "exit_code":0,"outcome_trust":"observed","degraded":false,"pipeline_exit_masked":false})
    }
    fn completed(p: &Value, response: Value) -> Value {
        let mut p = p.clone();
        p["tool_response"] =
            json!({"content":[{"type":"text","text":response.to_string()}],"isError":false});
        p
    }

    #[test]
    fn tc_other_checkout_cannot_attest_hook_candidate() {
        let (d, mut p, c) = setup();
        let checkout = d.0.join("candidate");
        let other = d.0.join("other");
        for path in [&checkout, &other] {
            std::fs::create_dir_all(path).unwrap();
            assert!(
                std::process::Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        p["cwd"] = json!(checkout);
        p["tool_input"]["cwd"] = json!(other);
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = final_response();
        event(&d, &p, "PostToolUse", &c);
        assert!(
            read_runs(&d.0, "task-1").unwrap().is_empty(),
            "a passing command from another checkout must not attest this candidate"
        );
    }

    #[test]
    fn tc_async_completion_survives_hook_restart_and_is_idempotent() {
        let (d, mut p, c) = setup();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        // No in-memory state: every call reconstructs pending jobs from disk.
        let status = json!({"tool_name":format!("{TC}command_status"),"tool_use_id":"status-1",
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":final_response()});
        assert!(event(&d, &status, "PostToolUse", &c));
        assert!(!event(&d, &status, "PostToolUse", &c));
        let runs = read_runs(&d.0, "task-1").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].command, "cargo test");
        assert_eq!(runs[0].candidate, c.id);
        assert_eq!(runs[0].cwd, cwd(d.0.to_str().unwrap(), ""));
        assert!(read_runs(&d.0, "task-2").unwrap().is_empty());
        let mut recovered = status;
        recovered["tool_response"]["outcome_trust"] = "reconstructed".into();
        assert!(!event(&d, &recovered, "PostToolUse", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap().len(), 1);
    }

    #[test]
    fn tc_failed_state_records_the_exit_code() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        let mut failed = final_response();
        failed["state"] = "failed".into();
        failed["exit_code"] = 3.into();
        let status = json!({"tool_name":format!("{TC}command_status"),"tool_use_id":"status-1",
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":failed});
        // A TC job whose process failed must still be recorded as terminal
        // with its exit code, not left pending forever.
        assert!(event(&d, &status, "PostToolUse", &c));
        let runs = read_runs(&d.0, "task-1").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].command, "cargo test");
        assert_eq!(runs[0].exit_code, Some(3));
        let completed = records(&d.0)
            .unwrap()
            .into_iter()
            .filter(|r| r["kind"] == "completed")
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["exit_code"], 3);
    }

    #[test]
    fn tc_degraded_failed_state_is_not_recorded() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        let mut degraded = final_response();
        degraded["state"] = "failed".into();
        degraded["exit_code"] = 3.into();
        degraded["degraded"] = true.into();
        let status = json!({"tool_name":format!("{TC}command_status"),"tool_use_id":"status-1",
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":degraded});
        assert!(!event(&d, &status, "PostToolUse", &c));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }

    #[test]
    fn tc_failed_state_with_zero_exit_stays_pending() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        let mut failed = final_response();
        failed["state"] = "failed".into();
        failed["exit_code"] = 0.into();
        let status = json!({"tool_name":format!("{TC}command_status"),"tool_use_id":"status-1",
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":failed});
        // A failed status with a zero exit is contradictory daemon output; journaling
        // it would mint a passing receipt from a job the daemon says failed.
        assert!(!event(&d, &status, "PostToolUse", &c));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }

    #[test]
    fn tc_reconstructed_completion_closes_only_pending_durable_linkage() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        // TC 0.3.15 reconstructed_status reads terminal state, exit code and
        // process evidence from its durable receipt, not supervisor completion.
        let response = json!({"job_id":"job-1","bucket_id":"bucket-1","state":"exited",
            "exit_code":0,"outcome_trust":"reconstructed","restarted":true,
            "pipeline_exit_masked":false,"process_cleanup":{"state":"complete"},
            "process_observation":{"stdout":{"bytes_total":42,"state":{"state":"complete"}},
                "stderr":{"bytes_total":0,"state":{"state":"complete"}}}});
        let status = json!({"tool_name":format!("{TC}command_status"),"session_id":"session-1",
            "cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":response});
        assert!(event(&d, &status, "PostToolUse", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap().len(), 1);
        assert!(!event(&d, &status, "PostToolUse", &c));
    }

    #[test]
    fn tc_recovery_rejects_incomplete_or_mismatched_provenance() {
        for invalid in [
            "unknown-job",
            "wrong-bucket",
            "wrong-session",
            "wrong-task",
            "changed-candidate",
            "missing-stdout",
            "missing-cleanup",
            "abandoned",
            "lost",
            "remote",
            "masked",
        ] {
            let (d, mut p, c) = setup();
            event(&d, &p, "PreToolUse", &c);
            p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1",
                        "pipeline_exit_masked":invalid == "masked"});
            event(&d, &p, "PostToolUse", &c);
            let mut status = json!({"tool_name":format!("{TC}command_status"),
                        "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},
                        "tool_response":{"job_id":"job-1","bucket_id":"bucket-1","state":"exited",
                            "exit_code":0,"outcome_trust":"reconstructed","restarted":true,
                            "pipeline_exit_masked":false,"process_cleanup":{"state":"complete"},
                            "process_observation":{"stdout":{"state":{"state":"complete"}},
                                "stderr":{"state":{"state":"complete"}}}}});
            let mut current = c.clone();
            match invalid {
                "unknown-job" => status["tool_input"]["job_id"] = "job-other".into(),
                "wrong-bucket" => status["tool_response"]["bucket_id"] = "bucket-other".into(),
                "wrong-session" => status["session_id"] = "session-other".into(),
                "changed-candidate" => current.id.dirty_digest = "changed".into(),
                "missing-stdout" => {
                    status["tool_response"]["process_observation"]["stdout"] = Value::Null
                }
                "missing-cleanup" => status["tool_response"]["process_cleanup"] = Value::Null,
                "abandoned" => status["tool_response"]["outcome_trust"] = "abandoned".into(),
                "lost" => {
                    status["tool_response"] = json!({"isError":true,"error":{"code":"JobLost"}})
                }
                "remote" => status["tool_input"]["target_id"] = "remote".into(),
                _ => {}
            }
            status["hook_event_name"] = "PostToolUse".into();
            assert!(
                !observe(
                    &d.0,
                    &status,
                    if invalid == "wrong-task" {
                        "task-other"
                    } else {
                        "task-1"
                    },
                    "main",
                    Some(&current),
                    20,
                    &criteria()
                )
                .unwrap(),
                "{invalid}"
            );
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty(), "{invalid}");
        }
    }

    #[test]
    fn tc_nested_directory_keeps_physical_worktree_identity() {
        let (d, mut p, c) = setup();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&d.0)
                .status()
                .unwrap()
                .success()
        );
        let subdir = d.0.join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        p["tool_input"]["cwd"] = json!(subdir);
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = final_response();
        assert!(event(&d, &p, "PostToolUse", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap().len(), 1);
        // Turning the execution directory into a nested checkout changes its
        // physical project identity; old evidence must no longer be reusable.
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&subdir)
                .status()
                .unwrap()
                .success()
        );
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }

    #[test]
    fn tc_rejects_unproven_status_and_nested_fake_exit() {
        for response in [
            json!({"job_id":"job-1","stdout":"{\"exit_code\":0}"}),
            json!({"job_id":"job-1","receipt":{"exit_code":0}}),
            json!({"job_id":"job-1","signals":[{"summary":"exit_code:0"}]}),
        ] {
            let (d, p, c) = setup();
            event(&d, &p, "PreToolUse", &c);
            event(&d, &completed(&p, response), "PostToolUse", &c);
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        }
        for (key, value) in [
            ("outcome_trust", json!("reconstructed")),
            ("outcome_trust", json!("abandoned")),
            ("degraded", json!(true)),
            ("pipeline_exit_masked", json!(true)),
            ("exit_code", Value::Null),
            ("state", json!("running")),
            ("complete", json!(false)),
        ] {
            let (d, p, c) = setup();
            event(&d, &p, "PreToolUse", &c);
            let mut r = final_response();
            r[key] = value;
            event(&d, &completed(&p, r), "PostToolUse", &c);
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty(), "{key}");
        }
    }
    #[test]
    fn tc_wrong_job_session_target_and_unknown_bucket_cannot_complete() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        let status = json!({"tool_name":format!("{TC}command_status"),"tool_use_id":"status-1",
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":final_response()});
        for pointer in [
            "/tool_input/job_id",
            "/session_id",
            "/tool_response/job_id",
            "/tool_input/target_id",
        ] {
            let mut wrong = status.clone();
            if pointer == "/tool_input/target_id" {
                wrong["tool_input"]["target_id"] = "other".into();
            } else {
                *wrong.pointer_mut(pointer).unwrap() = "other".into();
            }
            assert!(!event(&d, &wrong, "PostToolUse", &c));
        }
        let bucket = json!({"tool_name":format!("{TC}bucket_wait"),"session_id":"session-1",
            "tool_input":{"bucket_id":"bucket-1"},"tool_response":{"signals":[{"exit_code":0}],"heartbeat":false}});
        assert!(!event(&d, &bucket, "PostToolUse", &c));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }
    #[test]
    fn tc_preserves_argv_boundaries_and_invalidates_inflight_candidate_change() {
        let (d, mut p, c) = setup();
        p["tool_input"]["argv"] = json!(["echo", "a b"]);
        event(&d, &p, "PreToolUse", &c);
        let mut changed = c.clone();
        changed.id.dirty_digest = "two".into();
        assert!(!event(
            &d,
            &completed(&p, final_response()),
            "PostToolUse",
            &changed
        ));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        assert!(!event(
            &d,
            &completed(&p, final_response()),
            "PostToolUse",
            &c
        ));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        let (_, other) = identity(
            &json!({"argv":["echo","a","b"]}),
            &format!("{TC}run_and_watch"),
        )
        .unwrap();
        let records = records(&d.0).unwrap();
        assert_ne!(records[0]["command"], other);
        assert_eq!(records[0]["identity"]["argv"], json!(["echo", "a b"]));
    }

    #[test]
    fn marked_stored_identity_must_match_tool_shape_and_command() {
        for invalid in [
            "wrong-command",
            "mixed-shape",
            "future-schema",
            "legacy-schema",
        ] {
            let (d, mut p, c) = setup();
            assert!(event(&d, &p, "PreToolUse", &c));
            p["tool_response"] = final_response();
            assert!(event(&d, &p, "PostToolUse", &c));
            let mut journal = records(&d.0).unwrap();
            for row in &mut journal {
                match invalid {
                    "wrong-command" => row["command"] = "echo harmless".into(),
                    "mixed-shape" => row["identity"]["shell_line"] = "cargo test".into(),
                    "future-schema" => row["identity_schema"] = 999.into(),
                    "legacy-schema" => row["identity_schema"] = 2.into(),
                    _ => unreachable!(),
                }
            }
            std::fs::write(
                d.0.join("runs.jsonl"),
                journal
                    .iter()
                    .map(|row| format!("{row}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty(), "{invalid}");
        }
    }

    #[test]
    fn unmarked_shell_line_completion_cannot_be_reused() {
        let (d, mut p, c) = setup();
        p["tool_name"] = "Bash".into();
        p["tool_input"] = json!({"command":"cargo test"});
        assert!(event(&d, &p, "PreToolUse", &c));
        assert!(event(&d, &p, "PostToolUse", &c));
        let mut journal = records(&d.0).unwrap();
        for row in &mut journal {
            row.as_object_mut().unwrap().remove("identity_schema");
        }
        std::fs::write(
            d.0.join("runs.jsonl"),
            journal
                .iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        assert!(
            read_runs(&d.0, "task-1").unwrap().is_empty(),
            "unmarked shell provenance cannot distinguish pre-fix shadow fields"
        );
    }

    #[test]
    fn tc_legacy_shell_exec_argv_job_cannot_finish() {
        let (d, mut p, c) = setup();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        let mut journal = records(&d.0).unwrap();
        // Exact pre-fix shell_exec representation when unknown argv shadowed
        // the shell_line that TC actually executed.
        for row in &mut journal {
            row["tool"] = format!("{TC}shell_exec").into();
        }
        std::fs::write(
            d.0.join("runs.jsonl"),
            journal
                .iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let status = json!({"tool_name":format!("{TC}command_status"),
            "session_id":"session-1","cwd":d.0,"tool_input":{"job_id":"job-1"},
            "tool_response":final_response()});
        assert!(!event(&d, &status, "PostToolUse", &c));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }

    #[test]
    fn tc_legacy_shell_exec_argv_completion_cannot_be_reused() {
        let (d, mut p, c) = setup();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = final_response();
        assert!(event(&d, &p, "PostToolUse", &c));
        let mut journal = records(&d.0).unwrap();
        for row in &mut journal {
            row["tool"] = format!("{TC}shell_exec").into();
        }
        std::fs::write(
            d.0.join("runs.jsonl"),
            journal
                .iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        assert!(
            read_runs(&d.0, "task-1").unwrap().is_empty(),
            "pre-fix completion cannot attest a shell_line from argv provenance"
        );
    }

    #[test]
    fn tc_shell_exec_shadow_argv_cannot_attest_approved_command() {
        let (d, mut p, c) = setup();
        p["tool_name"] = format!("{TC}shell_exec").into();
        p["tool_input"] = json!({"shell_line":"echo harmless","argv":["cargo","test"],"cwd":d.0});
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = final_response();
        event(&d, &p, "PostToolUse", &c);
        assert!(
            read_runs(&d.0, "task-1").unwrap().is_empty(),
            "TC executes shell_line, so shadow argv must not attest cargo test"
        );
        assert!(records(&d.0).unwrap().is_empty());
    }

    #[test]
    fn authoritative_tool_field_prevents_shadow_command_attestation() {
        for (tool, input) in [
            (format!("{TC}shell_exec"), json!({"command":"cargo test"})),
            (
                "Bash".into(),
                json!({"command":"echo harmless","shell_line":"cargo test"}),
            ),
            (
                "PowerShell".into(),
                json!({"command":"echo harmless","shell_line":"cargo test"}),
            ),
            (
                format!("{TC}command_start_combed"),
                json!({"shell_line":"cargo test"}),
            ),
            (
                format!("{TC}run_and_watch"),
                json!({"command":"cargo test"}),
            ),
        ] {
            let (d, mut p, c) = setup();
            p["tool_name"] = tool.clone().into();
            p["tool_input"] = input;
            p["tool_input"]["cwd"] = json!(d.0);
            event(&d, &p, "PreToolUse", &c);
            p["tool_response"] = final_response();
            event(&d, &p, "PostToolUse", &c);
            assert!(
                read_runs(&d.0, "task-1").unwrap().is_empty(),
                "{tool} must use its authoritative command field"
            );
            assert!(
                records(&d.0).unwrap().is_empty(),
                "{tool} must not persist an approved shadow command"
            );
        }
    }

    #[test]
    fn shell_exec_uses_shell_line_despite_unknown_argv_and_command_fields() {
        let (d, mut p, c) = setup();
        p["tool_name"] = format!("{TC}shell_exec").into();
        p["tool_input"] = json!({"shell_line":"cargo test","argv":["echo","harmless"],
            "command":"echo harmless","cwd":d.0});
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = final_response();
        assert!(event(&d, &p, "PostToolUse", &c));
        let runs = read_runs(&d.0, "task-1").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].command, "cargo test");
        assert_eq!(
            records(&d.0).unwrap()[0]["identity"]["shell_line"],
            "cargo test"
        );
    }

    #[test]
    fn tc_shell_line_and_masked_pipeline_are_distinct() {
        let (d, mut p, c) = setup();
        p["tool_name"] = format!("{TC}shell_exec").into();
        p["tool_input"] = json!({"shell_line":"cargo test","cwd":d.0});
        event(&d, &p, "PreToolUse", &c);
        assert!(event(
            &d,
            &completed(&p, final_response()),
            "PostToolUse",
            &c
        ));
        assert_eq!(read_runs(&d.0, "task-1").unwrap()[0].command, "cargo test");

        for trust in ["observed", "reconstructed"] {
            let (d, mut p, c) = setup();
            p["tool_name"] = format!("{TC}command_start_combed").into();
            p["tool_input"]["argv"] = json!(["pwsh", "-Command", "false | echo ok"]);
            let approved = vec![Criterion {
                id: coding_supervisor::CriterionId("C1".into()),
                check: coding_supervisor::CheckName("test".into()),
                mandatory: true,
                command: vec![r#"pwsh -Command "false | echo ok""#.into()],
            }];
            p["hook_event_name"] = "PreToolUse".into();
            assert!(observe(&d.0, &p, "task-1", "main", Some(&c), 100, &approved).unwrap());
            let dispatches = records(&d.0).unwrap();
            assert_eq!(dispatches.len(), 1);
            assert_eq!(dispatches[0]["kind"], "dispatch");
            assert_eq!(dispatches[0]["identity"]["argv"], p["tool_input"]["argv"]);

            p["hook_event_name"] = "PostToolUse".into();
            p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
            assert!(!observe(&d.0, &p, "task-1", "main", Some(&c), 101, &approved).unwrap());
            let jobs = records(&d.0).unwrap();
            assert_eq!(jobs.len(), 2);
            assert_eq!(jobs[1]["kind"], "job");
            assert_eq!(jobs[1]["pipeline_exit_masked"], true);

            // TC reconstruction hardcodes its mask to false. The adapter must
            // retain the pipeline's dispatch-side classification for both trusts.
            let status = json!({"hook_event_name":"PostToolUse",
                    "tool_name":format!("{TC}command_status"),"session_id":"session-1",
                    "cwd":d.0,"tool_input":{"job_id":"job-1"},
                    "tool_response":{"job_id":"job-1","bucket_id":"bucket-1","state":"exited",
                        "exit_code":0,"outcome_trust":trust,"restarted":trust == "reconstructed",
                        "pipeline_exit_masked":false,"process_cleanup":{"state":"complete"},
                        "process_observation":{"stdout":{"state":{"state":"complete"}},
                            "stderr":{"state":{"state":"complete"}}}}});
            assert!(
                !observe(&d.0, &status, "task-1", "main", Some(&c), 102, &approved).unwrap(),
                "{trust}"
            );
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty(), "{trust}");
        }
    }
    #[test]
    fn post_without_dispatch_and_unknown_cwd_never_create_evidence() {
        let (d, p, c) = setup();
        assert!(!event(
            &d,
            &completed(&p, final_response()),
            "PostToolUse",
            &c
        ));
        let mut p = p;
        p["tool_input"].as_object_mut().unwrap().remove("cwd");
        assert!(!event(&d, &p, "PreToolUse", &c));
    }
    #[test]
    fn explicit_remote_target_cannot_attest_a_local_candidate() {
        let (d, mut p, c) = setup();
        p["tool_input"]["target_id"] = "remote".into();
        assert!(!event(&d, &p, "PreToolUse", &c));
        assert!(!event(
            &d,
            &completed(&p, final_response()),
            "PostToolUse",
            &c
        ));
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }
    #[test]
    fn actual_status_without_degraded_requires_complete_process_observation() {
        let (d, mut p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        // Schema captured from TC command_status on this host, 2026-10-30.
        let mut response = json!({"job_id":"job-1","bucket_id":"bucket-1","state":"exited",
            "exit_code":0,"outcome_trust":"observed","pipeline_exit_masked":false,"restarted":false,
            "process_cleanup":{"state":"complete"},"process_observation":{
                "stdout":{"bytes_total":42,"state":{"state":"complete"}},
                "stderr":{"bytes_total":0,"state":{"state":"complete"}}}});
        let mut status = json!({"tool_name":format!("{TC}command_status"),"session_id":"session-1",
            "tool_input":{"job_id":"job-1"},"tool_response":response});
        status["tool_response"]["process_observation"]["stdout"]["state"]["state"] =
            "pending".into();
        assert!(!event(&d, &status, "PostToolUse", &c));
        response["degraded"] = true.into();
        status["tool_response"] = response.clone();
        assert!(!event(&d, &status, "PostToolUse", &c));
        response.as_object_mut().unwrap().remove("degraded");
        status["tool_response"] = response;
        assert!(event(&d, &status, "PostToolUse", &c));
    }
    #[test]
    fn claude_mcp_bare_content_array_links_async_job_and_completion() {
        // Claude Code 2.1.314's live tool_use_result is the content array itself,
        // rather than the MCP CallToolResult object used by earlier replays.
        let (d, mut p, c) = setup();
        p["tool_name"] = format!("{TC}command_start_combed").into();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = json!([{"type":"text","text":json!({"job_id":"job-1","bucket_id":"bucket-1","probe_id":"probe-1","cursor":0}).to_string()}]);
        event(&d, &p, "PostToolUse", &c);
        assert!(
            records(&d.0)
                .unwrap()
                .iter()
                .any(|v| text(v, "kind") == Some("job"))
        );
        let status = json!({"tool_name":format!("{TC}command_status"),"session_id":"session-1",
            "tool_input":{"job_id":"job-1"},"tool_response":[{"type":"text","text":final_response().to_string()}]});
        assert!(event(&d, &status, "PostToolUse", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap().len(), 1);
        let mut ambiguous = status;
        ambiguous["tool_response"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"text","text":"exit_code:0"}));
        assert!(response(&ambiguous["tool_response"]).is_none());
    }
    #[test]
    fn malformed_durable_journal_is_not_silently_skipped() {
        let (d, p, c) = setup();
        event(&d, &p, "PreToolUse", &c);
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(d.0.join("runs.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        assert!(read_runs(&d.0, "task-1").is_err());
    }
    #[test]
    fn unrelated_commands_are_never_persisted_or_linked() {
        let marker = "synthetic-command-privacy-marker";
        for tool in [
            "Bash",
            "PowerShell",
            "mcp__terminal_commander__run_and_watch",
            "mcp__terminal_commander__shell_exec",
        ] {
            let (d, mut p, c) = setup();
            p["tool_name"] = tool.into();
            p["tool_input"] = if tool.starts_with(TC) {
                if tool.ends_with("shell_exec") {
                    json!({"cwd":d.0,"shell_line":format!("echo {marker}")})
                } else {
                    json!({"cwd":d.0,"argv":["echo",marker]})
                }
            } else {
                json!({"command":format!("echo {marker}")})
            };
            assert!(!event(&d, &p, "PreToolUse", &c));
            assert!(!event(
                &d,
                &completed(&p, final_response()),
                "PostToolUse",
                &c
            ));
            assert!(!d.0.join("runs.jsonl").exists());
        }
    }
    /// M6: the passed count from cargo (summed over test binaries), jest, vitest and pytest.
    #[test]
    fn test_summaries_give_the_passed_count() {
        assert_eq!(
            passed_tests(
                "test result: ok. 3 passed; 0 failed
test result: ok. 4 passed; 0 failed"
            ),
            Some(7)
        );
        assert_eq!(
            passed_tests("Tests:       2 failed, 120 passed, 122 total"),
            Some(120)
        );
        assert_eq!(passed_tests(" Tests  12 passed (12)"), Some(12));
        assert_eq!(
            passed_tests("===== 9 passed, 1 skipped in 0.50s ====="),
            Some(9)
        );
        assert_eq!(passed_tests("all good"), None);
    }
    /// F20: a `|` inside quotes, or inside one argument of a program that is not a shell, is
    /// text, not a pipe; the run's exit code is still the command's own.
    #[test]
    fn a_quoted_pipe_is_not_a_pipeline() {
        let approved = |command: &str| {
            vec![Criterion {
                id: coding_supervisor::CriterionId("C7".into()),
                check: coding_supervisor::CheckName("no_disabled_specs".into()),
                mandatory: true,
                command: vec![command.into()],
            }]
        };
        let shell = |command: &str| {
            let (d, mut p, c) = setup();
            p["tool_name"] = "Bash".into();
            p["tool_input"] = json!({ "command": command });
            p["hook_event_name"] = "PreToolUse".into();
            assert!(
                observe(
                    &d.0,
                    &p,
                    "task-1",
                    "main",
                    Some(&c),
                    100,
                    &approved(command)
                )
                .unwrap()
            );
            p["hook_event_name"] = "PostToolUse".into();
            p["tool_response"] = json!({"stdout":"","interrupted":false});
            let recorded = observe(
                &d.0,
                &p,
                "task-1",
                "main",
                Some(&c),
                101,
                &approved(command),
            )
            .unwrap();
            (recorded, read_runs(&d.0, "task-1").unwrap())
        };
        let (recorded, runs) = shell(r#"rg -n 'xdescribe|fdescribe' src"#);
        assert!(recorded);
        assert_eq!(runs[0].exit_code, Some(0));
        let (recorded, runs) = shell(r#"rg -n "xit|fit" src"#);
        assert!(recorded);
        assert_eq!(runs.len(), 1);
        for piped in [
            "rg -n xit src | head",
            r#"rg -n "a\" | head"#,
            "rg -n `\"a | head",
        ] {
            let (recorded, runs) = shell(piped);
            assert!(!recorded, "{piped}");
            assert!(runs.is_empty(), "{piped}");
        }

        let (d, mut p, c) = setup();
        p["tool_input"]["argv"] = json!(["rg", "-n", "xit|fit", "src"]);
        let command = r#"rg -n "xit|fit" src"#;
        p["hook_event_name"] = "PreToolUse".into();
        assert!(
            observe(
                &d.0,
                &p,
                "task-1",
                "main",
                Some(&c),
                100,
                &approved(command)
            )
            .unwrap()
        );
        let mut post = completed(&p, final_response());
        post["hook_event_name"] = "PostToolUse".into();
        assert!(
            observe(
                &d.0,
                &post,
                "task-1",
                "main",
                Some(&c),
                101,
                &approved(command)
            )
            .unwrap()
        );
        assert_eq!(read_runs(&d.0, "task-1").unwrap()[0].exit_code, Some(0));
    }
    #[test]
    fn shell_background_launch_is_not_completion_evidence() {
        for async_response in [false, true] {
            let (d, mut p, c) = setup();
            p["tool_name"] = "Bash".into();
            p["tool_input"] = json!({"command":"cargo test","run_in_background":!async_response});
            event(&d, &p, "PreToolUse", &c);
            p["tool_response"] = if async_response {
                json!({"backgroundTaskId":"background-1"})
            } else {
                json!({"stdout":"started"})
            };
            assert!(!event(&d, &p, "PostToolUse", &c));
            assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
        }
    }
    #[test]
    fn shell_failure_output_and_interpreter_configuration_are_not_persisted() {
        let (d, mut p, c) = setup();
        let marker = "synthetic-shell-output-marker";
        p["tool_name"] = "PowerShell".into();
        p["tool_input"] = json!({"command":"cargo test"});
        assert!(event(&d, &p, "PreToolUse", &c));
        p["error"] = format!("Exit code 7\n{marker}").into();
        assert!(event(&d, &p, "PostToolUseFailure", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap()[0].exit_code, Some(7));
        p["tool_use_id"] = "unapproved-interpreter".into();
        p["tool_input"]["shell"] = marker.into();
        assert!(event(&d, &p, "PreToolUse", &c));
        assert!(!event(&d, &p, "PostToolUseFailure", &c));
        assert_eq!(read_runs(&d.0, "task-1").unwrap().len(), 1);
        assert!(
            !std::fs::read_to_string(d.0.join("runs.jsonl"))
                .unwrap()
                .contains(marker)
        );
    }

    #[test]
    fn new_explicit_owner_recovers_job_without_rebinding_original_dispatch() {
        let (d, mut p, c) = setup();
        p["hook_event_name"] = "PreToolUse".into();
        assert!(observe(&d.0, &p, "task-1", "worker-1", Some(&c), 100, &criteria()).unwrap());
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        p["hook_event_name"] = "PostToolUse".into();
        observe(&d.0, &p, "task-1", "worker-1", Some(&c), 100, &criteria()).unwrap();
        std::fs::write(
            d.0.join("task.json"),
            json!({"task_id":"task-1"}).to_string(),
        )
        .unwrap();
        std::fs::write(d.0.join("autonomy.json"), json!({"schema":1,"source":"hook","session_id":"session-2","paused":false,"revisions":[{"action":"start"},{"action":"resume","from_session":"session-1","to_session":"session-2"}]}).to_string()).unwrap();
        let status = json!({"tool_name":format!("{TC}command_status"),"session_id":"session-2","cwd":d.0,"tool_input":{"job_id":"job-1"},"tool_response":final_response()});
        let mut old = status.clone();
        old["session_id"] = "session-1".into();
        assert!(!event(&d, &old, "PostToolUse", &c));
        let mut forged = status.clone();
        forged["session_id"] = "sibling".into();
        assert!(!event(&d, &forged, "PostToolUse", &c));
        let mut wrong = status.clone();
        wrong["tool_input"]["job_id"] = "other-job".into();
        assert!(!event(&d, &wrong, "PostToolUse", &c));
        let other = d.0.join("other-worktree");
        std::fs::create_dir(&other).unwrap();
        let mut wrong_worktree = status.clone();
        wrong_worktree["cwd"] = json!(other);
        assert!(!event(&d, &wrong_worktree, "PostToolUse", &c));
        let mut unknown = status.clone();
        unknown["tool_response"]["outcome_trust"] = "unknown".into();
        assert!(!event(&d, &unknown, "PostToolUse", &c));
        assert!(event(&d, &status, "PostToolUse", &c));
        assert!(!event(&d, &status, "PostToolUse", &c));
        let rows = records(&d.0).unwrap();
        let completed = rows.iter().find(|r| r["kind"] == "completed").unwrap();
        assert_eq!(completed["session_id"], "session-1");
        assert_eq!(completed["agent_id"], "worker-1");
        assert_eq!(completed["recovered_by_session"], "session-2");
        assert_eq!(completed["recovered_by_agent"], "main");
        assert_eq!(read_runs(&d.0, "task-1").unwrap()[0].candidate, c.id);
        assert_eq!(
            read_runs(&d.0, "task-1").unwrap()[0]
                .recovered_by
                .as_deref(),
            Some("main")
        );
    }

    #[test]
    fn pending_jobs_expose_identifiers_and_criteria_without_output_and_keep_invalidated_jobs() {
        let (d, mut p, c) = setup();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] =
            json!({"job_id":"job-1","bucket_id":"bucket-1","stdout":"private-output-marker"});
        event(&d, &p, "PostToolUse", &c);
        std::fs::write(d.0.join("contract.json"), json!({"id":"fixture","goal":"fixture","criteria":[{"id":"C1","check":"test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        std::fs::write(d.0.join("autonomy.json"), json!({"schema":1,"source":"hook","session_id":"session-1","paused":false,"revisions":[{"action":"start"}],"invalidated_dispatches":["dispatch-1"]}).to_string()).unwrap();
        let jobs = crate::autonomy::pending_jobs(&d.0, "task-1").unwrap();
        assert_eq!(jobs.as_array().unwrap().len(), 1);
        assert_eq!(jobs[0]["job_id"], "job-1");
        assert_eq!(jobs[0]["dispatch_id"], "dispatch-1");
        assert_eq!(jobs[0]["session_id"], "session-1");
        assert_eq!(jobs[0]["validity"], "invalidated");
        assert_eq!(jobs[0]["criteria"], json!(["C1"]));
        assert!(!jobs.to_string().contains("private-output-marker"));
        assert_eq!(
            crate::autonomy::pending_jobs(&d.0, "other-task").unwrap(),
            json!([])
        );
    }

    /// G13: a job stopped with `command_stop` is no longer pending, and stopping it records no
    /// completion. Another session cannot stop-mark it.
    #[test]
    fn a_stopped_tc_job_is_no_longer_pending_and_counts_as_no_completion() {
        let (d, mut p, c) = setup();
        assert!(event(&d, &p, "PreToolUse", &c));
        p["tool_response"] = json!({"job_id":"job-1","bucket_id":"bucket-1"});
        event(&d, &p, "PostToolUse", &c);
        std::fs::write(d.0.join("contract.json"), json!({"id":"fixture","goal":"fixture","criteria":[{"id":"C1","check":"test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let stop = |session: &str| {
            json!({"tool_name":format!("{TC}command_stop"),"tool_use_id":format!("stop-{session}"),
                "session_id":session,"cwd":d.0,"tool_input":{"job_id":"job-1"},
                "tool_response":{"job_id":"job-1","bucket_id":"bucket-1","bytes_total":10,"events_emitted":0,"frames_total":1}})
        };
        assert!(!event(&d, &stop("session-2"), "PostToolUse", &c));
        assert_eq!(
            crate::autonomy::pending_jobs(&d.0, "task-1")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(event(&d, &stop("session-1"), "PostToolUse", &c));
        assert_eq!(
            crate::autonomy::pending_jobs(&d.0, "task-1").unwrap(),
            json!([])
        );
        assert!(read_runs(&d.0, "task-1").unwrap().is_empty());
    }
}
