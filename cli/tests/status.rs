use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};
use supervisor_claude_code::lifecycle;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("supervisor-status-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("project")).unwrap();
        Self(dir)
    }
    fn status(&self, harness: &str) -> Value {
        let output = Command::new(BIN)
            .args(["status", "--harness", harness, "--json", "--project"])
            .arg(self.0.join("project"))
            .env("SUPERVISOR_STATE_DIR", self.0.join("state"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn register(&self, harness: &str) {
        let project = self.0.join("project");
        if harness == "codex" {
            fs::create_dir_all(project.join(".codex")).unwrap();
            fs::write(
                project.join(".codex/config.toml"),
                "[mcp_servers.supervisor]\ncommand = \"supervisor\"\n",
            )
            .unwrap();
        } else {
            fs::write(
                project.join(".mcp.json"),
                json!({"mcpServers":{"supervisor":{"command":BIN}}}).to_string(),
            )
            .unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fresh_and_registered_unloaded_remain_inactive_advisory() {
    let f = Fixture::new("fresh");
    let fresh = f.status("claude-code");
    assert_eq!(fresh["registration"]["state"], "not_configured");
    assert_eq!(fresh["task"]["state"], "inactive");
    assert_eq!(fresh["mode"], "advisory");
    assert_eq!(fresh["reporting_binary"]["running"], true);
    f.register("claude-code");
    let registered = f.status("claude-code");
    assert_eq!(registered["registration"]["state"], "configured");
    assert_eq!(registered["registration"]["configured_version"], "unknown");
    assert_eq!(registered["connection"]["state"], "unobserved");
    assert_eq!(registered["task"]["state"], "inactive");
}

#[test]
fn connected_inactive_does_not_claim_model_loaded_or_activate_task() {
    let f = Fixture::new("connected");
    f.register("claude-code");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    lifecycle::observe_connection(&root, "mcp-initialize").unwrap();
    let connected = f.status("claude-code");
    assert_eq!(connected["connection"]["state"], "observed");
    assert_eq!(connected["connection"]["model_session_loaded"], "unknown");
    assert_eq!(connected["task"]["state"], "inactive");
    assert_eq!(connected["mode"], "advisory");
}

#[test]
fn active_claude_is_conditional_on_hooks_and_codex_stays_advisory() {
    let f = Fixture::new("active");
    f.register("claude-code");
    f.register("codex");
    let project = f.0.join("project");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let mut hooks = serde_json::Map::new();
    for event in ["PreToolUse", "Stop", "SubagentStop"] {
        hooks.insert(
            event.to_owned(),
            json!([{"hooks":[{"type":"command","command":"supervisor hook"}]}]),
        );
    }
    fs::write(
        project.join(".claude/settings.json"),
        json!({"hooks":hooks}).to_string(),
    )
    .unwrap();
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    lifecycle::start(&root,&json!({"id":"fixture","goal":"status fixture","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2}),false,None,None).unwrap();
    lifecycle::observe_connection(&root, "claude-hook").unwrap();
    let claude = f.status("claude-code");
    assert_eq!(claude["task"]["state"], "active");
    assert_eq!(claude["mode"], "enforcing_on_named_hook_paths_if_loaded");
    assert_eq!(claude["connection"]["model_session_loaded"], "unknown");
    let codex = f.status("codex");
    assert_eq!(codex["task"]["state"], "active");
    assert_eq!(codex["mode"], "advisory");
    assert_eq!(codex["connection"]["state"], "unobserved");
    assert_eq!(codex["coverage"]["completion"], json!([]));
}

#[test]
fn malformed_registration_never_echoes_contents() {
    let f = Fixture::new("redaction");
    fs::write(f.0.join("project/.mcp.json"), "SYNTHETIC_PRIVATE_CONFIG").unwrap();
    let report = f.status("claude-code");
    assert_eq!(report["registration"]["state"], "unknown");
    assert!(!report.to_string().contains("SYNTHETIC_PRIVATE_CONFIG"));
}

#[test]
fn legacy_marker_requires_migration_and_echo_is_not_a_hook_registration() {
    let f = Fixture::new("legacy");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    fs::write(root.join("legacy-location.json"), "{}").unwrap();
    let project = f.0.join("project");
    fs::create_dir_all(project.join(".claude")).unwrap();
    fs::write(
        project.join(".claude/settings.json"),
        json!({"hooks":{"Stop":[{"hooks":[{"command":"echo supervisor hook"}]}]}}).to_string(),
    )
    .unwrap();
    let report = f.status("claude-code");
    assert_eq!(report["task"]["state"], "migration_required");
    assert_eq!(report["hooks"]["state"], "not_configured");
}

#[test]
fn restricted_noncommand_and_disabled_hooks_never_claim_full_coverage() {
    let f = Fixture::new("restricted");
    let project = f.0.join("project");
    fs::create_dir_all(project.join(".claude")).unwrap();
    let settings = project.join(".claude/settings.json");
    for pre in [
        json!({"matcher":"Read","hooks":[{"type":"command","command":"supervisor hook"}]}),
        json!({"hooks":[{"type":"prompt","command":"supervisor hook"}]}),
        json!({"matcher":17,"hooks":[{"type":"command","command":"supervisor hook"}]}),
        json!({"hooks":[{"type":"command","command":"supervisor hook","async":true}]}),
    ] {
        fs::write(&settings,json!({"hooks":{"PreToolUse":[pre],"Stop":[{"hooks":[{"type":"command","command":"supervisor hook"}]}],"SubagentStop":[{"hooks":[{"type":"command","command":"supervisor hook"}]}]}}).to_string()).unwrap();
        let report = f.status("claude-code");
        assert_eq!(report["hooks"]["state"], "partial");
        assert!(
            !report["hooks"]["events"]
                .as_array()
                .unwrap()
                .contains(&json!("PreToolUse"))
        );
        assert!(
            !report["coverage"]["completion"]
                .as_array()
                .unwrap()
                .contains(&json!("PreToolUse denial"))
        );
        assert_eq!(report["mode"], "advisory");
    }
    fs::write(
        project.join(".claude/settings.local.json"),
        json!({"disableAllHooks":true}).to_string(),
    )
    .unwrap();
    let report = f.status("claude-code");
    assert_eq!(report["hooks"]["state"], "disabled");
    assert_eq!(report["coverage"]["completion"], json!([]));
}
