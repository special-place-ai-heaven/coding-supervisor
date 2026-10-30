//! Prompt-bound lifecycle proposals. The host records intent; a model proposes, never approves.
//!
//! Hashes identify prompts but do not prove the semantic adequacy of a proposed goal or scope.
//! Hook intent is trusted only on the installed hook path. Caller-asserted MCP intent remains
//! advisory and cannot control a task established by that path. These are same-user policy
//! checks, not protection against a shell impersonating the harness or editing state files.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use coding_supervisor::TaskState;
use serde_json::{Value, json};

use crate::lifecycle;
use crate::state::{self, Stored};
use crate::transaction::{Transaction, atomic_write, copy_task, task_writes};

static NONCE: AtomicU64 = AtomicU64::new(0);

fn read(path: &Path) -> Result<Option<Value>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| format!("{}: invalid JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{key} must be a nonempty string"))
}

fn project_identity(project: &Path) -> Result<String, String> {
    fs::canonicalize(project)
        .map(|_| crate::location::canonical_project(project))
        .map_err(|e| format!("project identity unavailable: {e}"))
}

fn same_project(recorded: &Value, project: &Path) -> Result<bool, String> {
    let identity = project_identity(project)?;
    // Schema 1 originally stored raw canonical Windows paths, including nested cwd.
    Ok(recorded.as_str().is_some_and(|old| {
        old == identity || project_identity(Path::new(old)).as_deref() == Ok(identity.as_str())
    }))
}

fn legacy_subdirectory_context(recorded: &Value, project: &Path) -> Result<bool, String> {
    let identity = project_identity(project)?;
    let Some(recorded) = recorded.as_str() else {
        return Ok(false);
    };
    let captured =
        fs::canonicalize(recorded).map_err(|_| "recorded discovery directory unavailable")?;
    let root = fs::canonicalize(&identity).map_err(|_| "project root unavailable")?;
    Ok(captured != root && crate::location::canonical_project(&captured) == identity)
}

fn validate_discovery_context(recorded: &Value, project: &Path) -> Result<(), String> {
    if !same_project(recorded, project)? {
        return Err("discovery snapshot belongs to another physical worktree".into());
    }
    if legacy_subdirectory_context(recorded, project)? {
        return Err("legacy subdirectory discovery cannot certify project-root checks; submit a fresh root prompt and explicitly cancel the old task before restarting; unsupported repositories need an explicitly approved root contract".into());
    }
    Ok(())
}

fn load(root: &Path) -> Result<Value, String> {
    let store = read(&root.join("intents.json"))?
        .unwrap_or_else(|| json!({"schema":1,"events":{},"latest":{}}));
    if store["schema"] != 1
        || !store["events"].is_object()
        || !store["latest"].is_object()
        || store
            .get("registration_pending")
            .is_some_and(|latches| !latches.is_object())
        || store.get("superseded").is_some_and(|ids| !ids.is_object())
    {
        return Err("invalid intent registry".into());
    }
    Ok(store)
}

fn entry(v: &Value) -> Value {
    let mut result = v.clone();
    if let Some(map) = result.as_object_mut() {
        map.remove("applied");
    }
    result
}

fn control(prompt: &str) -> Option<&'static str> {
    match prompt
        .trim()
        .trim_end_matches(['.', '!'])
        .to_ascii_lowercase()
        .as_str()
    {
        "cancel" | "cancel task" | "cancel this task" => Some("cancel"),
        "pause" | "pause task" | "pause this task" => Some("pause"),
        "resume" | "resume task" | "resume this task" => Some("resume"),
        _ => None,
    }
}

// Only the last user transcript record can identify the current prompt. An earlier
// identical prompt is insufficient. Transcript contents are never persisted or reported.
fn transcript_prompt_id(payload: &Value, prompt: &str) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = fs::File::open(payload.get("transcript_path")?.as_str()?).ok()?;
    if file.metadata().ok()?.len() > 64 * 1024 * 1024 {
        return None;
    }
    let mut latest = None;
    for line in BufReader::new(file).lines() {
        let value: Value = serde_json::from_str(&line.ok()?).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = value.get("message").and_then(|m| m.get("content"));
        let matches = content.and_then(Value::as_str) == Some(prompt)
            || content.and_then(Value::as_array).is_some_and(|parts| {
                parts.len() == 1
                    && parts[0].get("type").and_then(Value::as_str) == Some("text")
                    && parts[0].get("text").and_then(Value::as_str) == Some(prompt)
            });
        latest = if matches {
            value
                .get("uuid")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        } else {
            None
        };
    }
    latest
}

fn criterion(id: &str, command: &str) -> Value {
    json!({"id":id,"check":"repository_validation","mandatory":true,"command":command})
}

/// A conservative floor discovered before edits. Unsupported repositories need an explicit
/// preexisting contract; arbitrary model-selected commands are not a trusted check floor.
fn repository_floor(project: &Path) -> Result<Vec<Value>, String> {
    let project = PathBuf::from(project_identity(project)?);
    let mut checks = Vec::new();
    if project.join("Cargo.toml").is_file() {
        checks.extend([
            criterion("AUTO_FMT", "cargo fmt --all -- --check"),
            criterion(
                "AUTO_LINT",
                "cargo clippy --workspace --all-targets -- -D warnings",
            ),
            criterion("AUTO_TEST", "cargo test --workspace"),
        ]);
    }
    if let Some(package) = read(&project.join("package.json"))? {
        for (script, id) in [
            ("test", "AUTO_NPM_TEST"),
            ("lint", "AUTO_NPM_LINT"),
            ("typecheck", "AUTO_NPM_TYPES"),
        ] {
            if package
                .get("scripts")
                .and_then(|s| s.get(script))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                checks.push(criterion(id, &format!("npm run {script}")));
            }
        }
    }
    if project.join(".git").exists() {
        checks.push(criterion("AUTO_DIFF", "git diff --check"));
    }
    Ok(checks)
}

