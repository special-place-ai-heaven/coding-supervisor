//! `supervisor install`: render the two skills and register the gate for one harness.
//!
//! Every location comes from `docs/research/2026-10-29-installable/HARNESS-LOCATIONS.md`
//! ("section N, row M" below). Skills go to the project-scope directory, next to the
//! project-scope MCP registration, so a skill that names the `supervisor` tools only loads
//! where that server is registered. User-level directories are printed as notes, not written.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::{Failure, Flags};

const WORKER: &str = include_str!("../../skills/supervisor-worker.SKILL.template.md");
const CHAIR: &str = include_str!("../../skills/supervisor-chair.SKILL.template.md");

pub struct Harness {
    pub id: &'static str,
    display: &'static str,
    /// Project-relative skills directory, `/`-separated.
    skills: &'static str,
    /// Project-relative MCP registration file; TOML when it ends in `.toml`, JSON otherwise.
    mcp_file: &'static str,
    /// Replaces `{MCP_NOTE}` in the templates.
    mcp_note: &'static str,
    /// Steps install does not perform. `{HOME}`, `{PROJECT}`, `{BINARY}` are filled in.
    manual: &'static str,
    notes: &'static [&'static str],
}

pub const HARNESSES: [Harness; 5] = [
    Harness {
        id: "claude-code",
        display: "Claude Code",
        // Section 1, row 1: project `.claude/skills/<name>/SKILL.md` (high).
        skills: ".claude/skills",
        // Section 1, row 3: project `<project>/.mcp.json`, `mcpServers` shape (high).
        mcp_file: ".mcp.json",
        mcp_note: "In Claude Code the gate's tools (`candidate_observe`, `receipt_record`,\n\
`accept_try`, `task_complete`) come from the MCP server named `supervisor`,\n\
which `supervisor install` registered in `.mcp.json` at the project root;\n\
Claude Code lists them as `mcp__supervisor__<tool>`. An interactive session\n\
asks once to approve that server. The gate's hooks in `.claude/settings.json`\n\
also see your tool calls and your stop, so a refusal can arrive from a hook.",
        manual: "Interactive Claude Code sessions ask once to approve the project MCP server `supervisor` from .mcp.json: approve it. `claude -p` loads it without asking (section 1, row 3).",
        notes: &[
            "User-level skills directory, not written: {HOME}/.claude/skills/<name>/SKILL.md (section 1, row 1).",
            "User-scope MCP is documented only as `claude mcp add --scope user supervisor -- {BINARY} mcp` (gap G7); not run.",
            "Hooks in .claude/settings.json run under `claude -p` without a trust prompt (section 1, row 4).",
        ],
    },
    Harness {
        id: "codex",
        display: "Codex CLI",
        // Section 2, row 1: REPO scope `$CWD/.agents/skills` up to `$REPO_ROOT/.agents/skills` (high).
        skills: ".agents/skills",
        // Section 2, row 3: project `<repo>/.codex/config.toml`, `[mcp_servers.<name>]`, trusted projects only (high).
        mcp_file: ".codex/config.toml",
        mcp_note: "In Codex CLI the gate's tools (`candidate_observe`, `receipt_record`,\n\
`accept_try`, `task_complete`) come from the MCP server named `supervisor`,\n\
which `supervisor install` registered in `.codex/config.toml` at the project\n\
root (`[mcp_servers.supervisor]`). Codex reads that file only for a trusted\n\
project; if the tools are missing, check the project's `trust_level` in\n\
`~/.codex/config.toml` and run `codex mcp list`.",
        manual: "Trust the project so Codex loads .codex/config.toml: add these lines to {HOME}/.codex/config.toml\n    [projects.'{PROJECT}']\n    trust_level = \"trusted\"\n  (section 2, row 3; the Windows spelling of that key follows this machine's config, not a documented rule: gap G3).",
        notes: &[
            "User-level skills directory, not written: {HOME}/.agents/skills (section 2, row 1); {HOME}/.codex/skills is not in the current Codex docs (gap G2).",
            "Cursor and OMP also read project .agents/skills (sections 3 and 4, row 1), so they can see this copy as well.",
            "Codex hooks are not written: only the Claude Code hook set is proven live.",
        ],
    },
    Harness {
        id: "cursor",
        display: "Cursor",
        // Section 3, row 1: project `.cursor/skills/` (high); `name` must equal the folder name.
        skills: ".cursor/skills",
        // Section 3, row 3: project `.cursor/mcp.json`, `mcpServers` shape (high).
        mcp_file: ".cursor/mcp.json",
        mcp_note: "In Cursor the gate's tools (`candidate_observe`, `receipt_record`,\n\
`accept_try`, `task_complete`) come from the MCP server named `supervisor`,\n\
which `supervisor install` registered in `.cursor/mcp.json` at the project\n\
root. Cursor keeps a list of approved servers; if the tools are missing, run\n\
`cursor-agent mcp enable supervisor` (or pass `--approve-mcps` in a scripted\n\
run) and check `cursor-agent mcp list`.",
        manual: "Approve the server once: run `cursor-agent mcp enable supervisor` in the project, or pass `--approve-mcps` in scripted runs (section 3, row 3; whether this covers project servers only is gap G4a). On this machine the bare `agent` command is not Cursor; use `cursor-agent`.",
        notes: &[
            "Also documented, not written: project .agents/skills, {HOME}/.cursor/skills and {HOME}/.agents/skills (section 3, row 1).",
            "Cursor also reads .claude/skills and the hooks in .claude/settings.json (section 3, rows 1 and 4).",
            "Cursor hooks are not written: only the Claude Code hook set is proven live.",
        ],
    },
    Harness {
        id: "omp",
        display: "OMP",
        // Section 4, row 1: project `<ancestor>/.omp/skills/<name>/SKILL.md` (high); `description` required.
        skills: ".omp/skills",
        // Section 4, row 3: project `<project>/.omp/mcp.json`, `mcpServers` shape (high).
        mcp_file: ".omp/mcp.json",
        mcp_note: "In OMP the gate's tools (`candidate_observe`, `receipt_record`,\n\
`accept_try`, `task_complete`) come from the MCP server named `supervisor`,\n\
which `supervisor install` registered in `.omp/mcp.json` at the project root.\n\
OMP loads project servers without an approval step; if the tools are missing,\n\
run `/mcp list` and `/mcp test supervisor`.",
        manual: "None: OMP loads project MCP servers without approval (section 4, row 3).",
        notes: &[
            "User-level skills directory, not written: {HOME}/.omp/agent/skills (section 4, row 1); managed-skills belongs to OMP's auto-learn feature and is not used.",
            "OMP also reads project .claude, .codex and .agents skills by default (section 4, row 1).",
            "OMP hooks are code modules, not a JSON file, so no hook is written (section 4, row 4; gap G6).",
        ],
    },
    Harness {
        id: "grok",
        display: "Grok Build",
        // Section 5, row 1: project `./.grok/skills/` walked up to the repo root (high).
        skills: ".grok/skills",
        // Section 5, row 3: project `<project>/.grok/config.toml`, `[mcp_servers.<name>]` (high).
        mcp_file: ".grok/config.toml",
        mcp_note: "In Grok Build the gate's tools (`candidate_observe`, `receipt_record`,\n\
`accept_try`, `task_complete`) come from the MCP server named `supervisor`,\n\
which `supervisor install` registered in `.grok/config.toml` at the project\n\
root (`[mcp_servers.supervisor]`). Grok loads project servers only in a\n\
trusted folder; if the tools are missing, start once with `--trust` or run\n\
`/hooks-trust`, then check `grok mcp list`.",
        manual: "Trust the folder once so Grok loads project MCP servers: start `grok --trust` in the project or run `/hooks-trust` (section 5, row 3; `--trust` is missing from the CLI reference: gap G5).",
        notes: &[
            "User-level skills directory, not written: {HOME}/.grok/skills (section 5, row 1); project .agents/skills is not documented for Grok (gap G5).",
            "Grok also reads Claude Code skills, .mcp.json and .cursor/mcp.json (section 5, rows 1 and 3).",
            "Grok hooks cannot refuse completion (section 5, row 4), so no hook is written.",
        ],
    },
];

