//! One explicitly shared active task per worktree. Rollover never rewrites prior task files.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::state::{self, Stored};
use crate::transaction::{Transaction, atomic_write, copy_task, task_writes};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct ActiveTask {
    pub task_id: String,
    pub dir: PathBuf,
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| format!("{}: invalid JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub(crate) fn safe_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("invalid task ID".to_owned());
    }
    Ok(())
}

/// Read after taking `Transaction` when a consistent snapshot is needed.
pub fn active_task(root: &Path) -> Result<Option<ActiveTask>, String> {
    let Some(pointer) = read_json(&root.join("active.json"))? else {
        return Ok(None);
    };
    let Some(id) = pointer.get("task_id").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let id = id.as_str().ok_or("invalid active task pointer")?;
    safe_id(id)?;
    let dir = root.join("tasks").join(id);
    let metadata = read_json(&dir.join("task.json"))?.ok_or("active task metadata missing")?;
    if metadata.get("task_id").and_then(Value::as_str) != Some(id) {
        return Err("active task identity mismatch".to_owned());
    }
    Ok(Some(ActiveTask {
        task_id: id.to_owned(),
        dir,
    }))
}

pub fn task_identity(dir: &Path, contract_id: &str) -> Result<String, String> {
    match read_json(&dir.join("task.json"))? {
        Some(metadata) => metadata
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "task metadata has no identity".to_owned()),
        None => Ok(contract_id.to_owned()),
    }
}

fn unique_id() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "task-{t:x}-{:x}-{:x}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn state_of(dir: &Path) -> Result<&'static str, String> {
    let (_, digest) = state::read_contract(dir)?;
    match state::load_gate(dir, &digest)? {
        Stored::Valid(gate) => Ok(state::state_name(gate.checkpoint.state)),
        Stored::Absent => Ok("queued"),
        Stored::Tampered(what) => Err(format!(
            "gate state tampered ({what}); restore authentic evidence before lifecycle changes"
        )),
    }
}

fn disposition(dir: &Path, requested: Option<&str>) -> Result<String, String> {
    let state = state_of(dir)?;
    if state == "completed" {
        return Ok("completed".to_owned());
    }
    let requested = requested.ok_or_else(|| format!("task is {state}; explicit disposition required: cancelled, failed, blocked or budget_exhausted"))?;
    if !matches!(
        requested,
        "cancelled" | "failed" | "blocked" | "budget_exhausted"
    ) {
        return Err("invalid unfinished-task disposition".to_owned());
    }
    Ok(requested.to_owned())
}

fn legacy_pending(root: &Path) -> Result<(), String> {
    if root.join("active.json").exists() {
        return Ok(());
    }
    if root.join("legacy-location.json").exists() || root.join("contract.json").exists() {
        return Err("legacy task state requires explicit task migrate --legacy <directory> before activation".to_owned());
    }
    Ok(())
}

/// Start or replace an active task. The contract ID remains caller-owned; task_id is generated.
/// `writer` is recorded in the stored contract's approval and supplies its base_sha.
pub fn start_as(
    root: &Path,
    writer: &state::Writer,
    contract: &Value,
    next: bool,
    requested: Option<&str>,
    expected: Option<&str>,
) -> Result<Value, String> {
    start_with(root, writer, contract, next, requested, expected, None)
}

/// WP3: [`start_as`] for a person's `task start --worktree <path>`: creates a git worktree at the
/// absolute `worktree` (which must not exist yet) on branch `supervisor/<task id>` at the
/// contract's base commit, checks it, and binds the task to it with `isolation: worktree`.
pub fn start_in_worktree(
    root: &Path,
    writer: &state::Writer,
    contract: &Value,
    next: bool,
    requested: Option<&str>,
    expected: Option<&str>,
    worktree: &Path,
) -> Result<Value, String> {
    start_with(
        root,
        writer,
        contract,
        next,
        requested,
        expected,
        Some(worktree),
    )
}