fn npm_definitions(project: &Path) -> Result<Value, String> {
    let project = PathBuf::from(project_identity(project)?);
    let mut scripts = serde_json::Map::new();
    if let Some(package) = read(&project.join("package.json"))?
        && let Some(entries) = package.get("scripts")
    {
        for (name, definition) in entries
            .as_object()
            .ok_or("package scripts must be an object")?
        {
            let definition = definition
                .as_str()
                .ok_or("package script must be a string")?;
            scripts.insert(
                name.clone(),
                crate::sha256::hex(definition.as_bytes()).into(),
            );
        }
    }
    Ok(scripts.into())
}

fn selected_entrypoints(contract: &Value, definitions: &Value) -> Result<Value, String> {
    let parsed = state::contract_from(contract)?;
    let mut selected = serde_json::Map::new();
    for command in parsed
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.command)
    {
        let parts: Vec<_> = command.split_whitespace().collect();
        let script = match parts.as_slice() {
            ["npm", "run", name, ..] => Some(*name),
            ["npm", "test", ..] => Some("test"),
            _ => None,
        };
        let Some(script) = script else {
            continue;
        };
        if definitions.get(script).and_then(Value::as_str).is_none() {
            return Err("required npm script was absent at prompt registration".into());
        }
        // npm invokes pre/post hooks for the selected entrypoint, including hooks added later.
        for name in [
            script.to_owned(),
            format!("pre{script}"),
            format!("post{script}"),
        ] {
            selected.insert(
                name.clone(),
                definitions.get(&name).cloned().unwrap_or(Value::Null),
            );
        }
    }
    Ok(selected.into())
}

fn check_entrypoint_map(expected: &Value, project: &Path) -> Result<(), String> {
    let expected = expected
        .as_object()
        .ok_or("invalid npm entrypoint snapshot")?;
    if expected.is_empty() {
        return Ok(());
    }
    let current = npm_definitions(project)?;
    for (name, digest) in expected {
        if !digest.is_null() && !digest.is_string() {
            return Err("invalid npm entrypoint digest".into());
        }
        if current.get(name).unwrap_or(&Value::Null) != digest {
            return Err("npm verification entrypoint changed or was removed; restore its captured definition; changing verification requires a separately reviewed migration".into());
        }
    }
    Ok(())
}

