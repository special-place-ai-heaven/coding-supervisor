//! How many git processes one hook call starts. Each costs tens of milliseconds on Windows, in
//! every tool call of a supervised session, so the count is pinned (measured 2026-11-03: a Stop
//! started 19, 16 of them the same `rev-parse --show-toplevel`).

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::json;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

#[test]
fn a_hook_call_starts_few_git_processes() {
    let root = std::env::temp_dir().join(format!("sup-git-calls-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("project"), root.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("README"), "one\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "README"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "-m",
            "one",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&project)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let contract = root.join("contract.json");
    fs::write(&contract, json!({"id":"count","goal":"count git calls","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
    let started = Command::new(BIN)
        .args(["task", "start", "--contract"])
        .arg(&contract)
        .arg("--project")
        .arg(&project)
        .env("SUPERVISOR_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    let calls = |name: &str, mut payload: serde_json::Value| -> usize {
        let trace: PathBuf = root.join(format!("{name}.trace"));
        payload["cwd"] = project.to_str().unwrap().into();
        payload["session_id"] = "s".into();
        let mut hook = Command::new(BIN)
            .arg("hook")
            .env("SUPERVISOR_STATE_DIR", &state)
            .env("SUPERVISOR_GIT_TRACE", &trace)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        hook.stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        assert!(hook.wait().unwrap().success());
        let lines = fs::read_to_string(&trace).unwrap_or_default();
        eprintln!("{name}:\n{lines}");
        lines.lines().count()
    };
    let pre = calls(
        "pre",
        json!({"hook_event_name":"PreToolUse","tool_use_id":"t1","tool_name":"Bash","tool_input":{"command":"cargo test"}}),
    );
    let stop = calls(
        "stop",
        json!({"hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"Done, all tests pass."}),
    );
    // One root lookup, one rev-parse for HEAD and the prefix, one status. A PreToolUse of a
    // criterion command adds the three gits of the worktree tree id (G9: index path, add,
    // write-tree); Stop computes it only when an exact candidate match fails, so it adds none.
    assert!(pre <= 6, "PreToolUse started {pre} git processes");
    assert!(stop <= 3, "Stop started {stop} git processes");
    let _ = fs::remove_dir_all(&root);
}
