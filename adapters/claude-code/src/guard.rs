//! PreToolUse path rule: agents may not write the gate's own state. Evaluated before
//! anything else and independent of gate state and contract.

use std::path::Path;

use serde_json::Value;

use crate::adapter::Decision;
use crate::level::Level;

fn normalized(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

/// Terminal Commander input fields that can carry a command, a working directory or a path.
const TC_TEXT_FIELDS: [&str; 9] = [
    "argv",
    "command",
    "script",
    "cwd",
    "path",
    "file_path",
    "root",
    "dir",
    "target",
];

/// The `.supervisor` marker (when `marker`) or the directory (either separator form) inside
/// lowercased command text; returns what matched.
fn text_reference(text: &str, dir: &str, marker: bool) -> Option<String> {
    if marker && text.contains(".supervisor") {
        return Some(".supervisor".to_owned());
    }
    [dir.to_owned(), dir.replace('/', "\\")]
        .into_iter()
        .find(|d| !d.is_empty() && text.contains(d.as_str()))
}

/// A deny decision when the tool call targets `.supervisor` or the resolved state dir.
pub fn state_write_denial(payload: &Value, state_dir: &Path) -> Option<Decision> {
    deny(payload, reference(payload, state_dir, true)?)
}

/// [`state_write_denial`], plus the user config dir that holds the global level file, plus a
/// command that sets a level (`supervisor level <word>`). Reading the level and `supervisor
/// status` stay allowed. A user's slash command does not pass through PreToolUse.
pub fn gate_write_denial(
    payload: &Value,
    state_dir: &Path,
    config_dir: Option<&Path>,
) -> Option<Decision> {
    state_write_denial(payload, state_dir).or_else(|| {
        let matched = config_dir
            .and_then(|dir| reference(payload, dir, false))
            .or_else(|| level_command(payload))?;
        deny(payload, matched)
    })
}

fn deny(payload: &Value, matched: String) -> Option<Decision> {
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    Some(Decision::Deny(format!(
        "GATE: {tool} refused: the gate's own state is not writable by agents ({matched})"
    )))
}

/// `supervisor level <off|lite|full|ultra>` in a Bash, PowerShell or Terminal Commander
/// command, whatever the binary's path, extension or quoting; returns the matched words.
fn level_command(payload: &Value) -> Option<String> {
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    let text = command_text(tool, payload.get("tool_input")?)?.to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "\"'`;&|()".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    words.windows(3).find_map(|w| {
        let program = w[0].rsplit(['/', '\\']).next().unwrap_or_default();
        let program = program.strip_suffix(".exe").unwrap_or(program);
        (program.ends_with("supervisor") && w[1] == "level" && Level::parse(w[2]).is_some())
            .then(|| format!("supervisor level {}", w[2]))
    })
}

/// The text of a shell or Terminal Commander call: the command, or every scanned TC field.
fn command_text(tool: &str, input: &Value) -> Option<String> {
    if matches!(tool, "Bash" | "PowerShell") {
        return input
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if !tool.starts_with("mcp__terminal_commander__") {
        return None;
    }
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
    Some(text)
}

/// What in the tool call names `dir` (or, when `marker`, any `.supervisor` directory).
fn reference(payload: &Value, dir: &Path, marker: bool) -> Option<String> {
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    let input = payload.get("tool_input")?;
    let dir = normalized(&dir.to_string_lossy());
    let dir = dir.trim_end_matches('/');
    match tool {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            // NotebookEdit names its target `notebook_path`.
            let path = input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
                .and_then(Value::as_str)?;
            let p = normalized(path);
            let rooted = format!("/{p}");
            let in_dir = !dir.is_empty() && (p == dir || p.starts_with(&format!("{dir}/")));
            let marked = rooted.contains("/.supervisor/") || rooted.ends_with("/.supervisor");
            (marker && marked || in_dir).then(|| path.to_owned())
        }
        // Terminal Commander runs commands and reads or writes files through the daemon;
        // every field that can name a command, a working directory or a path is scanned.
        // The guard matches on tool name and field text: a connector tool outside this
        // prefix, or a field not listed here, is not covered.
        t => text_reference(&command_text(t, input)?.to_lowercase(), dir, marker),
    }
}