fn start_with(
    root: &Path,
    writer: &state::Writer,
    contract: &Value,
    next: bool,
    requested: Option<&str>,
    expected: Option<&str>,
    worktree: Option<&Path>,
) -> Result<Value, String> {
    // A person's contract may start a task after prompts were registered (K13); the guard keeps
    // agents from running this command, so they still go through a floor-checked proposal.
    let tx = Transaction::open(root)?;
    if let Some(active) = active_task(root)?
        && crate::autonomy::is_autonomous_task(&active.dir)?
    {
        return Err(format!(
            "autonomous tasks require a prompt-bound cancellation proposal; a person can run `supervisor task cancel --project <dir> --task {} --reason <text>`",
            active.task_id
        ));
    }
    let (response, writes) =
        plan_start(root, writer, contract, next, requested, expected, worktree)?;
    // The worktree is created under the state lock, so no other start can bind the same path; a
    // replayed start wrote nothing and creates nothing.
    if let (Some(path), Some(project), Some(branch), Some(base)) = (
        worktree,
        writer.project(),
        response["branch"].as_str(),
        response["base_sha"].as_str(),
    ) {
        let stored = response["worktree"].as_str().unwrap_or_default();
        crate::worktree::create(project, path, stored, branch, base)?;
        return match tx.commit(writes) {
            Ok(()) => Ok(response),
            // A journal on disk finishes the start at the next lock, so its worktree must stay.
            Err(why) if !matches!(tx.root().join("transaction.json").try_exists(), Ok(false)) => {
                Err(format!(
                    "{why}; the start is recorded but not finished: the next supervisor command (for example `supervisor status`) finishes it, or reports why it cannot, so the new worktree {} on branch {branch} is kept for it",
                    path.display()
                ))
            }
            Err(why) => Err(crate::worktree::rollback(project, path, branch, &why)),
        };
    }
    tx.commit(writes)?;
    Ok(response)
}

