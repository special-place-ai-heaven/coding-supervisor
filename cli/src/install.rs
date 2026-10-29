//! `supervisor install`: render the two skills and register the gate for one harness.
//!
//! Every location comes from `docs/research/2026-10-29-installable/HARNESS-LOCATIONS.md`
//! ("section N, row M" below). Skills go to the project-scope directory, next to the
//! project-scope MCP registration, so a skill that names the `supervisor` tools only loads
//! where that server is registered. User-level directories are printed as notes, not written.

use std::fs;
use std::io::{ErrorKind, Write};
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
        manual: "Trust the project folder once: start Claude Code interactively here and accept the trust dialog. Until then Claude Code ignores the `permissions.allow` entry for `mcp__supervisor` that install wrote, and every gate tool call is refused (seen live, A-RESULTS.md session S3c). To run an isolated session without trusting the folder in your live config, set `CLAUDE_CONFIG_DIR` to a separate directory that holds your login and a `.claude.json` marking this project with `hasTrustDialogAccepted: true` (seen live, session S3d). Interactive sessions also ask once to approve the project MCP server `supervisor` from .mcp.json: approve it. `claude -p` loads it without asking (section 1, row 3).",
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
root (`[mcp_servers.supervisor]`). Codex names them `mcp__supervisor__<tool>`.\n\
In code mode they are deferred: the `exec` tool's description does not list\n\
them, so look them up in `ALL_TOOLS` inside a code cell and call them there.\n\
Codex reads that file only for a trusted project; if the tools are missing,\n\
check the project's `trust_level` in `~/.codex/config.toml` and run\n\
`codex mcp list`.",
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
root. Call them under the names Cursor's MCP tool list shows; how Cursor\n\
names them has not been measured. Cursor keeps a list of approved servers; if the tools are missing, run\n\
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
OMP mounts each tool as a device `xd://mcp__supervisor_<tool>`: read the\n\
device for its arguments, then write the JSON arguments to it to call it.\n\
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
root (`[mcp_servers.supervisor]`). Grok does not list them among its own\n\
tools: find them with `search_tool` (query `supervisor`) and call\n\
`supervisor__<tool>` through `use_tool`. Grok loads project servers only in a\n\
trusted folder; if the tools are missing, start once with `--trust` or run\n\
`/hooks-trust`, then check `grok mcp list`.",
        manual: "Trust the folder once so Grok loads project MCP servers: start `grok --trust` in the project or run `/hooks-trust` (section 5, row 3; `--trust` is missing from the CLI reference: gap G5).",
        notes: &[
            "User-level skills directory, not written: {HOME}/.grok/skills (section 5, row 1); project .agents/skills is not documented for Grok (gap G5).",
            "Grok also reads Claude Code skills, .mcp.json and .cursor/mcp.json (section 5, rows 1 and 3).",
            "No hooks are written for Grok yet. Grok 1.0.60 can refuse at PreToolUse, Stop and SubagentStop (grok-T3-RESULTS.md), so a Grok hook adapter is possible future work.",
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

    let Plan {
        state,
        changes,
        gitignore,
    } = plan(harness, &project, &binary)?;
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
    println!("state: {}", state.name());
    if dry_run {
        for h in &HARNESSES {
            let shown = match plan(h, &project, &binary) {
                Ok(p) => p.state.name(),
                Err(_) => "unreadable",
            };
            println!("harness {}: {shown}", h.id);
        }
    }
    if let State::Malformed(path) = &state {
        // Neither the parse error nor the file's text is printed: either can quote a credential.
        return Err(Failure::Error(format!(
            "{} does not parse, or a key install writes has another type; nothing was written. Fix or remove it, then run install again.",
            path.display()
        )));
    }
    if state == State::PresentCurrent {
        println!("unchanged: the registration runs this binary and every file is up to date");
    }
    for c in &changes {
        print!("{}", diff(&c.path, c.before.as_deref(), &c.after));
    }
    if dry_run {
        println!("dry run: nothing written");
    } else if !changes.is_empty() {
        for c in &changes {
            atomic_write(&c.path, &c.after)?;
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
        Err(e) if held_open(&e) => Err(held_open_failure(path)),
        Err(e) => Err(Failure::Error(format!("{}: {e}", path.display()))),
    }
}

/// What install finds for one harness in the project before it writes anything.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// Neither the gate's registration nor the harness's project directory exists.
    NotInstalled,
    /// The harness's directory or file exists without the gate's registration.
    Absent,
    /// Registered, runs this binary, and every file is already what install would write.
    PresentCurrent,
    /// Registered, but some file differs: another binary, other arguments, older skills.
    PresentStale,
    /// A config file does not parse (or a key install writes has another type); never written.
    Malformed(PathBuf),
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::NotInstalled => "not-installed",
            State::Absent => "absent",
            State::PresentCurrent => "present-current",
            State::PresentStale => "present-stale",
            State::Malformed(_) => "malformed",
        }
    }
}