fn prompt_identity(payload: &Value, source: &str) -> Result<(String, String, bool), String> {
    let session = text(payload, "session_id")?;
    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .ok_or("prompt must be a string")?;
    let prompt_hash = crate::sha256::hex(prompt.as_bytes());
    let explicit_id = payload
        .get("prompt_id")
        .or_else(|| payload.get("uuid"))
        .or_else(|| payload.get("message_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let transcript_id = if source == "hook" {
        transcript_prompt_id(payload, prompt)
    } else {
        None
    };
    let identified = explicit_id.is_some() || transcript_id.is_some();
    let prompt_id = explicit_id
        .or(transcript_id.as_deref())
        .unwrap_or(&prompt_hash);
    // Length-delimited fields prevent delimiter collisions in caller-chosen identities.
    let event_id = crate::sha256::hex(json!([source, session, prompt_id]).to_string().as_bytes());
    Ok((event_id, prompt_hash, identified))
}

fn register(
    root: &Path,
    project: &Path,
    payload: &Value,
    source: &str,
    owner: Option<&str>,
    prompt_identity: (String, String, bool),
) -> Result<Value, String> {
    let session = text(payload, "session_id")?;
    let prompt = payload["prompt"]
        .as_str()
        .ok_or("prompt must be a string")?;
    let (event_id, prompt_hash, identified) = prompt_identity;
    let identity = project_identity(project)?;
    let tx = Transaction::open(root)?;
    let mut store = load(root)?;
    if source == "hook"
        && (owner.is_none()
            || store["registration_pending"][session]["token"].as_str() != owner
            || store["registration_pending"][session]["event_id"] != event_id)
    {
        return Err("prompt registration superseded by another delivery".into());
    }
    if let Some(old) = store["events"].get(&event_id) {
        validate_discovery_context(&old["project"], project)?;
        if old["prompt_hash"] != prompt_hash || !same_project(&old["project"], project)? {
            return Err("prompt event ID reused with different input".into());
        }
        if !identified && old.get("applied").is_some() {
            return Err("ambiguous repeated prompt without a harness event ID; cannot distinguish a new request from duplicate delivery".into());
        }
        return Ok(entry(old));
    }
    if source == "caller-asserted" {
        if store
            .get("registration_pending")
            .and_then(Value::as_object)
            .is_some_and(|latches| !latches.is_empty())
        {
            return Err(
                "trusted prompt registration is pending; caller-asserted intent cannot replace it"
                    .into(),
            );
        }
        if let Some(latest) = store["latest"]
            .get(session)
            .and_then(Value::as_str)
            .and_then(|id| store["events"].get(id))
            && latest["source"] == "hook"
        {
            return Err(
                "this session has trusted hook intent; caller-asserted intent cannot replace it"
                    .into(),
            );
        }
    }
    let active = lifecycle::active_task(root)?;
    let mut floor = repository_floor(project)?;
    if let Some(active) = &active {
        let (contract, digest) = state::read_contract(&active.dir)?;
        if let Stored::Tampered(_) = state::load_gate(&active.dir, &digest)? {
            return Err("active task gate is tampered".into());
        }
        if !floor.iter().any(|criterion| criterion["id"] != "AUTO_DIFF")
            && let Some(contract) = contract
        {
            floor.extend(contract.criteria.iter().filter(|c| c.mandatory).map(
                |c| json!({"id":c.id.0,"check":c.check.0,"command":c.command,"mandatory":true}),
            ));
        }
    } else if !floor.iter().any(|criterion| criterion["id"] != "AUTO_DIFF")
        && let Some(approved) = store.get("project_floor")
    {
        validate_discovery_context(&approved["project"], project)?;
        floor.extend(
            approved["required_criteria"]
                .as_array()
                .ok_or("invalid approved project floor")?
                .iter()
                .cloned(),
        );
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock before Unix epoch")?
        .as_nanos();
    let nonce = crate::sha256::hex(
        format!(
            "{event_id}:{now}:{}:{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        )
        .as_bytes(),
    );
    let event = json!({"event_id":event_id,"nonce":nonce,"prompt_hash":prompt_hash,"source":source,"session_id":session,"project":identity,"required_criteria":floor,"npm_definitions":npm_definitions(project)?,"control":control(prompt),"task_id":active.map(|a| a.task_id)});
    store["events"][&event_id] = event.clone();
    store["latest"][session] = event_id.into();
    if source == "hook" {
        let latches = store["registration_pending"]
            .as_object_mut()
            .ok_or("invalid registration latch")?;
        latches.remove(session);
        latches.remove("*");
    }
    tx.commit(vec![("intents.json".into(), format!("{store}\n"))])?;
    Ok(event)
}

/// Called by UserPromptSubmit, before the model receives the prompt. No prompt text is stored.
pub fn record_prompt(root: &Path, project: &Path, payload: &Value) -> Result<Value, String> {
    // Persist activation before probing manifests. A malformed manifest must never make
    // the next hook fall back to the legacy inactive path. Concurrent registrations clear
    // only their own latch; a newer pending prompt cannot be cleared by an older delivery.
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("*");
    let token = format!(
        "{}:{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    );
    let prompt_identity = {
        let tx = Transaction::open(root)?;
        let mut store = load(root)?;
        let prompt_identity = prompt_identity(payload, "hook");
        if let Ok((id, hash, identified)) = &prompt_identity {
            if store["superseded"].get(id).is_some() {
                return Err("prompt registration superseded by another delivery".into());
            }
            // A historical duplicate never displaces a newer pending registration.
            if let Some(old) = store["events"].get(id) {
                validate_discovery_context(&old["project"], project)?;
                if old["prompt_hash"] != *hash || !same_project(&old["project"], project)? {
                    return Err("prompt event ID reused with different input".into());
                }
                if !identified && old.get("applied").is_some() {
                    return Err("ambiguous repeated prompt without a harness event ID; cannot distinguish a new request from duplicate delivery".into());
                }
                return Ok(entry(old));
            }
        }
        if let Some(previous) = store["registration_pending"][session]["event_id"]
            .as_str()
            .filter(|previous| {
                prompt_identity
                    .as_ref()
                    .ok()
                    .map(|identity| identity.0.as_str())
                    != Some(*previous)
            })
            .map(str::to_owned)
        {
            if store.get("superseded").is_none() {
                store["superseded"] = json!({});
            }
            store["superseded"][previous] = true.into();
        }
        if store.get("registration_pending").is_none() {
            store["registration_pending"] = json!({});
        }
        store["registration_pending"]
            .as_object_mut()
            .ok_or("invalid registration latch")?
            .insert(session.into(), json!({"token":token,"event_id":prompt_identity.as_ref().ok().map(|identity| &identity.0)}));
        tx.commit(vec![("intents.json".into(), format!("{store}\n"))])?;
        prompt_identity
    };
    register(
        root,
        project,
        payload,
        "hook",
        Some(&token),
        prompt_identity?,
    )
}

/// Explicit advisory registration for harnesses without a trusted prompt hook.
pub fn record_intent(root: &Path, project: &Path, payload: &Value) -> Result<Value, String> {
    register(
        root,
        project,
        payload,
        "caller-asserted",
        None,
        prompt_identity(payload, "caller-asserted")?,
    )
}

/// Returns only the current unconsumed event. Older event delivery never replaces a newer one.
pub fn pending(root: &Path, session: &str) -> Result<Option<Value>, String> {
    let _tx = Transaction::open(root)?;
    let store = load(root)?;
    require_registered(&store, session)?;
    Ok(store["latest"]
        .get(session)
        .and_then(Value::as_str)
        .and_then(|id| store["events"].get(id))
        .filter(|e| e.get("applied").is_none())
        .map(entry))
}

fn require_registered(store: &Value, session: &str) -> Result<(), String> {
    if store
        .get("registration_pending")
        .is_some_and(|latches| latches.get(session).is_some() || latches.get("*").is_some())
    {
        return Err("trusted prompt registration is incomplete; mutation remains disabled until a valid prompt is registered".into());
    }
    Ok(())
}

/// A discussion turn consumes its prompt without cancelling any existing task. It does
/// not authorize edits until another event is applied as actionable work.
pub fn discussion_turn(root: &Path, session: &str) -> Result<bool, String> {
    let _tx = Transaction::open(root)?;
    let store = load(root)?;
    Ok(store["latest"]
        .get(session)
        .and_then(Value::as_str)
        .and_then(|id| store["events"].get(id))
        .is_some_and(|event| event["applied"]["response"]["action"] == "discussion"))
}

#[derive(Clone, Copy)]
pub(crate) enum IntentAuthority {
    Pending,
    Discussion,
    Actionable,
}

/// The caller holds the project transaction lock; do not acquire it recursively.
pub(crate) fn locked_intent_authority(
    root: &Path,
    session: &str,
) -> Result<(IntentAuthority, String), String> {
    let store = load(root)?;
    require_registered(&store, session)?;
    let current = store["latest"]
        .get(session)
        .and_then(Value::as_str)
        .and_then(|id| store["events"].get(id));
    let authority = match current {
        Some(event) if event.get("applied").is_none() => IntentAuthority::Pending,
        Some(event) if event["applied"]["response"]["action"] == "discussion" => {
            IntentAuthority::Discussion
        }
        _ => IntentAuthority::Actionable,
    };
    Ok((
        authority,
        crate::sha256::hex(crate::chain::canonical(&store).as_bytes()),
    ))
}

pub(crate) fn intent_authority(
    root: &Path,
    session: &str,
) -> Result<(IntentAuthority, String), String> {
    let _tx = Transaction::open(root)?;
    locked_intent_authority(root, session)
}

pub fn is_autonomous_task(dir: &Path) -> Result<bool, String> {
    Ok(read(&dir.join("autonomy.json"))?.is_some())
}

/// Hook-owned autonomous tasks cannot accept caller-asserted MCP completion.
pub fn requires_observed_completion(dir: &Path) -> Result<bool, String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(false);
    };
    match metadata["source"].as_str() {
        Some("hook") => Ok(true),
        Some("caller-asserted") => Ok(false),
        _ => Err("invalid autonomous task authority".into()),
    }
}

pub fn is_paused(dir: &Path) -> Result<bool, String> {
    Ok(read(&dir.join("autonomy.json"))?.is_some_and(|v| v["paused"] == true))
}

/// Safe recovery context for agents that cannot read protected task journals.
pub fn pending_jobs(dir: &Path, task_id: &str) -> Result<Value, String> {
    crate::observation::pending_jobs(dir, task_id)
}

/// Completion checks the actual selected npm entrypoints, not just their runner command.
pub fn validate_entrypoints(dir: &Path, project: &Path) -> Result<(), String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(());
    };
    if let Some(recorded) = metadata.get("project") {
        validate_discovery_context(recorded, project)?;
    }
    match metadata.get("npm_entrypoints") {
        Some(expected) => check_entrypoint_map(expected, project),
        None => {
            let contract =
                read(&dir.join("contract.json"))?.ok_or("autonomous contract missing")?;
            if selected_entrypoints(&contract, &json!({}))?
                .as_object()
                .is_some_and(|map| map.is_empty())
            {
                Ok(())
            } else {
                Err(
                    "npm entrypoint snapshot missing; establish an explicitly authorized new task"
                        .into(),
                )
            }
        }
    }
}

