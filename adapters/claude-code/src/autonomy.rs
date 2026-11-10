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

/// `identity` is [`project_identity`] of the caller's project, computed before the state lock:
/// it spawns git, and git under the lock made concurrent prompts queue past its deadline (G26).
fn same_project(recorded: &Value, identity: &str) -> Result<bool, String> {
    // Schema 1 originally stored raw canonical Windows paths, including nested cwd.
    Ok(recorded.as_str().is_some_and(|old| {
        old == identity || project_identity(Path::new(old)).as_deref() == Ok(identity)
    }))
}

fn legacy_subdirectory_context(recorded: &Value, identity: &str) -> Result<bool, String> {
    let Some(recorded) = recorded.as_str() else {
        return Ok(false);
    };
    let captured =
        fs::canonicalize(recorded).map_err(|_| "recorded discovery directory unavailable")?;
    let root = fs::canonicalize(identity).map_err(|_| "project root unavailable")?;
    Ok(captured != root && crate::location::canonical_project(&captured) == identity)
}

fn validate_discovery_context(recorded: &Value, identity: &str) -> Result<(), String> {
    if !same_project(recorded, identity)? {
        return Err("discovery snapshot belongs to another physical worktree".into());
    }
    if legacy_subdirectory_context(recorded, identity)? {
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

/// Whether a prompt may be a control request even though it is not the exact control phrase:
/// a short prompt (80 characters or fewer) with one of these words or "hold on". Such a prompt
/// is never taken as a default `continue` and a Stop never records it as discussion; it still
/// needs an explicit `intent_apply`. Longer prompts only count when they are exact.
/// ponytail: word match; "stop using unwrap" in a short edit request also waits for intent_apply.
fn control_hint(prompt: &str) -> bool {
    if control(prompt).is_some() {
        return true;
    }
    let lowered = prompt.trim().to_ascii_lowercase();
    if lowered.chars().count() > 80 {
        return false;
    }
    lowered.contains("hold on")
        || lowered
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| matches!(w, "cancel" | "pause" | "stop" | "abort" | "resume"))
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

/// The root package.json (dir `None`) and those in direct subfolders, skipping `node_modules`,
/// hidden folders and names a shell would need quoted.
/// shortcut: workspace globs and deeper folders are not read; add them when a repo needs it.
fn npm_packages(project: &Path) -> Result<Vec<(Option<String>, Value)>, String> {
    let mut packages = Vec::new();
    if let Some(package) = read(&project.join("package.json"))? {
        packages.push((None, package));
    }
    for dir in subfolders(project)? {
        if let Some(package) = read(&project.join(&dir).join("package.json"))? {
            packages.push((Some(dir), package));
        }
    }
    Ok(packages)
}

/// The id suffix for a discovered subfolder: its name in upper case with `-` as `_`, unless
/// another folder of the same kind (`dirs`) would get the same suffix; then the name as it is,
/// so `a-b` and `a_b` keep distinct checks (G33 review). Lone folders keep the stable form.
fn id_suffix<S: AsRef<str>>(dir: &str, dirs: &[S]) -> String {
    let normal = |d: &str| d.to_ascii_uppercase().replace('-', "_");
    if dirs
        .iter()
        .filter(|d| normal(d.as_ref()) == normal(dir))
        .count()
        > 1
    {
        dir.to_owned()
    } else {
        normal(dir)
    }
}

/// The direct subfolders discovery looks into, sorted: plain names only, no hidden folders and
/// no `node_modules`.
fn subfolders(project: &Path) -> Result<Vec<String>, String> {
    let mut dirs: Vec<String> = match fs::read_dir(project) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| {
                name != "node_modules"
                    && !name.starts_with('.')
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .collect(),
        Err(e) => return Err(format!("{}: {e}", project.display())),
    };
    dirs.sort();
    Ok(dirs)
}

/// K14: read `contract_file` (relative to the project) into `contract`, so the proposal is
/// validated and replayed exactly as if the contract had been sent inline.
fn with_contract_file(project: &Path, proposal: &Value, file: &Value) -> Result<Value, String> {
    let name = file.as_str().ok_or("contract_file must be a string")?;
    if proposal.get("contract").is_some() {
        return Err("send contract or contract_file, not both".into());
    }
    let path = project.join(name);
    let text = fs::read_to_string(&path).map_err(|e| format!("contract_file {name}: {e}"))?;
    let contract: Value = serde_json::from_str(&text)
        .map_err(|e| format!("contract_file {name} is not valid JSON: {e}"))?;
    let mut proposal = proposal.clone();
    let fields = proposal
        .as_object_mut()
        .ok_or("proposal must be an object")?;
    fields.remove("contract_file");
    fields.insert("contract".into(), contract);
    Ok(proposal)
}

/// Add kept criteria to the floor, skipping ids it already has: a discovered check (such as
/// AUTO_DIFF) stays the only entry under its id, or no proposal could match both.
fn extend_floor(floor: &mut Vec<Value>, extra: Vec<Value>) {
    for criterion in extra {
        if !floor.iter().any(|c| c["id"] == criterion["id"]) {
            floor.push(criterion);
        }
    }
}

/// A conservative floor discovered before edits. Unsupported repositories need an explicit
/// preexisting contract; arbitrary model-selected commands are not a trusted check floor.
pub fn repository_floor(project: &Path) -> Result<Vec<Value>, String> {
    floor_at(Path::new(&project_identity(project)?))
}

/// [`repository_floor`] of a project root already resolved by [`project_identity`].
fn floor_at(project: &Path) -> Result<Vec<Value>, String> {
    let project = project.to_path_buf();
    let mut checks = Vec::new();
    // G33: a Cargo.toml at the root, else one in each direct subfolder (a virtual workspace
    // manifest counts), run from the root with `--manifest-path` like npm's `--prefix`.
    let cargo_dirs: Vec<Option<String>> = if project.join("Cargo.toml").is_file() {
        vec![None]
    } else {
        subfolders(&project)?
            .into_iter()
            .filter(|dir| project.join(dir).join("Cargo.toml").is_file())
            .map(Some)
            .collect()
    };
    let cargo_names: Vec<&str> = cargo_dirs.iter().flatten().map(String::as_str).collect();
    for dir in &cargo_dirs {
        let (suffix, manifest) = match dir {
            None => (String::new(), String::new()),
            Some(dir) => (
                format!("_{}", id_suffix(dir, &cargo_names)),
                format!(" --manifest-path {dir}/Cargo.toml"),
            ),
        };
        // G20: nextest runs the same tests faster; `cargo test` stays the advised first command.
        let mut test = criterion(
            &format!("AUTO_TEST{suffix}"),
            &format!("cargo test{manifest} --workspace"),
        );
        test["command"] = json!([
            format!("cargo test{manifest} --workspace"),
            format!("cargo nextest run{manifest} --workspace"),
        ]);
        checks.extend([
            criterion(
                &format!("AUTO_FMT{suffix}"),
                &format!("cargo fmt --all{manifest} -- --check"),
            ),
            criterion(
                &format!("AUTO_LINT{suffix}"),
                &format!("cargo clippy{manifest} --workspace --all-targets -- -D warnings"),
            ),
            test,
        ]);
    }
    let packages = npm_packages(&project)?;
    // Subfolder packages are the floor only when the root has none of its own.
    let root_only = packages.iter().any(|(dir, _)| dir.is_none());
    let npm_names: Vec<&str> = packages
        .iter()
        .filter_map(|(dir, _)| dir.as_deref())
        .collect();
    for (dir, package) in packages
        .iter()
        .filter(|(dir, _)| dir.is_none() || !root_only)
    {
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
                checks.push(match dir {
                    None => criterion(id, &format!("npm run {script}")),
                    Some(dir) => criterion(
                        &format!("{id}_{}", id_suffix(dir, &npm_names)),
                        &format!("npm --prefix {dir} run {script}"),
                    ),
                });
            }
        }
    }
    if project.join(".git").exists() {
        checks.push(criterion("AUTO_DIFF", "git diff --check"));
    }
    Ok(checks)
}

