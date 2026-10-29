//! `supervisor level`, `supervisor status` and the hook at level off, run as child processes with
//! the state root and the user config directory redirected into a temp directory.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use supervisor_claude_code::location::project_key;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

struct Temp {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
    config: PathBuf,
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn temp(name: &str) -> Temp {
    let root = std::env::temp_dir().join(format!("sup-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let t = Temp {
        project: root.join("proj"),
        state: root.join("state"),
        config: root.join("config"),
        root,
    };
    fs::create_dir_all(&t.project).unwrap();
    fs::create_dir_all(&t.state).unwrap();
    t
}

fn run(t: &Temp, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .env("SUPERVISOR_STATE_DIR", &t.state)
        .env("SUPERVISOR_CONFIG_DIR", &t.config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn ok(t: &Temp, args: &[&str]) -> String {
    let out = run(t, args, None);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
}
fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Every file under `dir` with its bytes, sorted by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            out.push((p.clone(), Vec::new()));
            out.extend(snapshot(&p));
        } else {
            out.push((p.clone(), fs::read(&p).unwrap()));
        }
    }
    out.sort();
    out
}

#[test]
fn level_off_writes_nothing_and_keeps_the_state_path_guard() {
    let t = temp("off");
    let home = path(&t.config);
    assert!(ok(&t, &["level", "off", "--home", home]).contains("level off written (global)"));
    fs::write(t.state.join("keep.txt"), "untouched").unwrap();
    let before = snapshot(&t.state);
    let cwd = path(&t.project);
    let payloads = [
        json!({"hook_event_name": "PreToolUse", "cwd": cwd, "session_id": "s1",
            "tool_name": "Write", "tool_input": {"file_path": format!("{cwd}/src/lib.rs")}}),
        json!({"hook_event_name": "PostToolUse", "cwd": cwd, "session_id": "s1",
            "tool_name": "Bash", "tool_input": {"command": "cargo test"}, "tool_use_id": "toolu_1",
            "tool_response": {"stdout": "ok"}}),
        json!({"hook_event_name": "Stop", "cwd": cwd, "session_id": "s1",
            "last_assistant_message": "Done, the task is complete.", "stop_hook_active": false}),
    ];
    for payload in &payloads {
        let out = run(&t, &["hook"], Some(&payload.to_string()));
        assert!(out.status.success());
        assert_eq!(stdout(&out), "", "{payload}");
    }
    // The guard still refuses a write into the (uncreated) state directory, and writes nothing.
    let state_dir = t.state.join(project_key(&t.project));
    let guarded = json!({"hook_event_name": "PreToolUse", "cwd": cwd, "session_id": "s1",
        "tool_name": "Write", "tool_input": {"file_path": path(&state_dir.join("level"))}});
    let out = stdout(&run(&t, &["hook"], Some(&guarded.to_string())));
    let decision: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(decision["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(snapshot(&t.state), before);
    assert!(!state_dir.exists());
    assert!(!t.project.join(".supervisor").exists());
}

#[test]
fn per_project_level_beats_the_global_one() {
    let t = temp("precedence");
    let (home, project) = (path(&t.config), path(&t.project));
    let show = ["level", "--project", project, "--home", home];
    assert_eq!(ok(&t, &show), "level: full (default)\n");
    assert!(ok(&t, &["level", "lite", "--home", home]).contains("(global)"));
    assert_eq!(ok(&t, &show), "level: lite (global)\n");
    assert!(
        ok(
            &t,
            &["level", "ultra", "--project", project, "--home", home]
        )
        .contains("(project)")
    );
    assert_eq!(ok(&t, &show), "level: ultra (project)\n");
    assert_eq!(
        fs::read_to_string(t.state.join(project_key(&t.project)).join("level")).unwrap(),
        "ultra\n"
    );
    assert_eq!(
        fs::read_to_string(t.config.join("level")).unwrap(),
        "lite\n"
    );

    // An unknown word is a usage error; an unknown word in a level file is an error.
    assert_eq!(
        run(&t, &["level", "strict", "--home", home], None)
            .status
            .code(),
        Some(2)
    );
    fs::write(t.config.join("level"), "strict\n").unwrap();
    let other = t.root.join("other");
    fs::create_dir_all(&other).unwrap();
    let out = run(
        &t,
        &["level", "--project", path(&other), "--home", home],
        None,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown level \"strict\""));
}

#[test]
fn status_counts_completions_and_names_the_refusal_reasons() {
    let t = temp("status");
    let (home, project) = (path(&t.config), path(&t.project));
    let empty = ok(&t, &["status", "--project", project, "--home", home]);
    assert!(empty.contains("level: full (default)"), "{empty}");
    assert!(empty.contains("contract: none"), "{empty}");
    assert!(empty.contains("journal: none"), "{empty}");

    let dir = t.state.join(project_key(&t.project));
    fs::create_dir_all(&dir).unwrap();
    let contract = json!({"id": "status-test", "goal": "g", "forbidden_effects": [], "scope": ["src/"],
        "max_repair_rounds": 2, "criteria": [
            {"id": "C1", "check": "quick", "mandatory": true, "command": "cargo test"},
            {"id": "C2", "check": "suite", "mandatory": true, "command": ["cargo test --workspace", "cargo nextest run"], "tier": "full"},
            {"id": "C3", "check": "lint", "mandatory": false, "command": "cargo clippy"}]});
    fs::write(dir.join("contract.json"), contract.to_string()).unwrap();
    let line = |event: &str, kind: &str, text: Option<&str>, accepted: bool, reasons: &[&str]| {
        json!({"v": 1, "event": event, "decision": {"kind": kind, "text": text},
            "accepted": accepted, "reasons": reasons})
        .to_string()
    };
    let never = "GATE refuses completion (refusal 1 of 3, harness cap 8): state verifying; missing C1; run `cargo test`: never ran; or report the task as not complete.";
    let journal = [
        line("Stop", "block", Some(never), false, &[]),
        line("Stop", "block", Some(never), false, &[]),
        line(
            "SubagentStop",
            "block",
            Some(
                "GATE: handback refused (refusal 1 of 3): stale C1. run `cargo test`: ran before your last change.",
            ),
            false,
            &[],
        ),
        line(
            "PreToolUse",
            "deny",
            Some("GATE: Write refused: task is completed"),
            false,
            &[],
        ),
        line(
            "Stop",
            "system",
            Some(never),
            false,
            &["level lite: reported, not blocked"],
        ),
        line("Stop", "none", None, true, &["completed"]),
        "not json".to_owned(),
    ];
    fs::write(dir.join("events.jsonl"), journal.join("\n") + "\n").unwrap();
    let text = ok(&t, &["status", "--project", project, "--home", home]);
    println!("{text}");
    let reason = "state verifying; missing C1; run `cargo test`: never ran; or report the task as not complete";
    for want in [
        "level: full (default)".to_owned(),
        "baseline failures: none (no gate state yet)".to_owned(),
        "  C1 | `cargo test` | quick | yes | yes".to_owned(),
        "  C2 | `cargo test --workspace` or `cargo nextest run` | full | yes | no".to_owned(),
        "  C3 | `cargo clippy` | quick | no | no".to_owned(),
        "completions accepted: 1".to_owned(),
        "completions refused: 3".to_owned(),
        "completions reported, not blocked (lite): 1".to_owned(),
        "journal lines not readable, skipped: 1".to_owned(),
        format!(
            "refusal reasons:\n  2x {reason}\n  1x stale C1\n  1x {reason} (lite, not blocked)"
        ),
    ] {
        assert!(text.contains(&want), "{want}\n---\n{text}");
    }
    assert!(!text.contains("task is completed"), "{text}");
    // At ultra the full-tier check is required.
    ok(
        &t,
        &["level", "ultra", "--project", project, "--home", home],
    );
    let text = ok(&t, &["status", "--project", project, "--home", home]);
    assert!(text.contains("level: ultra (project)"), "{text}");
    assert!(
        text.contains("  C2 | `cargo test --workspace` or `cargo nextest run` | full | yes | yes"),
        "{text}"
    );
}