/// Manual tasks retain shared-session behavior; autonomous tasks have one explicit owner.
pub fn owns_session(dir: &Path, session: &str) -> Result<bool, String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(true);
    };
    if metadata["schema"] != 1
        || !metadata["revisions"].is_array()
        || metadata["session_id"].as_str().is_none()
    {
        return Err("invalid autonomous session ownership".into());
    }
    Ok(metadata["session_id"] == session)
}

/// Recovery authorizes polling a persisted job, never rewriting its original provenance.
pub(crate) fn recovers_session(
    dir: &Path,
    task: &str,
    original: &str,
    current: &str,
) -> Result<bool, String> {
    if original == current || !owns_session(dir, current)? {
        return Ok(false);
    }
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(false);
    };
    let identity = read(&dir.join("task.json"))?.ok_or("autonomous task identity missing")?;
    if identity["task_id"] != task {
        return Ok(false);
    }
    Ok(metadata["revisions"]
        .as_array()
        .ok_or("invalid ownership history")?
        .iter()
        .any(|revision| revision["action"] == "resume" && revision["from_session"] == original))
}

/// A revision preserves pending job records for recovery but requires fresh dispatches.
pub fn dispatch_is_current(dir: &Path, dispatch: &str) -> Result<bool, String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(true);
    };
    if metadata.get("schema").and_then(Value::as_u64) != Some(1)
        || !metadata["revisions"].is_array()
    {
        return Err("invalid autonomous task metadata".into());
    }
    let Some(invalidated) = metadata.get("invalidated_dispatches") else {
        return Ok(true);
    };
    let invalidated = invalidated
        .as_array()
        .ok_or("invalid dispatch invalidation history")?;
    if invalidated.iter().any(|id| !id.is_string()) {
        return Err("invalid dispatch invalidation identity".into());
    }
    Ok(!invalidated.iter().any(|id| id.as_str() == Some(dispatch)))
}

fn normalized_scope(v: &Value) -> Result<Vec<String>, String> {
    let paths = v
        .get("scope")
        .and_then(Value::as_array)
        .ok_or("scope must be an array")?;
    if paths.is_empty() {
        return Err("autonomous task scope cannot be empty".into());
    }
    paths
        .iter()
        .map(|path| {
            let path = path.as_str().ok_or("scope contains a non-string")?;
            let path = path.replace('\\', "/");
            let directory = path.ends_with('/');
            let bare = path.trim_end_matches('/');
            if bare.is_empty()
                || bare.contains(':')
                || bare.starts_with('/')
                || bare
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == ".." || p.trim() != p)
            {
                return Err(
                    "scope must contain normalized relative paths without traversal".into(),
                );
            }
            Ok(format!("{bare}{}", if directory { "/" } else { "" }))
        })
        .collect()
}