struct Plan {
    state: State,
    /// Only the files whose content would change.
    changes: Vec<Change>,
    gitignore: Option<PathBuf>,
}

/// Scan one harness's files in `project` and compute what install would write. Reads only.
fn plan(harness: &Harness, project: &Path, binary: &str) -> Result<Plan, Failure> {
    let malformed = |path: PathBuf| Plan {
        state: State::Malformed(path),
        changes: Vec::new(),
        gitignore: None,
    };
    let claude = harness.id == "claude-code";
    let mcp_path = under(project, harness.mcp_file);
    let mcp_before = read_existing(&mcp_path)?;
    let toml = harness.mcp_file.ends_with(".toml");
    let Ok(registered) = registered_command(mcp_before.as_deref(), toml) else {
        return Ok(malformed(mcp_path));
    };
    let settings_path = under(project, ".claude/settings.json");
    let settings_before = if claude {
        read_existing(&settings_path)?
    } else {
        None
    };
    let Ok(hooks) = gate_hook_commands(settings_before.as_deref()) else {
        return Ok(malformed(settings_path));
    };

    // A registration that already runs this binary keeps its spelling (a symlink, another
    // form of the path, a .cmd wrapper), so re-running install over it changes nothing.
    let mcp_binary = registered
        .clone()
        .filter(|c| runs_binary(c, binary))
        .unwrap_or_else(|| binary.to_owned());
    let hook = match hooks.first() {
        Some(first)
            if hooks.iter().all(|h| h == first)
                && gate_hook_program(first).is_some_and(|p| runs_binary(p, binary)) =>
        {
            first.clone()
        }
        _ if binary.contains(char::is_whitespace) => format!("\"{binary}\" hook"),
        _ => format!("{binary} hook"),
    };

    let mut changes = Vec::new();
    for (name, template) in [("supervisor-worker", WORKER), ("supervisor-chair", CHAIR)] {
        let path = under(project, harness.skills).join(name).join("SKILL.md");
        let before = read_existing(&path)?;
        changes.push(Change {
            path,
            before,
            after: render(template, harness),
        });
    }
    // The server is told the project so it shares one gate state with the hooks, whatever
    // directory the harness starts it in.
    let project_arg = project.to_string_lossy().replace('\\', "/");
    let mcp_after = if toml {
        // Codex refuses every tool call under its non-interactive approval policy unless the
        // server approves them, and passes no environment variable it is not told to.
        let extra = if harness.id == "codex" {
            "default_tools_approval_mode = \"approve\"\nenv_vars = [\"SUPERVISOR_STATE_DIR\"]\n"
        } else {
            ""
        };
        Ok(merge_mcp_toml(
            mcp_before.as_deref(),
            &mcp_binary,
            &project_arg,
            extra,
        ))
    } else {
        merge_mcp_json(mcp_before.as_deref(), &mcp_binary, &project_arg)
    };
    let Ok(mcp_after) = mcp_after else {
        return Ok(malformed(mcp_path));
    };
    changes.push(Change {
        path: mcp_path.clone(),
        before: mcp_before,
        after: mcp_after,
    });
    let mut gitignore = None;
    if claude {
        let Ok(after) = merge_settings(settings_before.as_deref(), &hook) else {
            return Ok(malformed(settings_path));
        };
        changes.push(Change {
            path: settings_path,
            before: settings_before,
            after,
        });
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

    let dir = if claude {
        ".claude"
    } else {
        harness
            .mcp_file
            .split('/')
            .next()
            .unwrap_or(harness.mcp_file)
    };
    let state = if registered.is_some() || !hooks.is_empty() {
        if changes.is_empty() {
            State::PresentCurrent
        } else {
            State::PresentStale
        }
    } else if mcp_path.exists() || project.join(dir).exists() {
        State::Absent
    } else {
        State::NotInstalled
    };
    Ok(Plan {
        state,
        changes,
        gitignore,
    })
}

/// The command of the gate's MCP entry, `None` when there is none, `Err` when the file does
/// not parse.
// shortcut: TOML is read line by line, only the `command = "..."` line inside a plain
// `[mcp_servers.supervisor]` header; a file is malformed only when that section has no such
// line. Add a TOML parser if configs are written in other forms.
fn registered_command(text: Option<&str>, toml: bool) -> Result<Option<String>, ()> {
    if !toml {
        let root = json_object(text).map_err(|_| ())?;
        return Ok(root
            .get("mcpServers")
            .and_then(|m| m.get("supervisor"))
            .and_then(|s| s.get("command"))
            .and_then(Value::as_str)
            .map(str::to_owned));
    }
    let mut lines = text.unwrap_or("").lines().map(str::trim);
    if !lines.any(|l| l == "[mcp_servers.supervisor]") {
        return Ok(None);
    }
    for line in lines.take_while(|l| !l.starts_with('[')) {
        let Some(value) = line
            .strip_prefix("command")
            .and_then(|r| r.trim_start().strip_prefix('='))
        else {
            continue;
        };
        let quoted = value.trim();
        if quoted.len() < 2 || !quoted.starts_with('"') || !quoted.ends_with('"') {
            return Err(());
        }
        let inner = &quoted[1..quoted.len() - 1];
        return Ok(Some(inner.replace("\\\"", "\"").replace("\\\\", "\\")));
    }
    Err(())
}

/// Every hook command in the settings that is one of the gate's, in file order.
fn gate_hook_commands(text: Option<&str>) -> Result<Vec<String>, ()> {
    let root = json_object(text).map_err(|_| ())?;
    let Some(events) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    Ok(events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|g| g["hooks"].as_array())
        .flatten()
        .filter_map(|h| h["command"].as_str())
        .filter(|c| gate_hook_program(c).is_some())
        .map(str::to_owned)
        .collect())
}