/// The six events and matchers of the proven fixture `.claude/settings-adapter.json`.
const HOOK_EVENTS: [(&str, Option<&str>); 6] = [
    ("UserPromptSubmit", None),
    (
        "PreToolUse",
        Some("Agent|Write|Edit|NotebookEdit|Bash|PowerShell|mcp__terminal_commander__.*"),
    ),
    (
        "PostToolUse",
        Some("Agent|Bash|PowerShell|Write|Edit|mcp__terminal_commander__.*"),
    ),
    ("PostToolUseFailure", Some("Bash|PowerShell")),
    ("SubagentStop", None),
    ("Stop", None),
];

/// Claude Code permission rule that allows every tool of the MCP server `supervisor`.
const MCP_ALLOW: &str = "mcp__supervisor";

const CREDENTIAL_WORDS: [&str; 4] = ["token", "key", "secret", "password"];

struct Change {
    path: PathBuf,
    before: Option<String>,
    after: String,
}

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let id = flags
        .value("--harness")
        .ok_or_else(|| Failure::Usage("--harness is required".into()))?;
    let harness = HARNESSES
        .iter()
        .find(|h| h.id == id)
        .ok_or_else(|| Failure::Usage(format!("unknown harness: {id}")))?;
    let project = flags.project()?;
    let home = match flags.value("--home") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            std::env::var(var)
                .map(PathBuf::from)
                .map_err(|_| Failure::Error(format!("{var} is not set; pass --home")))?
        }
    };
    let home = std::path::absolute(&home)
        .map_err(|e| Failure::Error(format!("{}: {e}", home.display())))?;
    let binary = match flags.value("--binary") {
        Some(binary) => binary.to_string(),
        None => std::env::current_exe()
            .map_err(|e| Failure::Error(format!("running executable: {e}")))?
            .to_string_lossy()
            .into_owned(),
    }
    .replace('\\', "/");
    let dry_run = flags.switch("--dry-run");

    let mut changes = Vec::new();
    for (name, template) in [("supervisor-worker", WORKER), ("supervisor-chair", CHAIR)] {
        let path = under(&project, harness.skills).join(name).join("SKILL.md");
        changes.push(change(path, |_| Ok(render(template, harness)))?);
    }
    let mcp_path = under(&project, harness.mcp_file);
    // The server is told the project so it shares one gate state with the hooks, whatever
    // directory the harness starts it in.
    let project_arg = project.to_string_lossy().replace('\\', "/");
    changes.push(if harness.mcp_file.ends_with(".toml") {
        change(mcp_path, |before| {
            Ok(merge_mcp_toml(before, &binary, &project_arg))
        })?
    } else {
        change(mcp_path, |before| {
            merge_mcp_json(before, &binary, &project_arg)
        })?
    });
    let mut gitignore = None;
    if harness.id == "claude-code" {
        let command = if binary.contains(char::is_whitespace) {
            format!("\"{binary}\" hook")
        } else {
            format!("{binary} hook")
        };
        let settings = under(&project, ".claude/settings.json");
        changes.push(change(settings, |before| merge_settings(before, &command))?);
        let path = project.join(".gitignore");
        if let Some(before) = read_existing(&path)? {
            let listed = before.lines().any(|l| {
                matches!(
                    l.trim(),
                    ".supervisor/" | ".supervisor" | "/.supervisor/" | "/.supervisor"
                )
            });
            if !listed {
                let mut after = before.clone();
                if !after.is_empty() && !after.ends_with('\n') {
                    after.push('\n');
                }
                after.push_str(".supervisor/\n");
                gitignore = Some(path.clone());
                changes.push(Change {
                    path,
                    before: Some(before),
                    after,
                });
            }
        }
    }
    changes.retain(|c| c.before.as_deref() != Some(c.after.as_str()));
    if let Some(stray) = changes
        .iter()
        .find(|c| !c.path.starts_with(&project) && !c.path.starts_with(&home))
    {
        return Err(Failure::Error(format!(
            "refusing to write outside the project and home: {}",
            stray.path.display()
        )));
    }

    println!(
        "install {} into {}{}",
        harness.display,
        project.display(),
        if dry_run { " (dry run)" } else { "" }
    );
    if changes.is_empty() {
        println!("no changes: every file is already up to date");
    }
    for c in &changes {
        print!("{}", diff(&c.path, c.before.as_deref(), &c.after));
    }
    if dry_run {
        println!("dry run: nothing written");
    } else {
        for c in &changes {
            if let Some(parent) = c.path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| Failure::Error(format!("{}: {e}", parent.display())))?;
            }
            fs::write(&c.path, &c.after)
                .map_err(|e| Failure::Error(format!("{}: {e}", c.path.display())))?;
        }
        println!("wrote {} file(s)", changes.len());
    }
    if changes
        .iter()
        .any(|c| c.path.extension().is_some_and(|e| e == "json"))
    {
        println!(
            "note: JSON files are rewritten with two-space indentation; keys are written in sorted order."
        );
    }
    if let Some(path) = gitignore {
        let verb = if dry_run { "would add" } else { "added" };
        println!("{verb} .supervisor/ to {}", path.display());
    }
    let home_shown = home.to_string_lossy().replace('\\', "/");
    let fill = |text: &str| {
        text.replace("{HOME}", &home_shown)
            .replace("{PROJECT}", &project.to_string_lossy())
            .replace("{BINARY}", &binary)
    };
    println!("manual step (not performed): {}", fill(harness.manual));
    for note in harness.notes {
        println!("note: {}", fill(note));
    }
    Ok(())
}

