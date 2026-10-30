//! PreToolUse path rule: agents may not write the gate's own state. Evaluated before
//! anything else and independent of gate state and contract.

use std::path::Path;

use serde_json::Value;

use crate::adapter::Decision;

fn normalized(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

/// Terminal Commander input fields that can carry a command, a working directory or a path.
const TC_TEXT_FIELDS: [&str; 11] = [
    "argv",
    "command",
    "script",
    "shell_line",
    "cwd",
    "path",
    "file_path",
    "root",
    "dir",
    "target",
    "line",
];

/// The `.supervisor` marker or the state directory (either separator form) inside lowercased
/// command text; returns what matched.
fn text_reference(text: &str, dir: &str) -> Option<String> {
    if text.contains(".supervisor") {
        return Some(".supervisor".to_owned());
    }
    [dir.to_owned(), dir.replace('/', "\\")]
        .into_iter()
        .find(|d| !d.is_empty() && text.contains(d.as_str()))
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
            text_reference(&command, dir)
        }
        // Terminal Commander runs commands and reads or writes files through the daemon;
        // every field that can name a command, a working directory or a path is scanned.
        // The guard matches on tool name and field text: a connector tool outside this
        // prefix, or a field not listed here, is not covered.
        t if t.starts_with("mcp__terminal_commander__") => {
            let mut text = String::new();
            for field in TC_TEXT_FIELDS {
                match input.get(field) {
                    Some(Value::String(s)) => {
                        text.push_str(s);
                        text.push(' ');
                    }
                    Some(Value::Array(items)) => {
                        for s in items.iter().filter_map(Value::as_str) {
                            text.push_str(s);
                            text.push(' ');
                        }
                    }
                    _ => {}
                }
            }
            text_reference(&text.to_lowercase(), dir)
        }
        _ => None,
    }?;
    Some(Decision::Deny(format!(
        "GATE: {tool} refused: the gate's own state is not writable by agents ({matched})"
    )))
}