fn within(path: &str, allowed: &str) -> bool {
    if allowed.ends_with('/') {
        path.starts_with(allowed)
    } else {
        path == allowed
    }
}

fn validate_contract(contract: &Value) -> Result<Value, String> {
    let mut contract = contract.clone();
    contract["scope"] = normalized_scope(&contract)?.into();
    let parsed = state::contract_from(&contract)?;
    if parsed.id.0.trim().is_empty()
        || parsed.goal.trim().is_empty()
        || !parsed.criteria.iter().any(|c| c.mandatory)
    {
        return Err("contract requires an identity, goal and mandatory criteria".into());
    }
    let mut ids = BTreeSet::new();
    for c in parsed.criteria {
        if c.id.0.trim().is_empty() || c.check.0.trim().is_empty() || !ids.insert(c.id.0) {
            return Err("criteria must have distinct nonempty IDs and checks".into());
        }
    }
    Ok(contract)
}

fn preserves(old: &Value, new: &Value, allow_scope_change: bool) -> Result<(), String> {
    let old = state::contract_from(old)?;
    let new = state::contract_from(new)?;
    if old.id != new.id || old.goal != new.goal {
        return Err(
            "contract identity and initial goal cannot be rewritten; retain them and add criteria"
                .into(),
        );
    }
    for criterion in &old.criteria {
        let replacement = new
            .criteria
            .iter()
            .find(|c| c.id == criterion.id)
            .ok_or("contract revision removed a criterion")?;
        if replacement.check != criterion.check
            || replacement.command != criterion.command
            || (criterion.mandatory && !replacement.mandatory)
        {
            return Err("contract revision weakened or replaced a requirement".into());
        }
    }
    if old
        .forbidden_effects
        .iter()
        .any(|effect| !new.forbidden_effects.contains(effect))
        || new.budget.max_repair_rounds != old.budget.max_repair_rounds
    {
        return Err(
            "contract revision changed the repair budget or removed a forbidden effect".into(),
        );
    }
    if !allow_scope_change
        && new
            .scope
            .iter()
            .any(|scope| !old.scope.iter().any(|allowed| within(scope, allowed)))
    {
        return Err("scope expansion requires a new trusted prompt".into());
    }
    Ok(())
}

fn add_floor(contract: &Value, floor: &Value) -> Result<Value, String> {
    let mut contract = validate_contract(contract)?;
    let mut parsed = state::contract_from(&contract)?;
    let mut required_contract = contract.clone();
    required_contract["criteria"] = floor.clone();
    let required_contract = state::contract_from(&required_contract)?;
    let criteria = contract["criteria"]
        .as_array_mut()
        .ok_or("criteria must be an array")?;
    let floor = floor.as_array().ok_or("invalid requirement floor")?;
    if floor.is_empty() || floor.iter().all(|c| c["id"] == "AUTO_DIFF") {
        return Err("no deterministic repository checks found; establish an explicit initial contract first".into());
    }
    for (required, expected) in floor.iter().zip(&required_contract.criteria) {
        if let Some(existing) = parsed.criteria.iter().find(|c| c.id == expected.id) {
            if existing.check != expected.check
                || existing.command != expected.command
                || existing.mandatory != expected.mandatory
            {
                return Err(
                    "proposal conflicts with immutable repository requirement floor".into(),
                );
            }
        } else {
            criteria.push(required.clone());
            parsed.criteria.push(expected.clone());
        }
    }
    validate_contract(&contract)
}

fn gate_state(dir: &Path) -> Result<&'static str, String> {
    let (_, digest) = state::read_contract(dir)?;
    match state::load_gate(dir, &digest)? {
        Stored::Absent => Ok("queued"),
        Stored::Valid(gate) => Ok(state::state_name(gate.checkpoint.state)),
        Stored::Tampered(_) => Err("task gate is tampered".into()),
    }
}

fn observed_completion(dir: &Path, task_id: &str, project: &Path) -> Result<(), String> {
    let (contract, digest) = state::read_contract(dir)?;
    let contract = contract.ok_or("completed task contract missing")?;
    let Stored::Valid(gate) = state::load_gate(dir, &digest)? else {
        return Err("completed task has no authentic gate".into());
    };
    let candidate = gate
        .checkpoint
        .candidate
        .as_ref()
        .ok_or("completed task has no observed candidate")?;
    let rows = fs::read_to_string(dir.join("runs.jsonl"))
        .map_err(|_| "completed task has no observed command journal")?;
    let rows: Vec<Value> = rows
        .lines()
        .map(|line| {
            serde_json::from_str(line).map_err(|_| "invalid command journal during rollover")
        })
        .collect::<Result<_, _>>()?;
    let execution_root = crate::location::canonical_project(project);
    for criterion in contract.criteria.iter().filter(|c| c.mandatory) {
        let latest = rows.iter().rev().find(|row| {
            row["kind"] == "dispatch"
                && row["cwd"].as_str() == Some(execution_root.as_str())
                && row["task_id"] == task_id
                && row.get("candidate").is_some_and(|c| {
                    c["base_commit"] == candidate.base_commit
                        && c["dirty_digest"] == candidate.dirty_digest
                })
                && row
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| criterion.accepts(command))
        });
        let dispatch = latest
            .and_then(|row| row.get("dispatch_id"))
            .and_then(Value::as_str)
            .ok_or("completed requirement has no observed dispatch")?;
        if !dispatch_is_current(dir, dispatch)? {
            return Err("completed requirement dispatch predates a contract revision".into());
        }
        let observed = format!("observed:{task_id}:{dispatch}");
        let reconstructed = format!("tc-reconstructed:{task_id}:{dispatch}");
        let receipt = gate.receipts.iter().rev().find(|receipt| {
            receipt.criterion == criterion.id
                && receipt.candidate == *candidate
                && (receipt.output_ref == observed || receipt.output_ref == reconstructed)
        });
        if !receipt.is_some_and(|receipt| {
            receipt.exit_code == Some(0)
                && crate::adapter::receipt_is_observed(dir, task_id, receipt)
        }) {
            return Err(
                "completed requirement lacks its latest successful hook-observed receipt".into(),
            );
        }
    }
    Ok(())
}