/// Whether `program` runs `binary`: the same file, resolved through symlinks, or a `.cmd`
/// wrapper whose text names it.
fn runs_binary(program: &str, binary: &str) -> bool {
    let normal = |s: &str| {
        let s = s.replace('\\', "/");
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    let same = match (fs::canonicalize(program), fs::canonicalize(binary)) {
        (Ok(a), Ok(b)) => a == b,
        _ => normal(program) == normal(binary),
    };
    same || (Path::new(program)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
        && fs::read_to_string(program)
            .is_ok_and(|script| normal(&script).contains(&normal(binary))))
}

/// Windows refuses to read or replace a file another process opened without sharing it;
/// elsewhere only a read-only file or directory is refused.
fn held_open(e: &std::io::Error) -> bool {
    (cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33)))
        || e.kind() == ErrorKind::PermissionDenied
}

fn held_open_failure(path: &Path) -> Failure {
    Failure::Error(format!(
        "{} is held open by another process or is read-only, so install cannot replace it. Close the harness session that uses it and run install again; install does not stop processes or retry.",
        path.display()
    ))
}

/// Write `content` to a temporary file beside `path` and rename it over `path`, so a crash
/// mid-write leaves the old file whole. A symlinked file is written through and stays a link.
fn atomic_write(path: &Path, content: &str) -> Result<(), Failure> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let fail = |e: std::io::Error| {
        if held_open(&e) {
            held_open_failure(path)
        } else {
            Failure::Error(format!("{}: {e}", path.display()))
        }
    };
    let parent = target
        .parent()
        .ok_or_else(|| Failure::Error(format!("{} has no parent directory", path.display())))?;
    fs::create_dir_all(parent).map_err(fail)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = parent.join(format!(".{name}.supervisor-tmp-{}", std::process::id()));
    let written = fs::File::create(&temp).and_then(|mut f| {
        f.write_all(content.as_bytes())?;
        f.sync_all()
    });
    match written.and_then(|()| fs::rename(&temp, &target)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&temp);
            Err(fail(e))
        }
    }
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
        // The parser's message can quote the file, so it is not passed on.
        Some(text) => match serde_json::from_str(text).map_err(|_| "does not parse as JSON")? {
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

/// The program of a hook this gate installed: `<path> hook` (path optionally quoted) where the
/// program is `supervisor` or `supervisor-claude-code` (bare, `.exe`, or the npm `.cmd` shim),
/// at any path.
fn gate_hook_program(command: &str) -> Option<&str> {
    let program = command
        .trim()
        .strip_suffix(" hook")?
        .trim()
        .trim_matches('"');
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".cmd"))
        .unwrap_or(name);
    matches!(stem, "supervisor" | "supervisor-claude-code").then_some(program)
}

/// Per event, every gate hook (recognised by its shape, whatever the binary path) is dropped,
/// together with any group that leaves empty, and one fresh group is appended; other hooks and
/// groups stay as they are. So a moved binary or an npm upgrade leaves one entry per event.
/// Also adds `mcp__supervisor` to `permissions.allow` (Claude Code otherwise refuses the gate's
/// MCP tools under acceptEdits). No other permission key is written.
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
        for group in groups.iter_mut() {
            let Some(hs) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            let before = hs.len();
            hs.retain(|h| {
                !h["command"]
                    .as_str()
                    .is_some_and(|c| gate_hook_program(c).is_some())
            });
            // Mark a group emptied here; groups that were already empty are left alone.
            if hs.is_empty() && before > 0 {
                *group = Value::Null;
            }
        }
        groups.retain(|g| !g.is_null());
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
fn merge_mcp_toml(before: Option<&str>, binary: &str, project: &str, extra: &str) -> String {
    let quoted = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let section = format!(
        "[mcp_servers.supervisor]\ncommand = {}\nargs = [\"mcp\", \"--project\", {}]\n{extra}",
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