/// `base` joined with a `/`-separated relative path, one component at a time.
fn under(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_path_buf(), |p, part| p.join(part))
}

fn read_existing(path: &Path) -> Result<Option<String>, Failure> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Failure::Error(format!("{}: {e}", path.display()))),
    }
}

fn change(
    path: PathBuf,
    after: impl FnOnce(Option<&str>) -> Result<String, String>,
) -> Result<Change, Failure> {
    let before = read_existing(&path)?;
    let after = after(before.as_deref())
        .map_err(|e| Failure::Error(format!("{}: {e}; nothing written", path.display())))?;
    Ok(Change {
        path,
        before,
        after,
    })
}

fn render(template: &str, harness: &Harness) -> String {
    template
        .replace("\r\n", "\n")
        .replace("{HARNESS}", harness.display)
        .replace("{MCP_NOTE}", harness.mcp_note)
}

/// The existing JSON object (empty when the file is absent or blank).
fn json_object(before: Option<&str>) -> Result<Map<String, Value>, String> {
    match before.filter(|t| !t.trim().is_empty()) {
        None => Ok(Map::new()),
        Some(text) => match serde_json::from_str(text).map_err(|e| e.to_string())? {
            Value::Object(map) => Ok(map),
            _ => Err("not a JSON object".into()),
        },
    }
}