/// Apply once under the existing OS lock and write-ahead transaction. Replayed applications
/// return their historical response and never reactivate or rebind an archived task.
pub fn propose(root: &Path, project: &Path, proposal: &Value) -> Result<Value, String> {
    let id = text(proposal, "event_id")?;
    let tx = Transaction::open(root)?;
    let mut store = load(root)?;
    let event = store["events"]
        .get(id)
        .cloned()
        .ok_or("unknown prompt event")?;
    if event["nonce"].as_str() != Some(text(proposal, "nonce")?)
        || event["prompt_hash"].as_str() != Some(text(proposal, "prompt_hash")?)
    {
        return Err("proposal does not match its recorded prompt event".into());
    }
    validate_discovery_context(&event["project"], project)?;
    let digest = crate::sha256::hex(crate::chain::canonical(proposal).as_bytes());
    if let Some(applied) = event.get("applied") {
        if applied["digest"] != digest {
            return Err("prompt event already consumed by a different proposal".into());
        }
        return applied
            .get("response")
            .cloned()
            .ok_or_else(|| "invalid applied proposal".into());
    }
    let session = text(&event, "session_id")?;
    require_registered(&store, session)?;
    if store["latest"][session] != id {
        return Err("prompt event superseded by a newer prompt".into());
    }
    let action = text(proposal, "action")?;
    let active = lifecycle::active_task(root)?;
    let current_id = active.as_ref().map(|a| a.task_id.as_str());
    if event["task_id"].as_str() != current_id {
        return Err("active task changed after prompt registration".into());
    }
    let mut writes = Vec::new();
    let response = if action == "discussion" {
        json!({"action":action,"task_id":current_id,"inactive":active.is_none(),"unchanged":true})
    } else if action == "start" {
        if let Some(active) = &active
            && read(&active.dir.join("autonomy.json"))?.is_some_and(|m| m["source"] == "hook")
            && event["source"] != "hook"
        {
            return Err("caller-asserted intent cannot replace a trusted hook task".into());
        }
        if let Some(active) = &active
            && gate_state(&active.dir)? != "completed"
        {
            return Err(
                "unfinished task retained; continue it or explicitly cancel before a new task"
                    .into(),
            );
        }
        if let Some(active) = &active
            && read(&active.dir.join("autonomy.json"))?.is_some_and(|m| m["source"] == "hook")
        {
            validate_entrypoints(&active.dir, project)?;
            observed_completion(&active.dir, &active.task_id, project)?;
        }
        let mut contract = add_floor(
            proposal.get("contract").ok_or("start needs a contract")?,
            &event["required_criteria"],
        )?;
        contract["event_id"] = format!("autonomy-{id}").into();
        let (mut response, task_writes) =
            lifecycle::plan_start(root, &contract, active.is_some(), None, current_id)?;
        writes.extend(task_writes);
        let task_id = text(&response, "task_id")?;
        let npm_entrypoints = selected_entrypoints(
            &contract,
            event.get("npm_definitions").unwrap_or(&json!({})),
        )?;
        check_entrypoint_map(&npm_entrypoints, project)?;
        let metadata = json!({"schema":1,"source":event["source"],"project":event["project"],"session_id":session,"initial_event":id,"requirement_floor":contract,"npm_entrypoints":npm_entrypoints,"paused":false,"revisions":[{"event_id":id,"action":"start","contract":contract}]});
        writes.push((
            PathBuf::from("tasks").join(task_id).join("autonomy.json"),
            format!("{metadata}\n"),
        ));
        response["action"] = action.into();
        response
    } else {
        let active = active.as_ref().ok_or("no active task")?;
        if proposal.get("task_id").and_then(Value::as_str) != Some(&active.task_id) {
            return Err("proposal requires the current task_id".into());
        }
        let mut metadata = read(&active.dir.join("autonomy.json"))?.unwrap_or_else(|| json!({"schema":1,"source":event["source"],"session_id":session,"paused":false,"revisions":[]}));
        if action != "cancel"
            && let Some(recorded) = metadata.get("project")
        {
            validate_discovery_context(recorded, project)?;
        }
        if metadata.get("npm_entrypoints").is_none() {
            let current_contract =
                read(&active.dir.join("contract.json"))?.ok_or("active contract missing")?;
            let definitions = if metadata.get("requirement_floor").is_some() {
                json!({})
            } else {
                event
                    .get("npm_definitions")
                    .cloned()
                    .unwrap_or_else(|| json!({}))
            };
            metadata["npm_entrypoints"] = selected_entrypoints(&current_contract, &definitions)?;
            metadata
                .as_object_mut()
                .ok_or("invalid autonomous metadata")?
                .entry("project")
                .or_insert_with(|| event["project"].clone());
        }
        if metadata["source"] == "hook" && event["source"] != "hook" {
            return Err("caller-asserted intent cannot control a trusted hook task".into());
        }
        let transfer = metadata["session_id"] != session;
        if transfer
            && !(action == "resume"
                && event["control"] == "resume"
                && metadata["source"] == event["source"])
        {
            return Err("proposal belongs to another task session".into());
        }
        let task_state = gate_state(&active.dir)?;
        if matches!(action, "cancel" | "pause" | "resume") && event["control"] != action {
            return Err("control action requires an unambiguous direct user control prompt".into());
        }
        let retire_legacy_completed = task_state == "completed"
            && action == "cancel"
            && event["source"] == "hook"
            && metadata["source"] == "hook"
            && metadata
                .get("project")
                .map(|recorded| legacy_subdirectory_context(recorded, project))
                .transpose()?
                .unwrap_or(false);
        if task_state == "completed" && !retire_legacy_completed {
            return Err("completed task is immutable; start a new task".into());
        }
        let mut revision = json!({"event_id":id,"action":action});
        if transfer {
            revision["from_session"] = metadata["session_id"].clone();
            revision["to_session"] = session.into();
            metadata["session_id"] = session.into();
        }
        match action {
            "continue" => {
                if metadata["paused"] == true {
                    return Err("task is paused; explicit resume required".into());
                }
            }
            "pause" => metadata["paused"] = true.into(),
            "resume" => metadata["paused"] = false.into(),
            "cancel" => {
                let (contract, digest) = state::read_contract(&active.dir)?;
                if let Stored::Tampered(_) = state::load_gate(&active.dir, &digest)? {
                    return Err("active task gate is tampered".into());
                }
                let contract = contract.ok_or("active contract missing")?;
                let floor: Vec<Value> = contract
                    .criteria
                    .iter()
                    .filter(|c| c.mandatory)
                    .map(|c| json!({"id":c.id.0,"check":c.check.0,"command":c.command,"mandatory":true}))
                    .collect();
                // Retain only the approved task checks, never the proposed replacement.
                store["project_floor"] = json!({"project":metadata.get("project").unwrap_or(&event["project"]),"required_criteria":floor});
                let mut archive = json!({"schema":2,"task_id":active.task_id,"disposition":"cancelled","successor":null,"event_id":id});
                if retire_legacy_completed {
                    archive["unverified_legacy_context"] = true.into();
                }
                writes.push((
                    PathBuf::from("archives").join(format!("{}.json", active.task_id)),
                    format!("{archive}\n"),
                ));
                writes.push((
                    "active.json".into(),
                    "{\"schema\":2,\"task_id\":null}\n".into(),
                ));
            }
            "revise" => {
                let old =
                    read(&active.dir.join("contract.json"))?.ok_or("active contract missing")?;
                let new =
                    validate_contract(proposal.get("contract").ok_or("revise needs a contract")?)?;
                let new_entrypoints =
                    selected_entrypoints(&new, event.get("npm_definitions").unwrap_or(&json!({})))?;
                if metadata.get("npm_entrypoints").is_none() {
                    metadata["npm_entrypoints"] = json!({});
                }
                let expected = metadata["npm_entrypoints"]
                    .as_object_mut()
                    .ok_or("invalid npm entrypoint snapshot")?;
                for (name, digest) in new_entrypoints
                    .as_object()
                    .ok_or("invalid npm entrypoint selection")?
                {
                    expected.entry(name).or_insert_with(|| digest.clone());
                }
                preserves(&old, &new, event["source"] == "hook")?;
                if let Some(floor) = metadata.get("requirement_floor") {
                    preserves(floor, &new, event["source"] == "hook")?;
                }
                let stage = root.join("autonomy-stage");
                if stage.exists() {
                    fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
                }
                copy_task(&active.dir, &stage)?;
                let (_, old_digest) = state::read_contract(&stage)?;
                let gate = state::load_gate(&stage, &old_digest)?;
                revision["previous_gate"] = read(&stage.join("gate.json"))?.unwrap_or(Value::Null);
                let mut invalidated: BTreeSet<String> = metadata
                    .get("invalidated_dispatches")
                    .map(|v| {
                        v.as_array()
                            .ok_or("invalid dispatch invalidation history")?
                            .iter()
                            .map(|id| {
                                id.as_str()
                                    .map(str::to_owned)
                                    .ok_or("invalid dispatch identity")
                            })
                            .collect::<Result<BTreeSet<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                match fs::read_to_string(stage.join("runs.jsonl")) {
                    Ok(rows) => {
                        for row in rows.lines() {
                            let row: Value = serde_json::from_str(row)
                                .map_err(|_| "invalid command journal during revision")?;
                            if let Some(dispatch) = row.get("dispatch_id").and_then(Value::as_str) {
                                invalidated.insert(dispatch.to_owned());
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
                metadata["invalidated_dispatches"] =
                    invalidated.into_iter().collect::<Vec<_>>().into();
                atomic_write(&stage.join("contract.json"), format!("{new}\n").as_bytes())?;
                let (_, new_digest) = state::read_contract(&stage)?;
                if let Stored::Valid(mut gate) = gate {
                    gate.receipts.clear();
                    if gate.checkpoint.state == TaskState::ReadyForReview {
                        gate.checkpoint.state = TaskState::Verifying;
                    }
                    state::save_gate(&stage, &gate, &new_digest)?;
                }
                writes.extend(task_writes(
                    &stage,
                    &PathBuf::from("tasks").join(&active.task_id),
                )?);
                revision["previous_contract"] = old;
                revision["contract"] = new;
            }
            _ => return Err("unknown lifecycle proposal action".into()),
        }
        if !retire_legacy_completed {
            if metadata.get("requirement_floor").is_none() {
                metadata["requirement_floor"] =
                    read(&active.dir.join("contract.json"))?.ok_or("active contract missing")?;
            }
            metadata["revisions"]
                .as_array_mut()
                .ok_or("invalid task revision history")?
                .push(revision);
            writes.push((
                PathBuf::from("tasks")
                    .join(&active.task_id)
                    .join("autonomy.json"),
                format!("{metadata}\n"),
            ));
        }
        json!({"action":action,"task_id":active.task_id,"state":task_state,"paused":metadata["paused"],"inactive":action=="cancel"})
    };
    if action != "discussion" && action != "cancel" {
        let mut sessions = read(&root.join("sessions.json"))?.unwrap_or_else(|| json!({}));
        sessions
            .as_object_mut()
            .ok_or("invalid session bindings")?
            .insert(session.to_owned(), response["task_id"].clone());
        writes.push(("sessions.json".into(), format!("{sessions}\n")));
    }
    store["events"][id]["applied"] = json!({"digest":digest,"response":response});
    writes.push(("intents.json".into(), format!("{store}\n")));
    tx.commit(writes)?;
    Ok(response)
}

#[cfg(test)]
mod registration_repair_tests {
    use super::*;

    #[test]
    fn superseded_registration_cannot_mutate_newer_intent_or_return_on_public_retry() {
        let project = std::env::temp_dir().join(format!(
            "supervisor-registration-owner-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&project).unwrap();
        let root = project.join("state");
        let older = json!({"session_id":"session","prompt_id":"older","prompt":"Explain the older request"});
        let older_id =
            crate::sha256::hex(json!(["hook", "session", "older"]).to_string().as_bytes());
        // Model the durable first transaction of A, then let public hook B finish.
        // Only disposable fixture state is written; no active supervisor state is touched.
        let tx = Transaction::open(&root).unwrap();
        let mut store = load(&root).unwrap();
        store["registration_pending"] =
            json!({"session":{"token":std::process::id().to_string(),"event_id":older_id}});
        tx.commit(vec![("intents.json".into(), format!("{store}\n"))])
            .unwrap();
        drop(tx);
        let newer = record_prompt(&root, &project, &json!({"session_id":"session","prompt_id":"newer","prompt":"Explain the newer request"})).unwrap();
        let before = fs::read(root.join("intents.json")).unwrap();
        let delayed = register(
            &root,
            &project,
            &older,
            "hook",
            Some(&std::process::id().to_string()),
            prompt_identity(&older, "hook").unwrap(),
        );
        assert!(
            delayed.is_err(),
            "superseded registration returned an event"
        );
        assert_eq!(
            fs::read(root.join("intents.json")).unwrap(),
            before,
            "stale registration mutated intent"
        );
        assert!(
            record_prompt(&root, &project, &older).is_err(),
            "public retry resurrected superseded intent"
        );
        assert_eq!(
            pending(&root, "session").unwrap().unwrap()["event_id"],
            newer["event_id"]
        );
        propose(&root, &project, &json!({"event_id":newer["event_id"],"nonce":newer["nonce"],"prompt_hash":newer["prompt_hash"],"action":"discussion"})).unwrap();
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn historical_duplicate_preserves_a_newer_pending_latch_and_other_sessions() {
        let project = std::env::temp_dir().join(format!(
            "supervisor-registration-history-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&project).unwrap();
        let root = project.join("state");
        let older = json!({"session_id":"session","prompt_id":"older","prompt":"Explain the older request"});
        let historical = record_prompt(&root, &project, &older).unwrap();
        let newer = json!({"session_id":"session","prompt_id":"newer","prompt":"Explain the newer request"});
        let newer_id = prompt_identity(&newer, "hook").unwrap().0;
        let owner = std::process::id().to_string();
        let tx = Transaction::open(&root).unwrap();
        let mut store = load(&root).unwrap();
        store["registration_pending"] = json!({
            "session":{"token":owner,"event_id":newer_id},
            "other-session":{"token":owner,"event_id":"other-event"}
        });
        tx.commit(vec![("intents.json".into(), format!("{store}\n"))])
            .unwrap();
        drop(tx);
        let before = fs::read(root.join("intents.json")).unwrap();
        assert_eq!(
            record_prompt(&root, &project, &older).unwrap()["nonce"],
            historical["nonce"]
        );
        assert_eq!(fs::read(root.join("intents.json")).unwrap(), before);
        assert!(pending(&root, "session").is_err());
        let current = register(
            &root,
            &project,
            &newer,
            "hook",
            Some(&owner),
            prompt_identity(&newer, "hook").unwrap(),
        )
        .unwrap();
        assert_eq!(
            pending(&root, "session").unwrap().unwrap()["event_id"],
            current["event_id"]
        );
        assert!(pending(&root, "other-session").is_err());
        propose(&root, &project, &json!({"event_id":current["event_id"],"nonce":current["nonce"],"prompt_hash":current["prompt_hash"],"action":"discussion"})).unwrap();
        let _ = fs::remove_dir_all(project);
    }
}
