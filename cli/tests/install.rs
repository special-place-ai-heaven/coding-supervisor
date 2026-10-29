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
        assert!(second.contains("no changes"), "{harness}: {second}");
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

/// Lab repo, 2026-10-29: settings already held the gate's hooks from another binary path, and
/// install appended a second set (12 entries). Our hooks are recognised by the `<path> hook`
/// shape, whatever the path, and replaced.
#[test]
fn claude_settings_replace_our_hooks_from_another_binary_path() {
    let (project, home) = dirs("settings-moved");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let settings = project.join(".claude").join("settings.json");
    let events = [
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "SubagentStop",
        "Stop",
    ];
    // The broken state seen live: two old paths per event (one quoted, one an npm install),
    // and one foreign hook that must survive.
    let mut hooks = serde_json::Map::new();
    for event in events {
        hooks.insert(
            event.into(),
            json!([
                { "hooks": [{ "type": "command", "command": "\"C:/Program Files/old/supervisor.exe\" hook" }] },
                { "hooks": [{ "type": "command", "command": "/usr/lib/node_modules/coding-supervisor-linux-x64/bin/supervisor hook" }] }
            ]),
        );
    }
    hooks["Stop"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "hooks": [{ "type": "command", "command": "foreign-stop hook-like" }] }));
    fs::write(&settings, json!({ "hooks": hooks }).to_string()).unwrap();
    fs::write(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"supervisor":{"type":"stdio","command":"C:/old/supervisor.exe","args":["mcp"]}}}"#,
    )
    .unwrap();

    install(&project, &home, "claude-code", &[]);

    let v = read_json(&settings);
    for event in events {
        let commands: Vec<&str> = v["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .filter_map(|h| h["command"].as_str())
            .filter(|c| c.ends_with(" hook"))
            .collect();
        assert_eq!(
            commands,
            [OUR_HOOK],
            "{event}: exactly one gate hook, the new path"
        );
    }
    let stop = v["hooks"]["Stop"].to_string();
    assert!(stop.contains("foreign-stop hook-like"), "{stop}");
    assert_eq!(
        read_json(&project.join(".mcp.json"))["mcpServers"]["supervisor"]["command"],
        FAKE_BINARY
    );

    // A second run with the same path changes nothing.
    let before = snapshot(&project);
    install(&project, &home, "claude-code", &[]);
    assert_eq!(snapshot(&project), before);
}
