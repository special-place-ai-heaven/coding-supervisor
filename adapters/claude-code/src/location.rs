//! Where the gate keeps its state: outside the project tree when the environment allows.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve filesystem aliases and nested directories to the physical worktree, not its common git directory.
pub fn canonical_project(cwd: &Path) -> String {
    let mut command = std::process::Command::new("git");
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let git_root = command
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()));
    let path = git_root.as_deref().unwrap_or(cwd);
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
    let selected = chosen.unwrap_or_else(|| {
        (
            PathBuf::from(canonical_project(cwd)).join(".supervisor"),
            "cwd-fallback",
        )
    });
    // Compatibility discovery API: creation errors are checked by prepare_state, never a reason
    // to select a different namespace and silently leave an active gate behind.
    let _ = fs::create_dir_all(&selected.0);
    selected
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

pub fn prepare_state(root: &Path, cwd: &Path) -> Result<(), String> {
    let tx = crate::transaction::Transaction::open(root)?;
    let identity = canonical_project(cwd);
    let project = root.join("project.json");
    match fs::read_to_string(&project) {
        Ok(text) => {
            let v: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| "invalid project identity".to_owned())?;
            if v.get("canonical_worktree")
                .and_then(serde_json::Value::as_str)
                != Some(identity.as_str())
            {
                return Err("state directory belongs to a different worktree".to_owned());
            }
        }
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
