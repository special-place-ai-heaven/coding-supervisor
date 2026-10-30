//! Read-only status must expose pause without claiming hook loading.
use std::fs;
use std::process::Command;

use serde_json::{Value, json};
use supervisor_claude_code::{autonomy, location};

#[test]
fn paused_autonomous_task_is_visible_in_status() {
    let temp = std::env::temp_dir().join(format!("sup-auto-status-{}", std::process::id()));
    let project = temp.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("Cargo.toml"), "[workspace]\n").unwrap();
    let state = temp.join("state");
    let (root, _) = location::resolve_state_dir(&project, &|key| {
        (key == "SUPERVISOR_STATE_DIR").then(|| state.to_string_lossy().into_owned())
    });
    location::prepare_state(&root, &project).unwrap();
    let mut event = autonomy::record_intent(
        &root,
        &project,
        &json!({"session_id":"status","prompt_id":"one","prompt":"Add a feature"}),
    )
    .unwrap();
    event["action"] = "start".into();
    event["contract"] = json!({"id":"status","goal":"Add a feature","scope":["src/"],
        "criteria":[{"id":"C1","check":"feature","command":"cargo test","mandatory":true}],
        "forbidden_effects":[],"max_repair_rounds":2});
    let started = autonomy::propose(&root, &project, &event).unwrap();
    let mut event = autonomy::record_intent(
        &root,
        &project,
        &json!({"session_id":"status","prompt_id":"two","prompt":"pause"}),
    )
    .unwrap();
    event["action"] = "pause".into();
    event["task_id"] = started["task_id"].clone();
    autonomy::propose(&root, &project, &event).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_supervisor"))
        .args(["status", "--harness", "codex", "--json", "--project"])
        .arg(&project)
        .env("SUPERVISOR_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["task"]["autonomous"], true);
    assert_eq!(report["task"]["lifecycle_state"], "paused");
    assert_eq!(report["mode"], "advisory");
    fs::remove_dir_all(temp).unwrap();
}
