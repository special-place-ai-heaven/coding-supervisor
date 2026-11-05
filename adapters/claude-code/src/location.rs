//! Where the gate keeps its state: outside the project tree when the environment allows.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve filesystem aliases and nested directories to the physical worktree, not its common git directory.
/// A git that does not answer in time is treated as no repository; see [`checked_project`].
pub fn canonical_project(cwd: &Path) -> String {
    normalize(git_root(cwd).ok().flatten().as_deref().unwrap_or(cwd))
}

/// The main checkout of a linked worktree, read from the worktree's `.git` file
/// (`gitdir: <main>/.git/worktrees/<name>`). `None` for a main checkout, a submodule or no
/// repository. File reads only, no git process.
pub fn linked_main_checkout(worktree: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(worktree.join(".git")).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        worktree.join(gitdir)
    };
    let worktrees = gitdir.parent()?;
    let common = worktrees.parent()?;
    if worktrees.file_name()? != "worktrees" || common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(Path::to_path_buf)
}

/// [`canonical_project`], or an error when git does not answer in time, for callers whose
/// state namespace or gate decision must not silently follow a different identity.
pub(crate) fn checked_project(cwd: &Path) -> Result<String, String> {
    Ok(normalize(git_root(cwd)?.as_deref().unwrap_or(cwd)))
}

/// Each directory's worktree root, asked of git once per process: a Stop used to start the same
/// `rev-parse --show-toplevel` 16 times. Keyed by the normalized directory, so every spelling of
/// one directory shares an entry. Only roots found are kept, so a directory that becomes a
/// repository later is still seen.
static ROOTS: std::sync::Mutex<Vec<(String, PathBuf)>> = std::sync::Mutex::new(Vec::new());

fn git_root(cwd: &Path) -> Result<Option<PathBuf>, String> {
    let key = normalize(cwd);
    if let Some((_, root)) = ROOTS
        .lock()
        .ok()
        .and_then(|roots| roots.iter().find(|(dir, _)| *dir == key).cloned())
        && root_still_holds(&key, &root)
    {
        return Ok(Some(root));
    }
    match crate::git::run(cwd, &["rev-parse", "--show-toplevel"]) {
        Ok(out) if out.status.success() => {
            let root = String::from_utf8(out.stdout)
                .ok()
                .map(|s| PathBuf::from(s.trim()));
            // The root is its own root too: later lookups often start from it.
            if let (Some(root), Ok(mut roots)) = (&root, ROOTS.lock()) {
                roots.push((key, root.clone()));
                roots.push((normalize(root), root.clone()));
            }
            Ok(root)
        }
        Ok(_) | Err(crate::git::Failure::Unavailable(_)) => Ok(None),
        Err(crate::git::Failure::TimedOut(why)) => Err(why),
    }
}

/// A remembered root still answers for `dir` when the root keeps its `.git` and no directory
/// between them gained one: a long-lived process (the MCP server) sees a subdirectory turned
/// into its own checkout. File lookups only, no git process.
fn root_still_holds(dir: &str, root: &Path) -> bool {
    let root_key = normalize(root);
    let mut current = Some(Path::new(dir));
    while let Some(path) = current {
        if path.to_string_lossy() == root_key {
            return path.join(".git").exists();
        }
        if path.join(".git").exists() {
            return false;
        }
        current = path.parent();
    }
    false
}

