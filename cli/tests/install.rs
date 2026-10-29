//! `supervisor install` against throwaway project and home directories.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
const FAKE_BINARY: &str = "C:/opt/sup/supervisor.exe";
const OUR_HOOK: &str = "C:/opt/sup/supervisor.exe hook";

/// (harness, project skills directory, MCP registration file)
const EXPECTED: [(&str, &str, &str); 5] = [
    ("claude-code", ".claude/skills", ".mcp.json"),
    ("codex", ".agents/skills", ".codex/config.toml"),
    ("cursor", ".cursor/skills", ".cursor/mcp.json"),
    ("omp", ".omp/skills", ".omp/mcp.json"),
    ("grok", ".grok/skills", ".grok/config.toml"),
];

fn dirs(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("sup-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, home) = (root.join("proj"), root.join("home"));
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    (project, home)
}

fn install(project: &Path, home: &Path, harness: &str, extra: &[&str]) -> String {
    let out = Command::new(BIN)
        .args(["install", "--harness", harness, "--project"])
        .arg(project)
        .arg("--home")
        .arg(home)
        .args(["--binary", FAKE_BINARY])
        .args(extra)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "install {harness} failed: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// Every file under `dir` with its bytes, sorted by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                files.push((path, bytes));
            }
        }
    }
    files.sort();
    files
}

/// The project path as install writes it into the registration files.
fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The `[mcp_servers.supervisor]` section install writes for a TOML harness.
fn toml_section(harness: &str, project: &Path) -> String {
    let codex = if harness == "codex" {
        "default_tools_approval_mode = \"approve\"\nenv_vars = [\"SUPERVISOR_STATE_DIR\"]\n"
    } else {
        ""
    };
    format!(
        "[mcp_servers.supervisor]\ncommand = \"{FAKE_BINARY}\"\nargs = [\"mcp\", \"--project\", \"{}\"]\n{codex}",
        forward(project)
    )
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_harness_gets_its_documented_files_and_a_second_run_changes_nothing() {
    for (harness, skills, mcp) in EXPECTED {
        let (project, home) = dirs(&format!("each-{harness}"));
        install(&project, &home, harness, &[]);

        for name in ["supervisor-worker", "supervisor-chair"] {
            let skill = fs::read_to_string(project.join(skills).join(name).join("SKILL.md"))
                .unwrap_or_else(|e| panic!("{harness} {name}: {e}"));
            assert!(
                skill.starts_with(&format!("---\nname: {name}\n")),
                "{harness}"
            );
            assert!(!skill.contains("{HARNESS}") && !skill.contains("{MCP_NOTE}"));
            // Only the chair calls the gate's tools; the worker template has no {MCP_NOTE}.
            if name == "supervisor-chair" {
                assert!(skill.contains(mcp), "{harness} chair skill names {mcp}");
                // First real-project use: the receipt fields and no task_start where hooks run.
                assert!(
                    skill.contains("`output_ref`") && skill.contains("do not call `task_start`")
                );
            }
            assert!(
                skill.contains("nothing appended"),
                "{harness} {name} says the command runs alone"
            );
            // Both sides of the handoff name the criteria table, so a worker knows each command up front.
            assert!(
                skill.contains("id, command, mandatory"),
                "{harness} {name} names the criteria table"
            );
        }
        let registration = project.join(mcp);
        if mcp.ends_with(".toml") {
            let text = fs::read_to_string(&registration).unwrap();
            assert_eq!(text, toml_section(harness, &project));
        } else {
            assert_eq!(
                read_json(&registration)["mcpServers"]["supervisor"],
                json!({
                    "type": "stdio",
                    "command": FAKE_BINARY,
                    "args": ["mcp", "--project", forward(&project)]
                })
            );
        }
        assert!(snapshot(&home).is_empty(), "{harness} wrote into home");

        let before = snapshot(&project);
        let second = install(&project, &home, harness, &[]);
        assert_eq!(
            snapshot(&project),
            before,
            "{harness} second run changed bytes"
        );
        assert!(
            second.contains("state: present-current") && second.contains("unchanged"),
            "{harness}: {second}"
        );
        assert!(
            !second.contains("+++ "),
            "{harness} second run printed a diff"
        );
    }
}

#[test]
fn claude_settings_keep_foreign_hooks_and_permissions_and_gain_the_fixture_hooks() {
    let (project, home) = dirs("settings");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let settings = project.join(".claude").join("settings.json");
    fs::write(
        &settings,
        r#"{
  "permissions": { "allow": ["Read"] },
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "foreign-guard" }] }
    ]
  }
}
"#,
    )
    .unwrap();
    install(&project, &home, "claude-code", &[]);
    install(&project, &home, "claude-code", &[]);

    let v = read_json(&settings);
    assert_eq!(
        v["permissions"],
        json!({ "allow": ["Read", "mcp__supervisor"] })
    );
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "foreign-guard"
    );

    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/claude-code-enforcement/.claude/settings-adapter.json");
    let fixture = read_json(&fixture_path);
    let fixture_hooks = fixture["hooks"].as_object().unwrap();
    assert_eq!(fixture_hooks.len(), 6);
    assert_eq!(v["hooks"].as_object().unwrap().len(), 6);
    for (event, groups) in fixture_hooks {
        let ours: Vec<&Value> = v["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["hooks"][0]["command"] == OUR_HOOK)
            .collect();
        assert_eq!(ours.len(), 1, "{event}: exactly one of ours after two runs");
        assert_eq!(ours[0]["matcher"], groups[0]["matcher"], "{event} matcher");
        assert_eq!(ours[0]["hooks"][0]["type"], "command");
    }
}