/// The npm script definitions under a root resolved by [`project_identity`].
fn npm_definitions_at(project: &Path) -> Result<Value, String> {
    let project = project.to_path_buf();
    let mut scripts = serde_json::Map::new();
    for (dir, package) in npm_packages(&project)? {
        let Some(entries) = package.get("scripts") else {
            continue;
        };
        for (name, definition) in entries
            .as_object()
            .ok_or("package scripts must be an object")?
        {
            let definition = definition
                .as_str()
                .ok_or("package script must be a string")?;
            let key = dir
                .as_ref()
                .map_or_else(|| name.clone(), |dir| format!("{dir}/{name}"));
            scripts.insert(key, crate::sha256::hex(definition.as_bytes()).into());
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
        let (dir, script) = match parts.as_slice() {
            ["npm", "run", name, ..] => ("", *name),
            ["npm", "test", ..] => ("", "test"),
            ["npm", "--prefix", dir, "run", name, ..] => (*dir, *name),
            ["npm", "--prefix", dir, "test", ..] => (*dir, "test"),
            _ => continue,
        };
        let dir = dir.trim_start_matches("./").trim_end_matches('/');
        // Only direct subfolders are discovered; a deeper prefix stays unselected.
        if dir.contains(['/', '\\']) {
            continue;
        }
        let key = |name: &str| {
            if dir.is_empty() {
                name.to_owned()
            } else {
                format!("{dir}/{name}")
            }
        };
        if definitions
            .get(key(script))
            .and_then(Value::as_str)
            .is_none()
        {
            return Err("required npm script was absent at prompt registration".into());
        }
        // npm invokes pre/post hooks for the selected entrypoint, including hooks added later.
        for name in [
            key(script),
            key(&format!("pre{script}")),
            key(&format!("post{script}")),
        ] {
            selected.insert(
                name.clone(),
                definitions.get(&name).cloned().unwrap_or(Value::Null),
            );
        }
    }
    Ok(selected.into())
}

/// `root` is the project root resolved by [`project_identity`].
fn check_entrypoint_map(expected: &Value, root: &Path) -> Result<(), String> {
    let expected = expected
        .as_object()
        .ok_or("invalid npm entrypoint snapshot")?;
    if expected.is_empty() {
        return Ok(());
    }
    let current = npm_definitions_at(root)?;
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
        validate_discovery_context(&old["project"], &identity)?;
        if old["prompt_hash"] != prompt_hash || !same_project(&old["project"], &identity)? {
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
    let mut floor = floor_at(Path::new(&identity))?;
    // A person may replace a discovered command (`supervisor task amend-floor`).
    for check in &mut floor {
        if let Some(command) = check["id"]
            .as_str()
            .and_then(|id| store["floor_overrides"].get(id))
            .and_then(|o| o.get("command"))
        {
            check["command"] = command.clone();
        }
    }
    if let Some(active) = &active {
        let (contract, digest) = state::read_contract(&active.dir)?;
        if let Stored::Tampered(_) = state::load_gate(&active.dir, &digest)? {
            return Err("active task gate is tampered".into());
        }
        if !floor.iter().any(|criterion| criterion["id"] != "AUTO_DIFF")
            && let Some(contract) = contract
        {
            let extra: Vec<Value> = contract
                .criteria
                .iter()
                .filter(|c| c.mandatory)
                .map(
                    |c| json!({"id":c.id.0,"check":c.check.0,"command":c.command,"mandatory":true}),
                )
                .collect();
            extend_floor(&mut floor, extra);
        }
    } else if !floor.iter().any(|criterion| criterion["id"] != "AUTO_DIFF")
        && let Some(approved) = store.get("project_floor")
    {
        validate_discovery_context(&approved["project"], &identity)?;
        extend_floor(
            &mut floor,
            approved["required_criteria"]
                .as_array()
                .ok_or("invalid approved project floor")?
                .to_vec(),
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
    // G18: this session's own unfinished, unpaused task takes an unclassified prompt as
    // `continue`. Control prompts and other sessions' tasks keep the explicit proposal.
    let continues = match &active {
        Some(active) if source == "hook" && !control_hint(prompt) => {
            continues_by_default(&active.dir, session)?
        }
        _ => false,
    };
    let mut event = json!({"event_id":event_id,"nonce":nonce,"prompt_hash":prompt_hash,"source":source,"session_id":session,"project":identity,"required_criteria":floor,"npm_definitions":npm_definitions_at(Path::new(&identity))?,"control":control(prompt),"task_id":active.map(|a| a.task_id)});
    if continues {
        event["default_action"] = "continue".into();
    }
    if control_hint(prompt) && control(prompt).is_none() {
        event["control_hint"] = true.into();
    }
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

/// Whether `session` owns the prompt-managed hook task in `dir` and it is neither paused nor
/// finished.
fn continues_by_default(dir: &Path, session: &str) -> Result<bool, String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(false);
    };
    if metadata["source"] != "hook"
        || metadata["paused"] == true
        || metadata["session_id"] != session
    {
        return Ok(false);
    }
    let (_, digest) = state::read_contract(dir)?;
    Ok(match state::load_gate(dir, &digest)? {
        Stored::Valid(gate) => !gate.checkpoint.state.is_terminal(),
        Stored::Absent => true,
        Stored::Tampered(_) => false,
    })
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
    let identity = project_identity(project)?;
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
                validate_discovery_context(&old["project"], &identity)?;
                if old["prompt_hash"] != *hash || !same_project(&old["project"], &identity)? {
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
        .filter(|e| e.get("applied").is_none() && e.get("default_action").is_none())
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

/// Whether `session` has any prompt registered in this state root.
pub fn registered(root: &Path, session: &str) -> Result<bool, String> {
    Ok(load(root)?["latest"].get(session).is_some())
}

/// A discussion turn records its prompt without cancelling any existing task. It does
/// not authorize edits until the same event or a later one is applied as actionable work.
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
        Some(event) if event.get("applied").is_none() && event.get("default_action").is_none() => {
            IntentAuthority::Pending
        }
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

/// A person's change to one command of the stored project floor (the floor kept after a
/// cancel when the repository's files yield no checks). Recorded with old and new command and
/// the reason; the PreToolUse guard refuses this command to agents.
pub fn amend_floor(
    root: &Path,
    project: &Path,
    criterion: &str,
    command: &str,
    reason: &str,
) -> Result<Value, String> {
    let (command, reason) = (command.trim(), reason.trim());
    if command.is_empty() || reason.is_empty() {
        return Err("amend-floor needs a nonempty --command and --reason".into());
    }
    let identity = project_identity(project)?;
    let tx = Transaction::open(root)?;
    if lifecycle::active_task(root)?.is_some() {
        return Err(
            "a task is active and its contract owns the floor; finish or cancel it first".into(),
        );
    }
    let mut store = load(root)?;
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let discovered = floor_at(Path::new(&identity))?;
    if let Some(found) = discovered.iter().find(|c| c["id"] == criterion) {
        let entry = &mut store["floor_overrides"][criterion];
        let old = match entry.get("command").unwrap_or(&found["command"]) {
            Value::String(s) => json!([s]),
            other => other.clone(),
        };
        let mut amendments = entry
            .get("amendments")
            .cloned()
            .unwrap_or_else(|| json!([]));
        amendments
            .as_array_mut()
            .ok_or("invalid floor override history")?
            .push(json!({"t":t,"criterion":criterion,"old_command":old,"new_command":command,"reason":reason}));
        *entry = json!({"command":command,"amendments":amendments});
        tx.commit(vec![("intents.json".into(), format!("{store}\n"))])?;
        return Ok(
            json!({"criterion":criterion,"old_command":old,"new_command":command,"amended":true}),
        );
    }
    if discovered.iter().any(|c| c["id"] != "AUTO_DIFF") {
        return Err(format!(
            "no discovered check is named {criterion}; this repository's checks are discovered from its files"
        ));
    }
    let floor = store
        .get_mut("project_floor")
        .ok_or("no stored project floor; one is kept when a prompt-managed task is cancelled")?;
    validate_discovery_context(&floor["project"], &identity)?;
    let entry = floor["required_criteria"]
        .as_array_mut()
        .ok_or("invalid approved project floor")?
        .iter_mut()
        .find(|c| c["id"] == criterion)
        .ok_or_else(|| format!("the stored floor has no criterion {criterion}"))?;
    let old = match &entry["command"] {
        Value::String(s) => json!([s]),
        other => other.clone(),
    };
    entry["command"] = command.into();
    let amendment = json!({"t":t,"criterion":criterion,"old_command":old,"new_command":command,"reason":reason});
    match floor.get_mut("amendments").and_then(Value::as_array_mut) {
        Some(list) => list.push(amendment),
        None => floor["amendments"] = json!([amendment]),
    }
    tx.commit(vec![("intents.json".into(), format!("{store}\n"))])?;
    Ok(json!({"criterion":criterion,"old_command":old,"new_command":command,"amended":true}))
}

/// G32: a person's check for a repository whose checks the gate cannot discover (no root or
/// direct-subfolder Cargo.toml or package.json scripts). It joins the stored project floor, so
/// every later prompt's `required_criteria` carries it. Person-only: the guard refuses agents.
pub fn add_check(
    root: &Path,
    project: &Path,
    criterion: &str,
    command: &str,
    reason: &str,
) -> Result<Value, String> {
    let (criterion, command, reason) = (criterion.trim(), command.trim(), reason.trim());
    if criterion.is_empty() || command.is_empty() || reason.is_empty() {
        return Err("add-check needs a nonempty --criterion, --command and --reason".into());
    }
    // AUTO_* ids belong to discovery; the checkless test keys on AUTO_DIFF.
    if criterion.starts_with("AUTO_")
        || !criterion
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "{criterion}: a person's criterion id uses letters, digits, '_' or '-' and does not start with AUTO_"
        ));
    }
    let identity = project_identity(project)?;
    let tx = Transaction::open(root)?;
    if lifecycle::active_task(root)?.is_some() {
        return Err(
            "a task is active and its contract owns the floor; finish or cancel it first".into(),
        );
    }
    if floor_at(Path::new(&identity))?
        .iter()
        .any(|c| c["id"] != "AUTO_DIFF")
    {
        return Err("this repository's checks are discovered from its files; change one with `supervisor task amend-floor`".into());
    }
    let mut store = load(root)?;
    if store.get("project_floor").is_none() {
        store["project_floor"] = json!({"project":identity,"required_criteria":[]});
    }
    let floor = &mut store["project_floor"];
    validate_discovery_context(&floor["project"], &identity)?;
    let list = floor["required_criteria"]
        .as_array_mut()
        .ok_or("invalid approved project floor")?;
    if list.iter().any(|c| c["id"] == criterion) {
        return Err(format!(
            "{criterion} is already a check; change it with `supervisor task amend-floor`"
        ));
    }
    list.push(json!({"id":criterion,"check":"person_declared","command":command,"mandatory":true}));
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let addition = json!({"t":t,"criterion":criterion,"command":command,"reason":reason});
    match floor.get_mut("additions").and_then(Value::as_array_mut) {
        Some(additions) => additions.push(addition),
        None => floor["additions"] = json!([addition]),
    }
    tx.commit(vec![("intents.json".into(), format!("{store}\n"))])?;
    Ok(json!({"criterion":criterion,"command":command,"added":true}))
}

/// The mandatory checks of a task being cancelled; a tampered gate is not cancelled.
fn cancelled_floor(dir: &Path) -> Result<Vec<Value>, String> {
    let (contract, digest) = state::read_contract(dir)?;
    if let Stored::Tampered(_) = state::load_gate(dir, &digest)? {
        return Err("active task gate is tampered".into());
    }
    let contract = contract.ok_or("active contract missing")?;
    Ok(contract
        .criteria
        .iter()
        .filter(|c| c.mandatory)
        .map(|c| json!({"id":c.id.0,"check":c.check.0,"command":c.command,"mandatory":true}))
        .collect())
}

/// Retain only the approved task checks as the project floor, never a proposed replacement,
/// keeping a person's earlier amendments.
fn keep_floor(store: &mut Value, project: &Value, floor: Vec<Value>) {
    let old = store["project_floor"].take();
    store["project_floor"] = json!({"project":project,"required_criteria":floor});
    // A person's amendments and added checks (G32) keep their reasons.
    for history in ["amendments", "additions"] {
        if let Some(entries) = old.get(history) {
            store["project_floor"][history] = entries.clone();
        }
    }
}

/// A person's end to the active task without a supervised session (K17), for a prompt-managed
/// task that `task deactivate` refuses: archived as cancelled with the reason and journaled; its
/// checks are kept as the project floor, as a cancel prompt keeps them. A completed task whose
/// rollover cannot be verified would otherwise be stuck (start refuses it, a prompt cannot
/// cancel it), so a person can retire it too: it stays archived as completed, marked
/// `person_retired`. The PreToolUse guard refuses this command to agents.
pub fn cancel(root: &Path, expected: &str, reason: &str) -> Result<Value, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("cancel needs a nonempty --reason".into());
    }
    let tx = Transaction::open(root)?;
    let active = lifecycle::active_task(root)?.ok_or("no active task")?;
    if active.task_id != expected {
        return Err("active task changed".into());
    }
    let completed = gate_state(&active.dir)? == "completed";
    let disposition = if completed { "completed" } else { "cancelled" };
    let floor = cancelled_floor(&active.dir)?;
    let mut writes = Vec::new();
    if let Some(project) = read(&active.dir.join("autonomy.json"))?
        .as_ref()
        .and_then(|metadata| metadata.get("project"))
    {
        let mut store = load(root)?;
        keep_floor(&mut store, project, floor);
        writes.push((PathBuf::from("intents.json"), format!("{store}\n")));
    }
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut journal = match fs::read_to_string(active.dir.join("events.jsonl")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    journal.push_str(&format!(
        "{}\n",
        json!({"t":t,"event":"person_cancel","task_id":active.task_id,"reason":reason})
    ));
    let mut archive = json!({"schema":2,"task_id":active.task_id,"disposition":disposition,"successor":null,"reason":reason});
    if completed {
        archive["person_retired"] = true.into();
    }
    writes.extend([
        (
            PathBuf::from("tasks")
                .join(&active.task_id)
                .join("events.jsonl"),
            journal,
        ),
        (
            PathBuf::from("archives").join(format!("{}.json", active.task_id)),
            format!("{archive}\n"),
        ),
        (
            PathBuf::from("active.json"),
            "{\"schema\":2,\"task_id\":null}\n".to_owned(),
        ),
    ]);
    tx.commit(writes)?;
    Ok(json!({"inactive":true,"archived_task_id":active.task_id,"disposition":disposition}))
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
    // G5: the bound worktree is a place the task runs, not another project.
    let in_bound_worktree = metadata["worktree"]
        .as_str()
        .is_some_and(|bound| crate::location::canonical_project(project) == bound);
    if let Some(recorded) = metadata.get("project")
        && !in_bound_worktree
    {
        validate_discovery_context(recorded, &project_identity(project)?)?;
    }
    match metadata.get("npm_entrypoints") {
        Some(expected) => check_entrypoint_map(expected, Path::new(&project_identity(project)?)),
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

/// The active task and its metadata when `session` owns it.
fn owned_task(
    root: &Path,
    session: &str,
) -> Result<Option<(lifecycle::ActiveTask, Value)>, String> {
    let Some(active) = lifecycle::active_task(root)? else {
        return Ok(None);
    };
    Ok(read(&active.dir.join("autonomy.json"))?
        .filter(|metadata| metadata["session_id"] == session)
        .map(|metadata| (active, metadata)))
}

/// Brief 2026-11-09, change 1: the owning session's `SessionEnd` (`ended` carries its reason)
/// releases its unfinished task to a later session; any other hook call from that session
/// (a resume keeps the id) takes the release back. A session that never reported ending, as
/// after a crash or under an older install, stays protected. Only reads unless the mark changes.
pub fn mark_session(root: &Path, session: &str, ended: Option<&str>) -> Result<(), String> {
    let changes = |metadata: &Value| metadata.get("session_ended").is_some() != ended.is_some();
    // Unreadable state is judged where it always was; only a mark that must change can fail.
    if !owned_task(root, session)
        .ok()
        .flatten()
        .is_some_and(|(_, metadata)| changes(&metadata))
    {
        return Ok(());
    }
    let tx = Transaction::open(root)?;
    let Some((active, mut metadata)) =
        owned_task(root, session)?.filter(|(_, metadata)| changes(metadata))
    else {
        return Ok(());
    };
    let fields = metadata
        .as_object_mut()
        .ok_or("invalid autonomous metadata")?;
    match ended {
        Some(reason) => fields.insert("session_ended".into(), reason.into()),
        None => fields.remove("session_ended"),
    };
    tx.commit(vec![(
        PathBuf::from("tasks")
            .join(&active.task_id)
            .join("autonomy.json"),
        format!("{metadata}\n"),
    )])
}

/// How the prompt hook describes another session's task: `Some(true)` when that session has
/// ended, `Some(false)` when it has not reported ending, `None` when this session owns it.
pub fn foreign_owner_ended(dir: &Path, session: &str) -> Result<Option<bool>, String> {
    Ok(read(&dir.join("autonomy.json"))?
        .filter(|metadata| metadata["session_id"] != session)
        .map(|metadata| metadata.get("session_ended").is_some()))
}

/// Uncommitted work in the tree a task ran in. Anything git cannot answer counts as work.
fn uncommitted_work(tree: &Path) -> bool {
    // `status.showUntrackedFiles=no` must not hide the owner's new files.
    !crate::git::run(tree, &["status", "--porcelain", "--untracked-files=all"])
        .is_ok_and(|output| output.status.success() && output.stdout.is_empty())
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
    Ok(!invalidated_dispatches(dir)?.iter().any(|id| id == dispatch))
}

/// The dispatches a revision invalidated; none for a task that is not autonomous.
pub(crate) fn invalidated_dispatches(dir: &Path) -> Result<Vec<String>, String> {
    let Some(metadata) = read(&dir.join("autonomy.json"))? else {
        return Ok(Vec::new());
    };
    if metadata.get("schema").and_then(Value::as_u64) != Some(1)
        || !metadata["revisions"].is_array()
    {
        return Err("invalid autonomous task metadata".into());
    }
    let Some(invalidated) = metadata.get("invalidated_dispatches") else {
        return Ok(Vec::new());
    };
    invalidated
        .as_array()
        .ok_or("invalid dispatch invalidation history")?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "invalid dispatch invalidation identity".into())
        })
        .collect()
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

/// G34: the shape every refusal for a malformed contract shows, so one refusal is enough.
pub const MINIMAL_CONTRACT: &str = r#"{"id":"<short-slug>","goal":"<the request>","scope":["<path>/"],"criteria":<required_criteria>,"forbidden_effects":[],"max_repair_rounds":3}"#;

fn validate_contract(contract: &Value) -> Result<Value, String> {
    // G34 (AAP report): name every missing field at once instead of one per round trip.
    let missing: Vec<&str> = [
        "id",
        "goal",
        "scope",
        "criteria",
        "forbidden_effects",
        "max_repair_rounds",
    ]
    .into_iter()
    .filter(|field| contract.get(field).is_none())
    .collect();
    if !missing.is_empty() {
        return Err(format!(
            "contract is missing {}; a minimal contract is {MINIMAL_CONTRACT}",
            missing.join(", ")
        ));
    }
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
    // WP4: an agent may lower or add a task limit, never raise or remove one.
    if [
        (old.limits.turns, new.limits.turns),
        (old.limits.wall_clock_s, new.limits.wall_clock_s),
    ]
    .into_iter()
    .any(|(old, new)| old.is_some_and(|old| new.is_none_or(|new| new > old)))
    {
        return Err("contract revision raised or removed a task limit".into());
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
        return Err("no deterministic repository checks found (not retryable): this repository has no Cargo.toml and no package.json scripts in its root or a direct subfolder, so no proposal can start a task, whatever criteria it carries. Stop retrying and tell the user that a person declares the checks outside the session with `supervisor task add-check --project <dir> --criterion <ID> --command <command> --reason <text>`.".into());
    }
    for (required, expected) in floor.iter().zip(&required_contract.criteria) {
        if let Some(existing) = parsed.criteria.iter().find(|c| c.id == expected.id) {
            // G20: a proposal may keep any non-empty part of the floor's accepted commands.
            let kept = !existing.command.is_empty()
                && existing
                    .command
                    .iter()
                    .all(|c| expected.command.contains(c));
            if existing.check != expected.check || !kept || existing.mandatory != expected.mandatory
            {
                let amend = format!(
                    " A person can change it with `supervisor task amend-floor --criterion {} --command <command> --reason <text>` while no task is active.",
                    expected.id.0
                );
                let mut differs = Vec::new();
                if existing.command != expected.command {
                    differs.push(format!(
                        "command: floor `{}`, proposal `{}`",
                        expected.command.join("` or `"),
                        existing.command.join("` or `")
                    ));
                }
                if existing.check != expected.check {
                    differs.push(format!(
                        "check: floor `{}`, proposal `{}`",
                        expected.check.0, existing.check.0
                    ));
                }
                if existing.mandatory != expected.mandatory {
                    differs.push(format!(
                        "mandatory: floor {}, proposal {}",
                        expected.mandatory, existing.mandatory
                    ));
                }
                return Err(format!(
                    "proposal conflicts with immutable repository requirement floor: {} differs in {}. Copy the floor's values from required_criteria.{amend}",
                    expected.id.0,
                    differs.join("; "),
                ));
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
    let execution_root = crate::binding::execution_root(dir, project);
    for criterion in contract.criteria.iter().filter(|c| c.mandatory) {
        // A completed task rests on its latest receipt for the criterion: on the completed
        // candidate, and citing a project-root run of an accepted command. A dispatch journaled
        // after completion (a rerun of the check) is never judged again, so it never gets a
        // receipt and is not evidence; keying on the latest dispatch let one such rerun block
        // every rollover.
        let backing = gate.receipts.iter().rev().find_map(|receipt| {
            if receipt.criterion != criterion.id || receipt.candidate != *candidate {
                return None;
            }
            let dispatch = crate::adapter::cited_dispatch(task_id, &receipt.output_ref)?;
            rows.iter()
                .any(|row| {
                    row["kind"] == "dispatch"
                        && row["dispatch_id"].as_str() == Some(dispatch)
                        && row["cwd"].as_str() == Some(execution_root.as_str())
                        && row["task_id"] == task_id
                        && (row.get("candidate").is_some_and(|c| {
                            c["base_commit"] == candidate.base_commit
                                && c["dirty_digest"] == candidate.dirty_digest
                        }) || crate::observation::transferred(
                            &rows, task_id, dispatch, candidate,
                        ))
                        && row
                            .get("command")
                            .and_then(Value::as_str)
                            .is_some_and(|command| criterion.accepts(command))
                })
                .then_some((receipt, dispatch))
        });
        let (receipt, dispatch) =
            backing.ok_or("completed requirement has no observed dispatch")?;
        if !dispatch_is_current(dir, dispatch)? {
            return Err("completed requirement dispatch predates a contract revision".into());
        }
        if !(receipt.exit_code == Some(0)
            && crate::adapter::receipt_is_observed(dir, task_id, receipt))
        {
            return Err(
                "completed requirement lacks its latest successful hook-observed receipt".into(),
            );
        }
    }
    Ok(())
}

/// The person-only command that ends a task no call from this prompt can end.
pub fn person_cancel(event: &Value, task_id: &str) -> String {
    let project = event["project"].as_str().unwrap_or("<project>");
    format!(
        "a person can end it with: supervisor task cancel --project \"{project}\" --task {task_id} --reason \"<why>\""
    )
}

/// G5: the validated linked worktree a `start` or `revise` binds its task to, if it asks for one.
fn requested_worktree(project: &Path, proposal: &Value) -> Result<Option<String>, String> {
    match proposal.get("worktree").and_then(Value::as_str) {
        Some(requested) => crate::binding::resolve(project, requested),
        None => Ok(None),
    }
}

/// Apply once under the existing OS lock and write-ahead transaction. Replayed applications
/// return their historical response and never reactivate or rebind an archived task.
pub fn propose(root: &Path, project: &Path, proposal: &Value) -> Result<Value, String> {
    let resolved;
    let proposal = match proposal.get("contract_file") {
        None => proposal,
        Some(file) => {
            resolved = with_contract_file(project, proposal, file)?;
            &resolved
        }
    };
    let id = text(proposal, "event_id")?;
    let identity = project_identity(project)?;
    // Resolving a requested worktree runs git too; its error still surfaces only where used.
    let worktree = requested_worktree(project, proposal);
    // A takeover `start` asks git once whether the ended owner left uncommitted work; outside
    // the lock, like the calls above (G26).
    // The tree is the task's own: its bound worktree, else the project it was started in. The
    // answer is kept with the task it was asked about.
    let uncommitted = match lifecycle::active_task(root)? {
        Some(active) if proposal.get("action").and_then(Value::as_str) == Some("start") => {
            read(&active.dir.join("autonomy.json"))?
                .filter(|metadata| metadata.get("session_ended").is_some())
                .map(|metadata| {
                    let tree = metadata["worktree"]
                        .as_str()
                        .or(metadata["project"].as_str())
                        .map_or(project, Path::new);
                    (active.task_id, uncommitted_work(tree))
                })
        }
        _ => None,
    };
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
    validate_discovery_context(&event["project"], &identity)?;
    let digest = crate::sha256::hex(crate::chain::canonical(proposal).as_bytes());
    // A discussion changed no task, so the same prompt may still become work; the replaced
    // discussion's digest stays in the record. Work is never turned back into a discussion.
    let replaced_discussion = event
        .get("applied")
        .filter(|a| a["response"]["action"] == "discussion")
        .filter(|_| proposal.get("action").and_then(Value::as_str) != Some("discussion"))
        .map(|a| a["digest"].clone());
    if let Some(applied) = event
        .get("applied")
        .filter(|_| replaced_discussion.is_none())
    {
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
    if !matches!(
        action,
        "discussion" | "start" | "continue" | "revise" | "pause" | "resume" | "cancel"
    ) {
        return Err(format!(
            "unknown action {action:?}; expected one of discussion|start|continue|revise|pause|resume|cancel"
        ));
    }
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
        // Brief 2026-11-09, change 1: an unfinished task whose owning session has ended is
        // archived as `abandoned` by the session that starts the next one. With uncommitted
        // work in its tree the user chooses first (`abandon_previous`). A session that has not
        // reported ending keeps its task.
        let mut abandoned = None;
        if let Some(active) = &active
            && gate_state(&active.dir)? != "completed"
        {
            let metadata = read(&active.dir.join("autonomy.json"))?.unwrap_or(Value::Null);
            let previous = metadata["session_id"].as_str().filter(|s| *s != session);
            let Some(previous) = previous else {
                return Err(format!(
                    "unfinished task retained; continue it or explicitly cancel before a new task; {}",
                    person_cancel(&event, &active.task_id)
                ));
            };
            let Some(reason) = metadata.get("session_ended") else {
                return Err(format!(
                    "unfinished task retained: it belongs to another session that has not ended, so it stays protected; a direct resume prompt from the user takes it over, or, only if that session is closed, {}",
                    person_cancel(&event, &active.task_id)
                ));
            };
            // An answer git gave before the lock, for this task; a mark set since then, or
            // another task, counts as work. shortcut: an owner that came back, edited and
            // ended again while this call waited for the lock (10 s at most) is not seen;
            // ask git under the lock if that ever matters.
            let work = uncommitted != Some((active.task_id.clone(), false));
            if work && proposal.get("abandon_previous") != Some(&Value::Bool(true)) {
                return Err(format!(
                    "unfinished task {} belonged to a session that has ended, and its working tree has uncommitted changes that may be its unfinished work. Ask the user to choose: a direct resume prompt takes that task over with the work, or repeat this start with \"abandon_previous\":true to archive it as abandoned (the files stay as they are)",
                    active.task_id
                ));
            }
            abandoned = Some(
                json!({"previous_session":previous,"abandoned_by_session":session,
                "reason":format!("owning session ended ({})", reason.as_str().unwrap_or("unknown")),
                "uncommitted_work":work}),
            );
        }
        // G22: the rollover never dead-ends. A completion that cannot be re-verified is
        // archived as `completed-unverified` with the reason; only a verified one is `completed`.
        let mut unverified = None;
        if let Some(active) = &active
            && abandoned.is_none()
            && read(&active.dir.join("autonomy.json"))?.is_some_and(|m| m["source"] == "hook")
            && let Err(why) = validate_entrypoints(&active.dir, project)
                .and_then(|()| observed_completion(&active.dir, &active.task_id, project))
        {
            unverified = Some(why);
        }
        // WP4: a person's task (no autonomy.json) completed without observed evidence, e.g. on
        // caller-asserted MCP receipts, is not replaced by an agent's start, which would shed the
        // limits the person set; the person moves on with `task next`.
        if let Some(active) = &active
            && read(&active.dir.join("autonomy.json"))?.is_none()
            && observed_completion(&active.dir, &active.task_id, project).is_err()
        {
            return Err(format!(
                "the active task was started by a person and its completion rests on no hook-observed evidence, so an agent cannot replace it; a person runs `supervisor task next --task {} --contract <file>`",
                active.task_id
            ));
        }
        let mut contract = add_floor(
            proposal.get("contract").ok_or("start needs a contract")?,
            &event["required_criteria"],
        )?;
        contract["event_id"] = format!("autonomy-{id}").into();
        // The lifecycle knows an unfinished predecessor only as `cancelled`; the archive record
        // is corrected to `abandoned` below.
        let (mut response, task_writes) = lifecycle::plan_start(
            root,
            &state::Writer::agent(project),
            &contract,
            active.is_some(),
            abandoned.as_ref().map(|_| "cancelled"),
            current_id,
        )?;
        writes.extend(task_writes);
        let correction = match (unverified, abandoned) {
            (Some(why), _) => Some(json!({"disposition":"completed-unverified","reason":why})),
            (None, Some(mut record)) => {
                record["disposition"] = "abandoned".into();
                Some(record)
            }
            (None, None) => None,
        };
        if let (Some(correction), Some(active)) = (correction, &active) {
            let path = PathBuf::from("archives").join(format!("{}.json", active.task_id));
            let archive = writes
                .iter_mut()
                .find(|(p, _)| *p == path)
                .ok_or("rollover wrote no archive for the previous task")?;
            let mut record: Value = serde_json::from_str(&archive.1).map_err(|e| e.to_string())?;
            for (key, value) in correction.as_object().into_iter().flatten() {
                record[key] = value.clone();
            }
            archive.1 = format!("{record}\n");
        }
        let task_id = text(&response, "task_id")?;
        let npm_entrypoints = selected_entrypoints(
            &contract,
            event.get("npm_definitions").unwrap_or(&json!({})),
        )?;
        check_entrypoint_map(&npm_entrypoints, Path::new(&identity))?;
        let mut metadata = json!({"schema":1,"source":event["source"],"project":event["project"],"session_id":session,"initial_event":id,"requirement_floor":contract,"npm_entrypoints":npm_entrypoints,"paused":false,"revisions":[{"event_id":id,"action":"start","contract":contract}]});
        if let Some(worktree) = worktree.clone()? {
            metadata["worktree"] = worktree.into();
        }
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
        let existing = read(&active.dir.join("autonomy.json"))?;
        // A task started with `supervisor task start` becomes prompt-managed here; say so.
        let converted = existing.is_none() && action != "cancel";
        let mut metadata = existing.unwrap_or_else(|| json!({"schema":1,"source":event["source"],"session_id":session,"paused":false,"revisions":[]}));
        if action != "cancel"
            && let Some(recorded) = metadata.get("project")
        {
            validate_discovery_context(recorded, &identity)?;
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
            return Err(if metadata.get("session_ended").is_some() {
                "proposal belongs to another task session, which has ended: a start with a new contract archives its task as abandoned, or a direct resume prompt from the user takes it over".to_owned()
            } else {
                format!(
                    "proposal belongs to another task session that has not ended, so its task stays protected; a direct resume prompt from the user takes it over, or, only if that session is closed, {}",
                    person_cancel(&event, &active.task_id)
                )
            });
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
                .map(|recorded| legacy_subdirectory_context(recorded, &identity))
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
            // The new owner is live; the previous owner's end no longer releases the task.
            if let Some(fields) = metadata.as_object_mut() {
                fields.remove("session_ended");
            }
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
                let floor = cancelled_floor(&active.dir)?;
                keep_floor(
                    &mut store,
                    metadata.get("project").unwrap_or(&event["project"]),
                    floor,
                );
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
                // A revise naming the project itself moves the task back: the binding is cleared.
                match worktree.clone()? {
                    Some(worktree) => metadata["worktree"] = worktree.into(),
                    None if proposal.get("worktree").is_some() => {
                        if let Some(fields) = metadata.as_object_mut() {
                            fields.remove("worktree");
                        }
                    }
                    None => {}
                }
                preserves(&old, &new, event["source"] == "hook")?;
                if let Some(floor) = metadata.get("requirement_floor") {
                    preserves(floor, &new, event["source"] == "hook")?;
                }
                // The agent cannot assert these: a revision keeps the task's approval (WP4 counts
                // wall clock from approval.at), isolation and base. What is saved is checked.
                let mut stored = new.clone();
                if let Some(fields) = stored.as_object_mut() {
                    fields.insert("schema_version".to_owned(), 2.into());
                    for key in ["approval", "isolation", "base_sha"] {
                        match old.get(key) {
                            Some(kept) => fields.insert(key.to_owned(), kept.clone()),
                            None => fields.remove(key),
                        };
                    }
                }
                state::contract_from(&stored)?;
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
                atomic_write(
                    &stage.join("contract.json"),
                    format!("{stored}\n").as_bytes(),
                )?;
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
        let mut response = json!({"action":action,"task_id":active.task_id,"state":task_state,"paused":metadata["paused"],"inactive":action=="cancel"});
        if converted {
            response["converted_to_autonomous"] = true.into();
            response["note"] = "this operator-started task is now prompt-managed: Stop judges it without a completion claim, a failing check spends a repair round, and only a cancel prompt ends it".into();
        }
        response
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
    if let Some(old) = replaced_discussion {
        store["events"][id]["applied"]["replaced_discussion"] = old;
    }
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