fn normalize(path: &Path) -> String {
    let absolute = fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    let text = absolute.to_string_lossy().replace('\\', "/");
    let text = text
        .strip_prefix("//?/")
        .unwrap_or(&text)
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

pub fn legacy_project_key(cwd: &Path) -> String {
    std::path::absolute(cwd)
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn project_key(cwd: &Path) -> String {
    let identity = canonical_project(cwd);
    let slug: String = identity
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(72)
        .collect();
    format!("{slug}-{}", crate::sha256::hex(identity.as_bytes()))
}

/// The state directory and the rule that chose it: `env`, `local-app-data`, `xdg`, `home`
/// or `cwd-fallback`. An unavailable configured directory never selects a different namespace;
/// production callers use `resolve_state_dir_checked` to surface creation/discovery errors.
pub fn resolve_state_dir(
    cwd: &Path,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> (PathBuf, &'static str) {
    let selected = select_state_dir(cwd, env_lookup);
    // Compatibility discovery API: creation errors are checked by prepare_state, never a reason
    // to select a different namespace and silently leave an active gate behind.
    let _ = fs::create_dir_all(&selected.0);
    selected
}

/// The state directory [`resolve_state_dir`] picks, without creating it.
fn select_state_dir(
    cwd: &Path,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> (PathBuf, &'static str) {
    let var = |name: &str| env_lookup(name).filter(|v| !v.is_empty());
    let key = project_key(cwd);
    let chosen = match var("SUPERVISOR_STATE_DIR") {
        Some(dir) => Some((PathBuf::from(dir).join(&key), "env")),
        None => {
            let local_app_data = if cfg!(windows) {
                var("LOCALAPPDATA").map(|b| (PathBuf::from(b), "local-app-data"))
            } else {
                None
            };
            local_app_data
                .or_else(|| var("XDG_STATE_HOME").map(|b| (PathBuf::from(b), "xdg")))
                .or_else(|| {
                    var("HOME").map(|b| (PathBuf::from(b).join(".local").join("state"), "home"))
                })
                .map(|(base, label)| {
                    (
                        base.join("coding-supervisor").join("state").join(&key),
                        label,
                    )
                })
        }
    };
    chosen.unwrap_or_else(|| {
        (
            PathBuf::from(canonical_project(cwd)).join(".supervisor"),
            "cwd-fallback",
        )
    })
}

/// K9: when `SUPERVISOR_STATE_DIR` chose the state directory, the one the default rule would
/// pick, if it exists. A hook, MCP server or command started without that variable uses it,
/// and so sees a different task namespace for the same project.
pub fn shadow_state_dir(
    cwd: &Path,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    env_lookup("SUPERVISOR_STATE_DIR").filter(|v| !v.is_empty())?;
    let without = |name: &str| {
        if name == "SUPERVISOR_STATE_DIR" {
            None
        } else {
            env_lookup(name)
        }
    };
    let (dir, rule) = select_state_dir(cwd, &without);
    (rule != "cwd-fallback" && dir.is_dir()).then_some(dir)
}

/// Checked initialization for production callers. No configured namespace silently falls back.
pub fn resolve_state_dir_checked(
    cwd: &Path,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<(PathBuf, &'static str), String> {
    let selected = resolve_state_dir(cwd, env_lookup);
    prepare_state(&selected.0, cwd)?;
    Ok(selected)
}

/// [`resolve_state_dir_checked`] for a reader that writes nothing: the worktree identity is
/// checked when recorded, and no state is created, migrated or recovered.
pub fn resolve_state_dir_read_only(
    cwd: &Path,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<(PathBuf, &'static str), String> {
    let selected = select_state_dir(cwd, env_lookup);
    match fs::read_to_string(selected.0.join("project.json")) {
        Ok(text) => same_worktree(&text, &checked_project(cwd)?)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(selected)
}

/// project.json `text` names the worktree `identity`.
fn same_worktree(text: &str, identity: &str) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "invalid project identity".to_owned())?;
    if v.get("canonical_worktree")
        .and_then(serde_json::Value::as_str)
        != Some(identity)
    {
        return Err("state directory belongs to a different worktree".to_owned());
    }
    Ok(())
}

pub fn prepare_state(root: &Path, cwd: &Path) -> Result<(), String> {
    let identity = checked_project(cwd)?;
    let tx = crate::transaction::Transaction::open(root)?;
    let project = root.join("project.json");
    match fs::read_to_string(&project) {
        Ok(text) => same_worktree(&text, &identity)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::transaction::atomic_write(
                &project,
                format!(
                    "{}\n",
                    serde_json::json!({"schema":2,"canonical_worktree":identity})
                )
                .as_bytes(),
            )?;
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut legacy = Vec::new();
    if let Some(parent) = root.parent() {
        for path in [cwd.to_path_buf(), PathBuf::from(&identity)] {
            let old = parent.join(legacy_project_key(&path));
            if old != root && old.join("contract.json").exists() && !legacy.contains(&old) {
                legacy.push(old);
            }
        }
    }
    // The cwd-local fallback is legacy state even when it is still the selected root.
    let fallback = PathBuf::from(&identity).join(".supervisor");
    if fallback.join("contract.json").exists() && !legacy.contains(&fallback) {
        legacy.push(fallback);
    }
    if !legacy.is_empty() && !root.join("active.json").exists() {
        tx.commit(vec![(
            "legacy-location.json".into(),
            format!(
                "{}\n",
                serde_json::json!({"legacy_directories":legacy,"migration_required":true})
            ),
        )])?;
    }
    Ok(())
}