#[test]
fn claude_settings_allow_the_gate_tools_once_and_touch_no_other_permission() {
    let (project, home) = dirs("allow");
    install(&project, &home, "claude-code", &[]);
    let settings = project.join(".claude").join("settings.json");
    assert_eq!(
        read_json(&settings)["permissions"],
        json!({ "allow": ["mcp__supervisor"] })
    );

    let existing = json!({
        "permissions": {
            "allow": ["Read", "mcp__supervisor", "Bash(git status)"],
            "deny": ["WebFetch"],
            "defaultMode": "acceptEdits"
        }
    });
    fs::write(&settings, existing.to_string()).unwrap();
    install(&project, &home, "claude-code", &[]);
    install(&project, &home, "claude-code", &[]);
    assert_eq!(read_json(&settings)["permissions"], existing["permissions"]);

    fs::write(
        &settings,
        json!({ "permissions": { "ask": ["Bash"] } }).to_string(),
    )
    .unwrap();
    install(&project, &home, "claude-code", &[]);
    assert_eq!(
        read_json(&settings)["permissions"],
        json!({ "ask": ["Bash"], "allow": ["mcp__supervisor"] })
    );
}

#[test]
fn an_existing_mcp_json_keeps_its_other_server() {
    let (project, home) = dirs("mcp-json");
    fs::write(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"other":{"command":"other-server","args":["--x"]}}}"#,
    )
    .unwrap();
    install(&project, &home, "claude-code", &[]);
    let v = read_json(&project.join(".mcp.json"));
    assert_eq!(
        v["mcpServers"]["other"],
        json!({ "command": "other-server", "args": ["--x"] })
    );
    assert_eq!(v["mcpServers"]["supervisor"]["command"], FAKE_BINARY);
}