/// Caller holds the project transaction lock; allows prompt consumption and activation to commit together.
pub(crate) fn plan_start(
    root: &Path,
    writer: &state::Writer,
    contract: &Value,
    next: bool,
    requested: Option<&str>,
    expected: Option<&str>,
    worktree: Option<&Path>,
) -> Result<(Value, Vec<(PathBuf, String)>), String> {
    // The caller's own contract is checked first, so a newer schema is refused rather than
    // stamped over. What is saved is checked too: a v1-shaped request is stored as v2, so its
    // v2 fields must load as v2 now rather than fail every later read. The request digests
    // below stay over the caller's contract, so a replay still matches.
    state::contract_from(contract)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut stored = state::stamp_v2(contract, writer, now);
    let parsed = state::contract_from(&stored)?;
    legacy_pending(root)?;
    let previous = active_task(root)?;
    if let Some(previous) = &previous {
        state_of(&previous.dir)?;
    }
    if !next && let Some(event_id) = contract.get("event_id").and_then(Value::as_str) {
        let digest = crate::sha256::hex(contract.to_string().as_bytes());
        let tasks = root.join("tasks");
        let mut replay: Option<(ActiveTask, String, Option<String>)> = None;
        match fs::read_dir(&tasks) {
            Ok(entries) => {
                for entry in entries {
                    let dir = entry.map_err(|e| e.to_string())?.path();
                    let Some(metadata) = read_json(&dir.join("task.json"))? else {
                        continue;
                    };
                    if metadata.get("start_event_id").and_then(Value::as_str) != Some(event_id) {
                        continue;
                    }
                    if metadata.get("start_request_digest").and_then(Value::as_str)
                        != Some(digest.as_str())
                    {
                        return Err("task-start event ID reused with different input".to_owned());
                    }
                    if replay.is_some() {
                        return Err("task-start event ID is ambiguous in task history".to_owned());
                    }
                    let task_id = metadata
                        .get("task_id")
                        .and_then(Value::as_str)
                        .ok_or("task metadata has no identity")?
                        .to_owned();
                    let contract_id = metadata
                        .get("contract_id")
                        .and_then(Value::as_str)
                        .ok_or("task metadata has no contract ID")?
                        .to_owned();
                    let earlier = metadata["worktree"].as_str().map(str::to_owned);
                    replay = Some((ActiveTask { task_id, dir }, contract_id, earlier));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        if let Some((previous, contract_id, earlier)) = replay {
            // WP3: a replay must not drop a requested worktree silently.
            if let Some(path) = worktree
                && earlier.as_deref() != Some(crate::location::normalize(path).as_str())
            {
                return Err(format!(
                    "task-start event ID reused with a different worktree setting: the earlier start had {}; nothing was created",
                    earlier.map_or("no worktree".to_owned(), |w| format!("the worktree {w}"))
                ));
            }
            let mut response = json!({"schema":2,"task_id":previous.task_id,"contract_id":contract_id,"state":state_of(&previous.dir)?,"replayed":true});
            // A replay without `--worktree` still names the earlier start's worktree.
            if let Some(earlier) = earlier {
                response["worktree"] = earlier.into();
            }
            return Ok((response, Vec::new()));
        }
    }
    if previous.is_some() && !next {
        return Err("an active task already exists; use task next or task resume".to_owned());
    }
    if next && previous.is_none() {
        return Err("no active task; use task start".to_owned());
    }
    if let Some(previous) = &previous
        && expected != Some(previous.task_id.as_str())
    {
        return Err("task next requires the current --task ID".to_owned());
    }
    let id = unique_id();
    // WP3: the only way a task gets `isolation: worktree`; the worktree itself is created by
    // `start_with` once these checks pass.
    let bound = match worktree {
        None => None,
        Some(path) => {
            let project = writer
                .project()
                .ok_or("--worktree needs the project the task starts in")?;
            let base = stored["base_sha"]
                .as_str()
                .ok_or("--worktree needs a commit to start from: the repository has no HEAD")?
                .to_owned();
            let path = crate::worktree::check_request(project, path)?;
            stored["isolation"] = "worktree".into();
            state::contract_from(&stored)?;
            Some((path, base))
        }
    };
    let mut writes = Vec::new();
    let mut recovery_of = Value::Null;
    if let Some(previous) = &previous {
        let outcome = disposition(&previous.dir, requested)?;
        recovery_of = previous.task_id.clone().into();
        writes.push((
            PathBuf::from("archives").join(format!("{}.json", previous.task_id)),
            format!(
                "{}\n",
                json!({"schema":2,"task_id":previous.task_id,"disposition":outcome,"successor":id})
            ),
        ));
    }
    let prefix = PathBuf::from("tasks").join(&id);
    writes.push((prefix.join("contract.json"), format!("{stored}\n")));
    // G48: what is already changed in the project is not this task's edit.
    if let Some(baseline) = writer.project().and_then(crate::candidate::baseline) {
        writes.push((prefix.join(crate::candidate::BASELINE), baseline));
    }
    let mut metadata = json!({"schema":2,"task_id":id,"contract_id":parsed.id.0,"recovery_of":recovery_of,"start_event_id":contract.get("event_id"),"start_request_digest":crate::sha256::hex(contract.to_string().as_bytes())});
    // WP3: the binding `binding::bound_root` reads after autonomy.json, and the checkout whose
    // edits are refused while it holds (never read back from the worktree's own `.git`).
    if let (Some((path, _)), Some(project)) = (&bound, writer.project()) {
        metadata["worktree"] = path.as_str().into();
        metadata["base_checkout"] = crate::location::canonical_project(project).into();
    }
    writes.push((prefix.join("task.json"), format!("{metadata}\n")));
    writes.push((
        PathBuf::from("active.json"),
        format!("{}\n", json!({"schema":2,"task_id":id})),
    ));
    let mut response = json!({"schema":2,"task_id":id,"contract_id":parsed.id.0,"state":"queued","recovery_of":recovery_of});
    if let Some((path, base)) = &bound {
        response["worktree"] = path.as_str().into();
        response["branch"] = crate::worktree::branch(&id).into();
        response["base_sha"] = base.as_str().into();
        response["note"] = "work and run the checks in the worktree: edits in the main checkout are refused while this task is bound to it; a check counts when it runs with the worktree as its directory (a Terminal Commander run with cwd set to it, or a shell whose session works in it); start from an up-to-date main, since CI compares the receipt with the merge base".into();
    }
    let mut warnings = Vec::new();
    if !parsed.forbidden_effects.is_empty() {
        // F13b: only typed entries in strict mode become refusals (M5); a reviewer checks the rest.
        warnings.push("forbidden_effects are recorded but not enforced, except typed entries (deny_path:, deny_git:, deny_argv_prefix:, deny_git_push_target:, deny_git_force_push) in strict mode; a reviewer must check the rest. Pushes to main, master or origin/HEAD, force-pushes and npm publish are refused by default from enforce up".to_owned());
    }
    let replaced: Vec<&str> = ["isolation", "approval", "base_sha"]
        .into_iter()
        .filter(|key| {
            contract
                .get(*key)
                .is_some_and(|v| Some(v) != stored.get(*key))
        })
        .collect();
    if !replaced.is_empty() {
        let isolation = if bound.is_some() {
            "isolation worktree (set by --worktree)"
        } else {
            "isolation none (only `supervisor task start --worktree` sets isolation worktree)"
        };
        warnings.push(format!(
            "{} replaced by the writer's own record: the gate stores who started the task, the repository head and {isolation}",
            replaced.join(", ")
        ));
    }
    if !warnings.is_empty() {
        response["warnings"] = warnings.into();
    }
    Ok((response, writes))
}

pub fn deactivate(root: &Path, expected: &str, requested: Option<&str>) -> Result<Value, String> {
    let tx = Transaction::open(root)?;
    let active = active_task(root)?.ok_or("no active task")?;
    if crate::autonomy::is_autonomous_task(&active.dir)? {
        return Err("autonomous tasks require a prompt-bound cancellation proposal; a person can run `supervisor task cancel --task <id> --reason <text>`".to_owned());
    }
    if active.task_id != expected {
        return Err("active task changed".to_owned());
    }
    let outcome = disposition(&active.dir, requested)?;
    tx.commit(vec![
        (
            PathBuf::from("archives").join(format!("{}.json", active.task_id)),
            format!(
                "{}\n",
                json!({"schema":2,"task_id":active.task_id,"disposition":outcome,"successor":null})
            ),
        ),
        (
            PathBuf::from("active.json"),
            "{\"schema\":2,\"task_id\":null}\n".to_owned(),
        ),
    ])?;
    Ok(json!({"inactive":true,"archived_task_id":active.task_id,"disposition":outcome}))
}

/// Resume is an assertion of identity, never a reset of repairs or evidence.
pub fn resume(root: &Path, expected: &str) -> Result<Value, String> {
    let _tx = Transaction::open(root)?;
    let active = active_task(root)?.ok_or("no active task")?;
    if active.task_id != expected {
        return Err(
            "only the current active task may resume; archived tasks are immutable".to_owned(),
        );
    }
    Ok(json!({"task_id":active.task_id,"state":state_of(&active.dir)?,"resumed":true}))
}

/// A person's escape from `budget_exhausted`: back to `repairing` with no rounds spent,
/// journaled with the reason. The PreToolUse guard refuses this command to agents.
pub fn reset_rounds(root: &Path, expected: &str, reason: &str) -> Result<Value, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("reset-rounds needs a nonempty --reason".to_owned());
    }
    let tx = Transaction::open(root)?;
    let active = active_task(root)?.ok_or("no active task")?;
    if active.task_id != expected {
        return Err("active task changed".to_owned());
    }
    let (contract, digest) = state::read_contract(&active.dir)?;
    let mut gate = match state::load_gate(&active.dir, &digest)? {
        Stored::Valid(gate) => gate,
        Stored::Absent => {
            return Err(
                "task is queued; only a budget_exhausted task can have its rounds reset".to_owned(),
            );
        }
        Stored::Tampered(what) => return Err(format!("gate state tampered ({what})")),
    };
    let before = state::state_name(gate.checkpoint.state);
    if before != "budget_exhausted" {
        return Err(format!(
            "task is {before}; only a budget_exhausted task can have its rounds reset"
        ));
    }
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // WP4: a reached task limit is recomputed at every hook call, so reopening the rounds would
    // not reopen the task; a new contract does.
    if let Some(contract) = &contract
        && let Some(reached) =
            crate::limits::usage(&active.dir, contract, t)?.and_then(|usage| usage.reached)
    {
        return Err(format!(
            "task limit reached: {}; reset-rounds does not lift a task limit; for new work, {}",
            crate::limits::reached_text(&reached),
            crate::limits::way_out(&active.dir, &active.task_id)?
        ));
    }
    let rounds_before = gate.checkpoint.consumed_repair_rounds;
    gate.checkpoint.state = coding_supervisor::TaskState::Repairing;
    gate.checkpoint.consumed_repair_rounds = 0;
    gate.continuations = 0;
    let line = json!({"t":t,"event":"reset_rounds","task_id":active.task_id,"reason":reason,"rounds_before":rounds_before,"state_before":before,"state":"repairing"});
    let prefix = PathBuf::from("tasks").join(&active.task_id);
    let mut journal = match fs::read_to_string(active.dir.join("events.jsonl")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    journal.push_str(&format!("{line}\n"));
    tx.commit(vec![
        (prefix.join("gate.json"), state::gate_text(&gate, &digest)),
        (prefix.join("events.jsonl"), journal),
    ])?;
    Ok(json!({"task_id":active.task_id,"state":"repairing","rounds_before":rounds_before}))
}

/// G58 review: a new prompt from the session that owns the active task starts a new turn, so the
/// last turn's pending hold releases nothing. Writes only when the mark is set on a valid gate.
pub fn clear_pending_hold(root: &Path, session: &str) -> Result<(), String> {
    let tx = Transaction::open(root)?;
    let Some(active) = active_task(root)? else {
        return Ok(());
    };
    if !crate::autonomy::owns_session(&active.dir, session)? {
        return Ok(());
    }
    let (_, digest) = state::read_contract(&active.dir)?;
    let Stored::Valid(mut gate) = state::load_gate(&active.dir, &digest)? else {
        return Ok(());
    };
    if !gate.pending_hold {
        return Ok(());
    }
    gate.pending_hold = false;
    let path = PathBuf::from("tasks")
        .join(&active.task_id)
        .join("gate.json");
    tx.commit(vec![(path, state::gate_text(&gate, &digest))])
}

pub fn rebind(root: &Path, expected: &str, session: &str) -> Result<Value, String> {
    if session.is_empty() {
        return Err("session ID is empty".to_owned());
    }
    let tx = Transaction::open(root)?;
    let active = active_task(root)?.ok_or("no active task")?;
    if crate::autonomy::is_autonomous_task(&active.dir)? {
        return Err("autonomous session bindings are controlled by prompt proposals".to_owned());
    }
    if active.task_id != expected {
        return Err("active task changed".to_owned());
    }
    let mut sessions = read_json(&root.join("sessions.json"))?.unwrap_or_else(|| json!({}));
    sessions
        .as_object_mut()
        .ok_or("invalid session bindings")?
        .insert(session.to_owned(), expected.into());
    tx.commit(vec![("sessions.json".into(), format!("{sessions}\n"))])?;
    Ok(json!({"task_id":expected,"session_id":session,"rebound":true}))
}

/// Import exactly the explicitly selected legacy directory. Its files are copied byte-for-byte.
pub fn migrate(root: &Path, legacy: &Path) -> Result<Value, String> {
    let tx = Transaction::open(root)?;
    if root.join("active.json").exists() {
        return Err("task layout already initialized".to_owned());
    }
    let (contract, digest) = state::read_contract(legacy)?;
    let contract = contract.ok_or("selected legacy directory has no contract")?;
    if matches!(state::load_gate(legacy, &digest)?, Stored::Tampered(_)) {
        return Err("selected legacy state is tampered".to_owned());
    }
    let id = unique_id();
    let stage = root.join("migration-stage");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    copy_task(legacy, &stage)?;
    atomic_write(
        &stage.join("task.json"),
        format!(
            "{}\n",
            json!({"schema":2,"task_id":id,"contract_id":contract.id.0,"legacy_source":legacy})
        )
        .as_bytes(),
    )?;
    let mut writes = task_writes(&stage, &PathBuf::from("tasks").join(&id))?;
    writes.push((
        "active.json".into(),
        format!("{}\n", json!({"schema":2,"task_id":id})),
    ));
    tx.commit(writes)?;
    fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    Ok(json!({"task_id":id,"contract_id":contract.id.0,"migrated":true,"legacy_preserved":true}))
}

/// Legacy task state that a person must move with `task migrate --legacy` before hooks use it.
pub fn legacy_migration_pending(root: &Path) -> bool {
    root.join("legacy-location.json").exists() && !root.join("active.json").exists()
}

/// Mutate a task under one OS lock and one recoverable commit, including every task journal.
/// Cached responses make replayed event IDs idempotent. A changed payload under one ID is refused.
pub fn with_task(
    root: &Path,
    expected: Option<&str>,
    session: Option<&str>,
    event: Option<(&str, &str)>,
    action: impl FnOnce(&Path) -> Result<Value, String>,
) -> Result<Value, String> {
    let tx = Transaction::open(root)?;
    let active = active_task(root)?;
    if active.is_none() && legacy_migration_pending(root) {
        return Err(
            "legacy task state found; explicitly task migrate --legacy <directory> before use"
                .to_owned(),
        );
    }
    let (source, prefix) = match &active {
        Some(active) => (
            active.dir.clone(),
            PathBuf::from("tasks").join(&active.task_id),
        ),
        None if root.join("active.json").exists() => {
            (root.join("inactive"), PathBuf::from("inactive"))
        }
        None => (root.to_path_buf(), PathBuf::new()), // v1 fixture/import compatibility; never adopt another location.
    };
    let mut sessions = read_json(&root.join("sessions.json"))?.unwrap_or_else(|| json!({}));
    if let Some(active) = &active {
        if let Some(expected) = expected {
            if expected != active.task_id {
                return Err("stale task ID; active task changed".to_owned());
            }
        } else if session.is_none() {
            return Err("task_id is required for task mutation".to_owned());
        }
        if let Some(session) = session {
            let bindings = sessions.as_object_mut().ok_or("invalid session bindings")?;
            match bindings.get(session).and_then(Value::as_str) {
                Some(bound) if bound != active.task_id => {
                    let project = read_json(&root.join("project.json"))
                        .ok()
                        .flatten()
                        .and_then(|v| v["canonical_worktree"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "<project>".to_owned());
                    // G56: `rebind` refuses a prompt-managed task; a resume of the task's own
                    // source binds it (a hook prompt cannot resume a caller-asserted task).
                    if crate::autonomy::is_autonomous_task(&active.dir)? {
                        let id = &active.task_id;
                        return Err(
                            if crate::autonomy::requires_observed_completion(&active.dir)? {
                                format!(
                                    "session belongs to an earlier task; the active task {id} is prompt-managed, so the user's direct prompt to resume it (intent_apply action resume for task {id}) binds this session to it"
                                )
                            } else {
                                format!(
                                    "session belongs to an earlier task; the active task {id} is prompt-managed through caller-asserted intent, which a hook-observed prompt cannot resume: in a session whose prompts the gate's hooks do not record, a caller-asserted resume (intent_record with the user's resume prompt, then intent_apply action resume for task {id}) binds that session to it; otherwise a person ends the task with `supervisor task cancel --project \"{project}\" --task {id} --reason \"<why>\"`"
                                )
                            },
                        );
                    }
                    return Err(format!(
                        "session belongs to an earlier task; a person runs `supervisor task rebind --project \"{project}\" --task {} --session {session}` from their own terminal before this session continues",
                        active.task_id
                    ));
                }
                Some(_) => {}
                None => {
                    bindings.insert(session.to_owned(), active.task_id.clone().into());
                }
            }
        }
    }
    let mut events = read_json(&source.join("applied-events.json"))?.unwrap_or_else(|| json!({}));
    if let Some((id, digest)) = event
        && let Some(cached) = events.get(id)
    {
        let (_, contract_digest) = state::read_contract(&source)?;
        if let Stored::Tampered(what) = state::load_gate(&source, &contract_digest)? {
            return Err(format!(
                "task state tampered ({what}); cached response refused"
            ));
        }
        if cached.get("digest").and_then(Value::as_str) != Some(digest) {
            return Err("event ID reused with different input".to_owned());
        }
        if cached.get("state_digest").and_then(Value::as_str)
            != Some(task_digest(root, &source)?.as_str())
        {
            return Err("task state changed after event; cached response refused".to_owned());
        }
        return cached
            .get("response")
            .cloned()
            .ok_or_else(|| "invalid cached event response".to_owned());
    }
    let stage = root.join("task-stage");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    copy_task(&source, &stage)?;
    let response = action(&stage)?;
    if let Some((id, digest)) = event {
        events.as_object_mut().ok_or("invalid event index")?.insert(
            id.to_owned(),
            json!({"digest":digest,"state_digest":task_digest(root, &stage)?,"response":response}),
        );
        atomic_write(
            &stage.join("applied-events.json"),
            format!("{events}\n").as_bytes(),
        )?;
    }
    let mut writes = task_writes(&stage, &prefix)?;
    if active.is_some() {
        writes.push(("sessions.json".into(), format!("{sessions}\n")));
    }
    tx.commit(writes)?;
    fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    Ok(response)
}

fn task_digest(root: &Path, dir: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    for (base, name) in [
        (dir, "contract.json"),
        (dir, "gate.json"),
        (dir, "runs.jsonl"),
        (dir, "mcp-candidate.json"),
        (dir, "autonomy.json"),
        (root, "intents.json"),
    ] {
        bytes.extend_from_slice(name.as_bytes());
        match fs::read(base.join(name)) {
            Ok(content) => bytes.extend_from_slice(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(crate::sha256::hex(&bytes))
}

pub fn observe_connection(root: &Path, source: &str) -> Result<(), String> {
    let tx = Transaction::open(root)?;
    let mut observations = read_json(&root.join("connection.json"))?.unwrap_or_else(|| json!({}));
    observations
        .as_object_mut()
        .ok_or("invalid connection observations")?
        .insert(
            source.to_owned(),
            json!({"observed_at":crate::adapter::now(),"source":source,"session_load":"unproven"}),
        );
    tx.commit(vec![(
        "connection.json".into(),
        format!("{observations}\n"),
    )])
}