/// Two-space pretty print. serde_json without `preserve_order` writes keys in sorted order.
fn pretty(map: Map<String, Value>) -> Result<String, String> {
    serde_json::to_string_pretty(&Value::Object(map))
        .map(|s| s + "\n")
        .map_err(|e| e.to_string())
}

fn merge_mcp_json(before: Option<&str>, binary: &str, project: &str) -> Result<String, String> {
    let mut root = json_object(before)?;
    root.entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"mcpServers\" is not an object")?
        .insert(
            "supervisor".into(),
            json!({ "type": "stdio", "command": binary, "args": ["mcp", "--project", project] }),
        );
    pretty(root)
}

/// Appends our group to each event unless a hook with our command is already there, and
/// adds `mcp__supervisor` to `permissions.allow` (Claude Code otherwise refuses the gate's MCP
/// tools under acceptEdits). No other permission key is written.
fn merge_settings(before: Option<&str>, command: &str) -> Result<String, String> {
    let mut root = json_object(before)?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"hooks\" is not an object")?;
    for (event, matcher) in HOOK_EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not an array"))?;
        let present = groups.iter().any(|g| {
            g["hooks"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(|h| h["command"] == command))
        });
        if !present {
            let mut group = Map::new();
            if let Some(m) = matcher {
                group.insert("matcher".into(), m.into());
            }
            group.insert(
                "hooks".into(),
                json!([{ "type": "command", "command": command }]),
            );
            groups.push(Value::Object(group));
        }
    }
    let allow = root
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"permissions\" is not an object")?
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("permissions.allow is not an array")?;
    if !allow.iter().any(|rule| rule == MCP_ALLOW) {
        allow.push(MCP_ALLOW.into());
    }
    pretty(root)
}