#[test]
fn a_toml_config_keeps_other_sections_and_replaces_only_ours() {
    for (harness, file) in [("codex", ".codex"), ("grok", ".grok")] {
        let (project, home) = dirs(&format!("toml-{harness}"));
        fs::create_dir_all(project.join(file)).unwrap();
        let config = project.join(file).join("config.toml");
        let foreign = "[mcp_servers.x]\ncommand = \"x-server\"\n\n";
        let tail = "\n# about the profile\n[profile]\nmodel = \"m\"\n";
        fs::write(
            &config,
            format!(
                "{foreign}[mcp_servers.supervisor]\ncommand = \"old\"\nargs = []\nenabled = false\n{tail}"
            ),
        )
        .unwrap();
        install(&project, &home, harness, &[]);
        let ours = toml_section(harness, &project);
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            format!("{foreign}{ours}{tail}")
        );

        // Absent section: appended after the existing text.
        fs::write(&config, "[mcp_servers.x]\ncommand = \"x-server\"").unwrap();
        install(&project, &home, harness, &[]);
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            format!("[mcp_servers.x]\ncommand = \"x-server\"\n\n{ours}")
        );
        let before = snapshot(&project);
        install(&project, &home, harness, &[]);
        assert_eq!(snapshot(&project), before);
    }
}

#[test]
fn dry_run_prints_the_diff_and_writes_nothing() {
    for (harness, _, _) in EXPECTED {
        let (project, home) = dirs(&format!("dry-{harness}"));
        fs::write(project.join(".gitignore"), "target/\n").unwrap();
        let before = snapshot(&project);
        let out = install(&project, &home, harness, &["--dry-run"]);
        assert!(
            out.contains("+++ ") && out.contains("dry run: nothing written"),
            "{out}"
        );
        assert_eq!(snapshot(&project), before, "{harness}");
        assert!(snapshot(&home).is_empty());
    }
}

#[test]
fn credential_lines_print_as_redacted() {
    let (project, home) = dirs("redact");
    fs::create_dir_all(project.join(".claude")).unwrap();
    fs::write(
        project.join(".claude").join("settings.json"),
        "{\n  \"token\": \"abc\",\n  \"env\": {\n    \"API_BASE\": \"https://private.example\"\n  },\n  \"model\": \"m\"\n}\n",
    )
    .unwrap();
    let out = install(&project, &home, "claude-code", &[]);
    assert!(out.contains("<redacted>"), "{out}");
    assert!(!out.contains("abc"), "{out}");
    assert!(!out.contains("private.example"), "{out}");
    let v = read_json(&project.join(".claude").join("settings.json"));
    assert_eq!(v["token"], "abc", "the value itself is kept");
}

#[test]
fn gitignore_gains_the_state_directory_once() {
    let (project, home) = dirs("gitignore");
    fs::write(project.join(".gitignore"), "target/").unwrap();
    let out = install(&project, &home, "claude-code", &[]);
    assert!(out.contains("added .supervisor/ to"), "{out}");
    install(&project, &home, "claude-code", &[]);
    assert_eq!(
        fs::read_to_string(project.join(".gitignore")).unwrap(),
        "target/\n.supervisor/\n"
    );
}

