//! Narrow, configured PreToolUse policy. This is not a shell interpreter or sandbox.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::adapter::Decision;

struct Policy {
    paths: Vec<PathBuf>,
    branches: Vec<String>,
    markers: Vec<String>,
    tools: Vec<String>,
    prefixes: Vec<Vec<String>>,
}

fn strings(value: Option<&Value>) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| "invalid policy list".to_owned())
            })
            .collect(),
        _ => Err("invalid policy list".to_owned()),
    }
}

fn read(root: &Path) -> Result<Option<Policy>, String> {
    let text = match fs::read_to_string(root.join("policy.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("configured action policy could not be read".to_owned()),
    };
    // Never include parser errors or field values: a misconfigured file may contain secrets.
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| "configured action policy is invalid".to_owned())?;
    let object = value.as_object().ok_or("invalid policy object")?;
    let fields = [
        "version",
        "protected_paths",
        "protected_branches",
        "synthetic_secret_markers",
        "denied_tools",
        "denied_argv_prefixes",
    ];
    if value["version"] != 1 || object.keys().any(|k| !fields.contains(&k.as_str())) {
        return Err("unsupported action policy schema".to_owned());
    }
    let paths = strings(value.get("protected_paths"))?
        .into_iter()
        .map(|s| {
            let path = PathBuf::from(s);
            if !path.is_absolute() {
                return Err("policy protected paths must be absolute".to_owned());
            }
            Ok(normal_path(&path))
        })
        .collect::<Result<_, _>>()?;
    let prefixes = match value.get("denied_argv_prefixes") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                let args = strings(Some(v))?;
                if args.is_empty() {
                    Err("empty denied argv prefix".to_owned())
                } else {
                    Ok(args)
                }
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("invalid denied argv prefixes".to_owned()),
    };
    Ok(Some(Policy {
        paths,
        branches: strings(value.get("protected_branches"))?,
        markers: strings(value.get("synthetic_secret_markers"))?,
        tools: strings(value.get("denied_tools"))?,
        prefixes,
    }))
}

fn normal_path(path: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn overlaps(a: &Path, b: &Path) -> bool {
    let a = a.to_string_lossy().replace('\\', "/");
    let b = b.to_string_lossy().replace('\\', "/");
    let (a, b) = if cfg!(windows) {
        (a.to_lowercase(), b.to_lowercase())
    } else {
        (a, b)
    };
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

/// Recognize one simple command with whitespace and paired quotes. Compound commands,
/// expansions, escapes and scripts are deliberately unsupported rather than guessed.
fn words(text: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for ch in text.chars() {
        if matches!(ch, ';' | '|' | '&' | '$' | '`' | '\n' | '\r' | '<' | '>') {
            return None;
        }
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), c) if q == c => quote = None,
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    result.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(ch),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !word.is_empty() {
        result.push(word);
    }
    Some(result)
}

fn command(tool: &str, input: &Value) -> Option<Vec<String>> {
    match tool {
        "Bash" | "PowerShell" => input.get("command").and_then(Value::as_str).and_then(words),
        "mcp__terminal_commander__shell_exec" => input
            .get("shell_line")
            .and_then(Value::as_str)
            .and_then(words),
        "mcp__terminal_commander__shell_session_exec" => {
            input.get("line").and_then(Value::as_str).and_then(words)
        }
        "mcp__terminal_commander__run_and_watch"
        | "mcp__terminal_commander__command_start_combed"
        | "mcp__terminal_commander__pty_command_start" => {
            strings(input.get("argv")).ok().filter(|v| !v.is_empty())
        }
        _ => None,
    }
}

fn program(arg: &str) -> String {
    arg.rsplit(['/', '\\'])
        .next()
        .unwrap_or(arg)
        .trim_end_matches(".exe")
        .to_lowercase()
}

fn command_reason(
    args: &[String],
    cwd: &Path,
    policy: &Policy,
    cwd_known: bool,
) -> Option<&'static str> {
    if policy
        .prefixes
        .iter()
        .any(|prefix| args.starts_with(prefix))
    {
        return Some("configured external command");
    }
    let executable = program(args.first()?);
    if matches!(
        executable.as_str(),
        "rm" | "rmdir" | "del" | "erase" | "remove-item" | "rd"
    ) {
        for arg in args
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with('-') && !matches!(a.as_str(), "/s" | "/q"))
        {
            let path = Path::new(arg);
            if !cwd_known && !path.is_absolute() && !policy.paths.is_empty() {
                return Some("relative deletion with unknown working directory");
            }
            let target = normal_path(&if path.is_absolute() {
                path.to_owned()
            } else {
                cwd.join(path)
            });
            if policy.paths.iter().any(|p| overlaps(&target, p)) {
                return Some("configured protected-path deletion");
            }
        }
    }
    if executable == "git" {
        let rest = &args[1..];
        let force = rest.iter().any(|a| {
            matches!(
                a.as_str(),
                "--force" | "-f" | "--force-with-lease" | "-D" | "--delete"
            )
        });
        if matches!(rest.first().map(String::as_str), Some("push" | "branch"))
            && force
            && rest.iter().any(|a| {
                policy.branches.iter().any(|b| {
                    a == b || a.ends_with(&format!(":{b}")) || a == &format!("refs/heads/{b}")
                })
            })
        {
            return Some("configured protected-branch rewrite");
        }
        if rest.first().is_some_and(|a| a == "reset" || a == "rebase") {
            if !cwd_known && !policy.branches.is_empty() {
                return Some("protected-branch context unavailable");
            }
            let mut git = Command::new("git");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                git.creation_flags(0x08000000);
            }
            let current = git
                .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
                .current_dir(cwd)
                .output()
                .ok();
            if current
                .as_ref()
                .filter(|o| o.status.success())
                .and_then(|o| std::str::from_utf8(&o.stdout).ok())
                .is_some_and(|b| policy.branches.iter().any(|p| p == b.trim()))
            {
                return Some("configured protected-branch rewrite");
            }
        }
    }
    None
}