/// Replaces the lines of `[mcp_servers.supervisor]` or appends the section; other lines stay
/// byte for byte.
// shortcut: only the plain header line `[mcp_servers.supervisor]` is recognised; a quoted key,
// a trailing comment on the header, dotted keys or an inline table are not. Use a TOML parser
// if a user's config is written that way.
fn merge_mcp_toml(before: Option<&str>, binary: &str, project: &str) -> String {
    let quoted = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let section = format!(
        "[mcp_servers.supervisor]\ncommand = {}\nargs = [\"mcp\", \"--project\", {}]\n",
        quoted(binary),
        quoted(project)
    );
    let text = before.unwrap_or("");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    match lines
        .iter()
        .position(|l| l.trim() == "[mcp_servers.supervisor]")
    {
        Some(start) => {
            let mut end = lines[start + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map_or(lines.len(), |i| start + 1 + i);
            // Blank lines and comments just above the next header belong to that header.
            while end > start + 1 && {
                let t = lines[end - 1].trim();
                t.is_empty() || t.starts_with('#')
            } {
                end -= 1;
            }
            [lines[..start].concat(), section, lines[end..].concat()].concat()
        }
        None => {
            let mut out = text.to_string();
            if !out.is_empty() {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
            out + &section
        }
    }
}

/// Line diff without context: the common head and tail are skipped, the rest is printed as
/// removed then added. Lines that look like credentials print as `<redacted>`.
fn diff(path: &Path, before: Option<&str>, after: &str) -> String {
    let old: Vec<&str> = before.unwrap_or("").lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let shown = path.display().to_string();
    let mut out = format!(
        "--- {}\n+++ {shown}\n",
        if before.is_some() {
            shown.as_str()
        } else {
            "/dev/null"
        }
    );
    for (sign, lines) in [('-', &old), ('+', &new)] {
        let hidden = sensitive_lines(lines);
        for (line, hide) in lines.iter().zip(hidden).take(lines.len() - tail).skip(head) {
            let line = if hide { "<redacted>" } else { line };
            out.push_str(&format!("{sign}{line}\n"));
        }
    }
    out
}

/// Marks lines that name a credential word, lines inside a JSON `"env"` object and lines
/// inside a TOML `[...env]` table.
fn sensitive_lines(lines: &[&str]) -> Vec<bool> {
    let mut json_env_depth = 0usize;
    let mut toml_env = false;
    lines
        .iter()
        .map(|line| {
            let lower = line.trim().to_ascii_lowercase();
            let opens = lower.matches('{').count();
            let closes = lower.matches('}').count();
            if json_env_depth > 0 {
                json_env_depth = (json_env_depth + opens).saturating_sub(closes);
                return true;
            }
            if lower.starts_with("\"env\"")
                || lower.starts_with("env ")
                || lower.starts_with("env=")
            {
                json_env_depth = opens.saturating_sub(closes);
                return true;
            }
            let header = lower.starts_with('[');
            if header {
                toml_env = lower.trim_end_matches(']').ends_with("env");
            }
            (toml_env && !header) || CREDENTIAL_WORDS.iter().any(|w| lower.contains(w))
        })
        .collect()
}
