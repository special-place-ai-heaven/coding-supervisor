//! PreToolUse path rule: agents may not write the gate's own state. Evaluated before
//! anything else and independent of gate state and contract.

use std::path::Path;

use serde_json::Value;

use crate::adapter::Decision;

fn normalized(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

/// A deny decision when the tool call targets `.supervisor` or the resolved state dir.
pub fn state_write_denial(payload: &Value, state_dir: &Path) -> Option<Decision> {
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    let input = payload.get("tool_input")?;
    let dir = normalized(&state_dir.to_string_lossy());
    let dir = dir.trim_end_matches('/');
    let matched = match tool {
        "Write" | "Edit" | "NotebookEdit" => {
            // NotebookEdit names its target `notebook_path`.
            let path = input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
                .and_then(Value::as_str)?;
            let p = normalized(path);
            let rooted = format!("/{p}");
            let in_dir = !dir.is_empty() && (p == dir || p.starts_with(&format!("{dir}/")));
            (rooted.contains("/.supervisor/") || rooted.ends_with("/.supervisor") || in_dir)
                .then(|| path.to_owned())
        }
        "Bash" | "PowerShell" => {
            let command = input.get("command").and_then(Value::as_str)?.to_lowercase();
            if command.contains(".supervisor") {
                Some(".supervisor".to_owned())
            } else {
                [dir.to_owned(), dir.replace('/', "\\")]
                    .into_iter()
                    .find(|d| !d.is_empty() && command.contains(d.as_str()))
            }
        }
        _ => None,
    }?;
    Some(Decision::Deny(format!(
        "GATE: {tool} refused: the gate's own state is not writable by agents ({matched})"
    )))
}