#[test]
fn an_unknown_harness_is_a_usage_error() {
    let (project, home) = dirs("unknown");
    let out = Command::new(BIN)
        .args(["install", "--harness", "nope", "--project"])
        .arg(&project)
        .arg("--home")
        .arg(&home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(snapshot(&project).is_empty());
}

/// Install with an explicit binary; returns (success, stdout + stderr).
fn run_install(
    project: &Path,
    home: &Path,
    harness: &str,
    binary: &str,
    extra: &[&str],
) -> (bool, String) {
    let out = Command::new(BIN)
        .args(["install", "--harness", harness, "--project"])
        .arg(project)
        .arg("--home")
        .arg(home)
        .args(["--binary", binary])
        .args(extra)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

const EVENTS: [&str; 6] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStop",
    "Stop",
];

/// Lab repo, 2026-10-29: settings already held the gate's hooks from another binary path, and
/// install appended a second set (12 entries). Our hooks are recognised by the `<path> hook`
/// shape, whatever the path, dropped, and one fresh entry appended per event.
#[test]
fn claude_settings_replace_our_hooks_from_another_binary_path() {
    let (project, home) = dirs("settings-moved");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let settings = project.join(".claude").join("settings.json");
    // The broken state seen live, two old paths per event (one quoted, one an npm install),
    // plus an unrelated hook on every event and one sharing a group with an old entry.
    let foreign = |event: &str| json!({ "matcher": "Bash", "hooks": [{ "type": "command", "command": format!("guard-{event} hook-like") }] });
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert(
            event.into(),
            json!([
                foreign(event),
                { "hooks": [{ "type": "command", "command": "\"C:/Program Files/old/supervisor.exe\" hook" }] },
                { "hooks": [
                    { "type": "command", "command": "/usr/lib/node_modules/coding-supervisor-linux-x64/bin/supervisor hook" },
                    { "type": "command", "command": "audit-log" }
                ] }
            ]),
        );
    }
    fs::write(&settings, json!({ "hooks": hooks }).to_string()).unwrap();
    fs::write(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"other":{"command":"x"},"supervisor":{"type":"stdio","command":"C:/old/supervisor.exe","args":["mcp"]}}}"#,
    )
    .unwrap();

    let (ok, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, &[]);
    assert!(ok && out.contains("state: present-stale"), "{out}");

    let v = read_json(&settings);
    for event in EVENTS {
        let groups = v["hooks"][event].as_array().unwrap();
        let gate: Vec<&str> = groups
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .filter_map(|h| h["command"].as_str())
            .filter(|c| c.ends_with(" hook"))
            .collect();
        assert_eq!(
            gate,
            [OUR_HOOK],
            "{event}: exactly one gate hook, the new path"
        );
        assert_eq!(
            groups[0],
            foreign(event),
            "{event}: unrelated group untouched"
        );
        assert_eq!(
            groups[1]["hooks"],
            json!([{ "type": "command", "command": "audit-log" }]),
            "{event}: the unrelated hook sharing a group stays"
        );
        assert_eq!(groups.len(), 3, "{event}: foreign, shared, ours");
    }
    let mcp = read_json(&project.join(".mcp.json"));
    assert_eq!(mcp["mcpServers"]["supervisor"]["command"], FAKE_BINARY);
    assert_eq!(mcp["mcpServers"]["other"], json!({ "command": "x" }));

    let before = snapshot(&project);
    let (ok, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, &[]);
    assert!(ok && out.contains("unchanged"), "{out}");
    assert_eq!(snapshot(&project), before);
}