/// No policy means silent allowance. Invalid configured policy is an error, which the
/// hook must deny without printing the policy or the proposed tool arguments.
pub fn action_policy_denial(
    payload: &Value,
    project: &Path,
    root: &Path,
) -> Result<Option<Decision>, String> {
    let Some(policy) = read(root)? else {
        return Ok(None);
    };
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    let is_tc = tool.starts_with("mcp__terminal_commander__");
    let session_exec = tool == "mcp__terminal_commander__shell_session_exec";
    let cwd_value = input.get("cwd").and_then(Value::as_str).map(Path::new);
    let cwd_known = !is_tc || (!session_exec && cwd_value.is_some_and(Path::is_absolute));
    let reason = if policy.tools.iter().any(|t| t == tool) {
        Some("configured external tool")
    } else if policy.markers.iter().any(|m| input.to_string().contains(m)) {
        // Fixtures only; real secrets must never be copied into policy configuration.
        Some("configured synthetic secret marker")
    } else if tool == "Bash" || tool == "PowerShell" || is_tc {
        let cwd = cwd_value.unwrap_or(project);
        let cwd = if cwd.is_absolute() {
            cwd.to_owned()
        } else {
            project.join(cwd)
        };
        if let Some(args) = command(tool, input) {
            command_reason(&args, &cwd, &policy, cwd_known)
        } else if tool.starts_with("mcp__terminal_commander__file_")
            && (tool.ends_with("delete") || tool.ends_with("remove"))
        {
            input
                .get("path")
                .and_then(Value::as_str)
                .map(Path::new)
                .and_then(|path| {
                    if !cwd_known && !path.is_absolute() && !policy.paths.is_empty() {
                        Some("relative deletion with unknown working directory")
                    } else {
                        let target = normal_path(&if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            cwd.join(path)
                        });
                        policy
                            .paths
                            .iter()
                            .any(|protected| overlaps(&target, protected))
                            .then_some("configured protected-path deletion")
                    }
                })
        } else {
            None
        }
    } else {
        None
    };
    Ok(reason.map(|reason| Decision::Deny(format!("GATE: action refused: {reason}"))))
}

/// Safe status projection; configuration contents and proposed inputs are never returned.
pub fn configuration_status(root: &Path) -> Value {
    match read(root) {
        Ok(None) => serde_json::json!({"state":"not_configured"}),
        Ok(Some(policy)) => {
            serde_json::json!({"state":"configured","protected_path_count":policy.paths.len(),"protected_branch_count":policy.branches.len(),"synthetic_marker_count":policy.markers.len(),"denied_tool_count":policy.tools.len(),"denied_argv_prefix_count":policy.prefixes.len()})
        }
        Err(_) => serde_json::json!({"state":"invalid","pre_tool_use":"deny"}),
    }
}
