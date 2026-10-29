//! Where the gate keeps its state: outside the project tree when the environment allows.

use std::fs;
use std::path::{Path, PathBuf};

/// The absolute cwd with every character that is not ASCII alphanumeric replaced by `-`
/// (`E:\project\x` becomes `E--project-x`).
pub fn project_key(cwd: &Path) -> String {
    let absolute = std::path::absolute(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    absolute
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The state directory and the rule that chose it: `env`, `local-app-data`, `xdg`, `home`
/// or `cwd-fallback`. The chosen directory is created; if that fails, the cwd fallback wins.
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
    match chosen {
        Some((dir, label)) if fs::create_dir_all(&dir).is_ok() => (dir, label),
        _ => (cwd.join(".supervisor"), "cwd-fallback"),
    }
}