#[test]
fn dry_run_prints_the_state_of_every_harness() {
    let (project, home) = dirs("states");
    let states = |out: &str| -> Vec<String> {
        out.lines()
            .filter_map(|l| l.strip_prefix("harness "))
            .map(str::to_owned)
            .collect()
    };
    let (_, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, &["--dry-run"]);
    assert_eq!(
        states(&out),
        [
            "claude-code: not-installed",
            "codex: not-installed",
            "cursor: not-installed",
            "omp: not-installed",
            "grok: not-installed"
        ],
        "{out}"
    );
    assert!(snapshot(&project).is_empty(), "the dry run wrote");

    install(&project, &home, "claude-code", &[]);
    install(&project, &home, "codex", &[]);
    fs::create_dir_all(project.join(".cursor")).unwrap();
    fs::write(project.join(".cursor/mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
    // OMP registered with another binary: stale.
    fs::create_dir_all(project.join(".omp")).unwrap();
    fs::write(
        project.join(".omp/mcp.json"),
        r#"{"mcpServers":{"supervisor":{"command":"/old/supervisor","args":["mcp"]}}}"#,
    )
    .unwrap();
    let (_, out) = run_install(&project, &home, "grok", FAKE_BINARY, &["--dry-run"]);
    assert_eq!(
        states(&out),
        [
            "claude-code: present-current",
            "codex: present-current",
            "cursor: absent",
            "omp: present-stale",
            "grok: not-installed"
        ],
        "{out}"
    );
}

#[test]
fn a_malformed_config_is_never_written_and_its_text_is_never_printed() {
    let (project, home) = dirs("malformed");
    let broken = r#"{"mcpServers": {"supervisor": {"env": {"API_TOKEN": "s3cr3t-value"}"#;
    fs::write(project.join(".mcp.json"), broken).unwrap();
    let before = snapshot(&project);
    for extra in [&[][..], &["--dry-run"][..]] {
        let (ok, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, extra);
        assert!(!ok, "{out}");
        assert!(
            out.contains("state: malformed") && out.contains(".mcp.json"),
            "{out}"
        );
        assert!(out.contains("nothing was written"), "{out}");
        assert!(
            !out.contains("s3cr3t") && !out.contains("API_TOKEN"),
            "{out}"
        );
        assert!(
            !out.contains("EOF") && !out.contains("line 1"),
            "parse error printed: {out}"
        );
        assert_eq!(
            snapshot(&project),
            before,
            "a malformed project was written"
        );
    }
    // A TOML section without a readable command is malformed too.
    let (project, home) = dirs("malformed-toml");
    fs::create_dir_all(project.join(".codex")).unwrap();
    fs::write(
        project.join(".codex/config.toml"),
        "[mcp_servers.supervisor]\ncommand = unquoted\n",
    )
    .unwrap();
    let (ok, out) = run_install(&project, &home, "codex", FAKE_BINARY, &[]);
    assert!(!ok && out.contains("state: malformed"), "{out}");
}

/// The registration names a `.cmd` wrapper that invokes this binary: the same install, so
/// nothing is rewritten.
#[test]
fn a_registration_through_a_cmd_wrapper_of_this_binary_is_current() {
    let (project, home) = dirs("wrapper");
    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let real = bin_dir.join("supervisor.exe");
    fs::write(&real, "binary").unwrap();
    let real = forward(&real);
    let wrapper = home.join("supervisor.cmd");
    fs::write(
        &wrapper,
        format!("@echo off\r\n\"{}\" %*\r\n", real.replace('/', "\\")),
    )
    .unwrap();
    let wrapper = forward(&wrapper);

    let (ok, out) = run_install(&project, &home, "claude-code", &real, &[]);
    assert!(ok, "{out}");
    // Point the registration and every hook at the wrapper.
    for file in [".mcp.json", ".claude/settings.json"] {
        let path = project.join(file);
        let text = fs::read_to_string(&path).unwrap().replace(&real, &wrapper);
        fs::write(&path, text).unwrap();
    }
    let before = snapshot(&project);
    let (ok, out) = run_install(&project, &home, "claude-code", &real, &[]);
    assert!(
        ok && out.contains("state: present-current") && out.contains("unchanged"),
        "{out}"
    );
    assert_eq!(snapshot(&project), before);
}

/// Every file is written through a temporary file beside it and renamed over: none is left.
#[test]
fn install_leaves_no_temporary_file() {
    let (project, home) = dirs("no-temp");
    for harness in ["claude-code", "codex", "cursor", "omp", "grok"] {
        install(&project, &home, harness, &[]);
    }
    let leftovers: Vec<_> = snapshot(&project)
        .into_iter()
        .filter(|(p, _)| p.to_string_lossy().contains("supervisor-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A symlinked config is written through: the link stays, its target gets the content.
#[cfg(unix)]
#[test]
fn a_symlinked_config_stays_a_link() {
    let (project, home) = dirs("symlink");
    let real = home.join("mcp-real.json");
    fs::write(&real, "{}").unwrap();
    std::os::unix::fs::symlink(&real, project.join(".mcp.json")).unwrap();
    install(&project, &home, "claude-code", &[]);
    assert!(
        fs::symlink_metadata(project.join(".mcp.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        read_json(&real)["mcpServers"]["supervisor"]["command"],
        FAKE_BINARY
    );
}

/// A config another process holds open is not forced: install fails naming the file, the old
/// content stays whole, and no temporary file is left.
#[cfg(windows)]
#[test]
fn a_config_held_open_fails_with_advice_and_stays_intact() {
    use std::os::windows::fs::OpenOptionsExt;
    let (project, home) = dirs("held-open");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let settings = project.join(".claude").join("settings.json");
    fs::write(&settings, "{}\n").unwrap();
    // Share read only: install can read it but not replace it, as with a harness holding it.
    let holder = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&settings)
        .unwrap();
    let (ok, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, &[]);
    drop(holder);
    assert!(!ok, "{out}");
    assert!(
        out.contains("settings.json") && out.contains("Close the harness session"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{}\n");
    let leftovers: Vec<_> = snapshot(&project)
        .into_iter()
        .filter(|(p, _)| p.to_string_lossy().contains("supervisor-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// Writes `<root>/.claude/<name>` with the given `enabledPlugins` entries.
fn write_plugin_settings(root: &Path, name: &str, enabled: Value) {
    let dir = root.join(".claude");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(name),
        json!({ "enabledPlugins": enabled }).to_string(),
    )
    .unwrap();
}

const PLUGIN_ENABLED: &str = "claude-code: the coding-supervisor plugin is enabled for this project; it carries the hooks and the MCP server, so nothing was written.";

/// The gate's plugin, enabled in any of the three settings files Claude Code reads, carries the
/// hooks and the MCP server: install says so and writes nothing.
#[test]
fn an_enabled_plugin_makes_claude_code_install_write_nothing() {
    for (n, place) in [
        "project settings.json",
        "project settings.local.json",
        "home settings.json",
    ]
    .into_iter()
    .enumerate()
    {
        let (project, home) = dirs(&format!("plugin-on-{n}"));
        let enabled = json!({ "coding-supervisor@coding-supervisor": true });
        match n {
            0 => write_plugin_settings(&project, "settings.json", enabled),
            1 => write_plugin_settings(&project, "settings.local.json", enabled),
            _ => write_plugin_settings(&home, "settings.json", enabled),
        }
        let (before_project, before_home) = (snapshot(&project), snapshot(&home));
        let out = install(&project, &home, "claude-code", &[]);
        assert_eq!(out.trim(), PLUGIN_ENABLED, "{place}");
        assert_eq!(
            snapshot(&project),
            before_project,
            "{place} wrote to the project"
        );
        assert_eq!(snapshot(&home), before_home, "{place} wrote to home");
    }
}

/// A disabled plugin, another plugin and a settings file that does not parse all leave install
/// doing its normal work.
#[test]
fn a_plugin_that_is_not_enabled_does_not_stop_claude_code_install() {
    for (n, enabled) in [
        json!({ "coding-supervisor@coding-supervisor": false }),
        json!({ "other@coding-supervisor": true, "coding-supervisor@other": true }),
    ]
    .into_iter()
    .enumerate()
    {
        let (project, home) = dirs(&format!("plugin-off-{n}"));
        write_plugin_settings(&project, "settings.json", enabled);
        let out = install(&project, &home, "claude-code", &[]);
        assert!(!out.contains("plugin is enabled"), "{out}");
        assert!(project.join(".mcp.json").is_file(), "{out}");
    }
    let (project, home) = dirs("plugin-off-garbage");
    fs::create_dir_all(project.join(".claude")).unwrap();
    fs::write(
        project.join(".claude").join("settings.local.json"),
        "not json",
    )
    .unwrap();
    let (_ok, out) = run_install(&project, &home, "claude-code", FAKE_BINARY, &[]);
    assert!(!out.contains("plugin is enabled"), "{out}");
}

/// The plugin is Claude Code's: the other four harnesses install as before.
#[test]
fn an_enabled_plugin_does_not_change_install_for_other_harnesses() {
    let (project, home) = dirs("plugin-other-harness");
    write_plugin_settings(
        &project,
        "settings.json",
        json!({ "coding-supervisor@coding-supervisor": true }),
    );
    let out = install(&project, &home, "codex", &[]);
    assert!(!out.contains("plugin is enabled"), "{out}");
    assert!(
        project.join(".codex").join("config.toml").is_file(),
        "{out}"
    );
}
