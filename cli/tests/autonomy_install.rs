//! Installer coverage only; loading hooks in a live Claude model is deferred.
use std::fs;
use std::process::Command;

use serde_json::Value;

#[test]
fn installation_observes_all_tool_routes_for_autonomous_intent() {
    let root = std::env::temp_dir().join(format!("sup-autonomy-install-{}", std::process::id()));
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_supervisor"))
        .args(["install", "--harness", "claude-code", "--project"])
        .arg(&project)
        .arg("--home")
        .arg(root.join("home"))
        .env("SUPERVISOR_STATE_DIR", root.join("state"))
        .output()
        .unwrap();
    assert!(output.status.success(), "installer failed");
    let settings: Value =
        serde_json::from_slice(&fs::read(project.join(".claude/settings.json")).unwrap()).unwrap();
    for event in ["PreToolUse", "PostToolUse", "PostToolUseFailure"] {
        let groups = settings["hooks"][event].as_array().unwrap();
        assert!(
            groups.iter().any(|group| {
                group.get("matcher").is_none()
                    || matches!(group["matcher"].as_str(), Some("" | "*"))
            }),
            "{event} omits tool routes needed for pending-intent gating and observation"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
