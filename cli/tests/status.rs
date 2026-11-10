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
    lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"status fixture","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2}),false,None,None).unwrap();
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

/// K9: with `SUPERVISOR_STATE_DIR` set, status names the default-rule state directory for the
/// same project when one exists, because a process without the variable would use it.
#[test]
fn status_names_a_second_state_directory_for_the_project() {
    let root = std::env::temp_dir().join(format!("sup-status-shadow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (project, env_state, base) = (root.join("project"), root.join("env"), root.join("base"));
    std::fs::create_dir_all(&project).unwrap();
    let key = supervisor_claude_code::location::project_key(&project);
    let shadow = base.join("coding-supervisor").join("state").join(&key);
    std::fs::create_dir_all(&shadow).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_supervisor"))
        .args(["status", "--json", "--project"])
        .arg(&project)
        .env("SUPERVISOR_STATE_DIR", &env_state)
        .env("LOCALAPPDATA", &base)
        .env("XDG_STATE_HOME", &base)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let other = report["other_state"]
        .as_str()
        .unwrap_or_default()
        .replace('\\', "/");
    assert!(
        other.ends_with(&format!("coding-supervisor/state/{key}")),
        "{report}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// M2: status reports the requested mode and the one the harness can enforce: only Claude Code
/// with the refusing hooks configured enforces; anywhere else a requested enforce runs as audit.
#[test]
fn status_reports_requested_and_effective_mode() {
    let root = std::env::temp_dir().join(format!("sup-status-mode-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("project"), root.join("state"));
    fs::create_dir_all(project.join(".claude")).unwrap();
    let status = |harness: &str| -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_supervisor"))
            .args(["status", "--json", "--harness", harness, "--project"])
            .arg(&project)
            .env("SUPERVISOR_STATE_DIR", &state)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    for harness in ["codex", "claude-code"] {
        let report = status(harness);
        assert_eq!(report["requested_level"], "enforce", "{harness}: {report}");
        assert_eq!(report["effective_level"], "audit", "{harness}: {report}");
        assert!(
            report["level_note"]
                .as_str()
                .unwrap_or_default()
                .contains("cannot refuse"),
            "{harness}: {report}"
        );
    }
    let hook = json!([{"hooks":[{"type":"command","command":"supervisor hook"}]}]);
    fs::write(
        project.join(".claude/settings.json"),
        json!({"hooks":{"PreToolUse":hook,"Stop":hook,"SubagentStop":hook}}).to_string(),
    )
    .unwrap();
    let report = status("claude-code");
    assert_eq!(report["effective_level"], "enforce", "{report}");
    assert!(report["level_note"].is_null(), "{report}");
    let _ = fs::remove_dir_all(&root);
}

/// WP4: a task with limits shows them with turns used and elapsed seconds; one without shows none.
#[test]
fn status_shows_task_limits_and_their_use() {
    let f = Fixture::new("limits");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    let started = lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"limits fixture","criteria":[{"id":"C2","check":"test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2,"limits":{"turns":40,"wall_clock_s":600}}),false,None,None).unwrap();
    let task = root
        .join("tasks")
        .join(started["task_id"].as_str().unwrap());
    fs::write(
        task.join("events.jsonl"),
        "{\"v\":1,\"event\":\"PreToolUse\"}\n{\"v\":1,\"event\":\"Stop\"}\n{\"v\":1,\"event\":\"PreToolUse\"}\n",
    )
    .unwrap();
    let report = f.status("claude-code");
    let limits = &report["task"]["limits"];
    assert_eq!(limits["turns"], 40, "{report}");
    assert_eq!(limits["wall_clock_s"], 600, "{report}");
    assert_eq!(limits["turns_used"], 2, "{report}");
    assert!(
        limits["elapsed_s"].as_u64().is_some_and(|s| s < 600),
        "{report}"
    );
    assert_eq!(limits["reached"], Value::Null, "{report}");
    let human = Command::new(BIN)
        .args(["status", "--harness", "claude-code", "--project"])
        .arg(f.0.join("project"))
        .env("SUPERVISOR_STATE_DIR", f.0.join("state"))
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(
        human.contains("limits: 2 of 40 turns used; elapsed "),
        "{human}"
    );
    assert!(human.contains(" s of 600 s"), "{human}");
    for word in ["spend", "cost"] {
        assert!(!human.to_lowercase().contains(word), "{human}");
    }

    let plain = Fixture::new("no-limits");
    let root = PathBuf::from(
        plain.status("claude-code")["project_state"]
            .as_str()
            .unwrap(),
    );
    lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"no limits","criteria":[{"id":"C2","check":"test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2}),false,None,None).unwrap();
    assert_eq!(plain.status("claude-code")["task"].get("limits"), None);
}

/// WP4 review: a clock earlier than `approval.at` is named, not shown as a silent 0 s elapsed.
#[test]
fn status_names_a_clock_behind_the_approval_time() {
    let f = Fixture::new("clock-behind");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    let started = lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"clock fixture","criteria":[{"id":"C2","check":"test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2,"limits":{"wall_clock_s":600}}),false,None,None).unwrap();
    let path = root
        .join("tasks")
        .join(started["task_id"].as_str().unwrap())
        .join("contract.json");
    let mut contract: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let at = contract["approval"]["at"].as_u64().unwrap();
    contract["approval"]["at"] = (at + 1_000_000).into();
    fs::write(&path, format!("{contract}\n")).unwrap();
    let report = f.status("claude-code");
    assert_eq!(
        report["task"]["limits"]["clock_behind_approval"], true,
        "{report}"
    );
    let human = Command::new(BIN)
        .args(["status", "--harness", "claude-code", "--project"])
        .arg(f.0.join("project"))
        .env("SUPERVISOR_STATE_DIR", f.0.join("state"))
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("clock behind approval"), "{human}");
}

/// WP4 review round 3: without a turn limit the journal is not read, so the line shows only the
/// wall clock.
#[test]
fn status_shows_a_wall_clock_only_limit_without_a_turn_count() {
    let f = Fixture::new("wall-only");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"wall fixture","criteria":[{"id":"C2","check":"test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2,"limits":{"wall_clock_s":600}}),false,None,None).unwrap();
    let report = f.status("claude-code");
    assert_eq!(
        report["task"]["limits"]["turns_used"],
        Value::Null,
        "{report}"
    );
    let human = Command::new(BIN)
        .args(["status", "--harness", "claude-code", "--project"])
        .arg(f.0.join("project"))
        .env("SUPERVISOR_STATE_DIR", f.0.join("state"))
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    let line = human
        .lines()
        .find(|l| l.starts_with("limits: "))
        .unwrap_or_else(|| panic!("{human}"));
    assert!(line.starts_with("limits: elapsed "), "{line}");
    assert!(line.ends_with(" s of 600 s"), "{line}");
    assert!(!line.contains("turns"), "{line}");
}

/// WP5: status shows the review policy and the latest review for the task's candidate.
#[test]
fn status_shows_the_review_policy_and_the_latest_review() {
    use coding_supervisor::{
        CandidateId, Checkpoint, ReviewDecision, ReviewProvenance, ReviewRecord, TaskState,
    };
    use supervisor_claude_code::state::{Gate, StoredReview, read_contract, save_gate};
    let f = Fixture::new("review");
    let root = PathBuf::from(f.status("claude-code")["project_state"].as_str().unwrap());
    let started = lifecycle::start_as(&root, &supervisor_claude_code::state::Writer::task_start(None),&json!({"id":"fixture","goal":"review fixture","criteria":[{"id":"C2","check":"test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2,"review":{"required":true,"independent":true}}),false,None,None).unwrap();
    let report = f.status("claude-code");
    assert_eq!(
        report["task"]["review"],
        json!({"required":true,"independent":true,"latest":null}),
        "{report}"
    );
    let task = root
        .join("tasks")
        .join(started["task_id"].as_str().unwrap());
    let (_, digest) = read_contract(&task).unwrap();
    let candidate = CandidateId {
        base_commit: "base".into(),
        dirty_digest: "one".into(),
    };
    let mut gate = Gate::new(
        "fixture".to_owned(),
        Checkpoint {
            state: TaskState::Verifying,
            candidate: Some(candidate.clone()),
            open_findings: vec![],
            consumed_repair_rounds: 0,
            next_action: String::new(),
        },
    );
    gate.reviews.push(StoredReview {
        review: ReviewRecord {
            reviewer: "r1".into(),
            model: None,
            decision: ReviewDecision::Rejected,
            findings: vec!["secret-finding-text".into()],
            at: 1_000,
            candidate,
            provenance: ReviewProvenance::Observed,
        },
        tree: None,
    });
    save_gate(&task, &gate, &digest).unwrap();
    let report = f.status("claude-code");
    assert_eq!(
        report["task"]["review"]["latest"],
        json!({"reviewer":"r1","model":null,"decision":"rejected","provenance":"observed","findings":1,"at":1000}),
        "{report}"
    );
    let human = Command::new(BIN)
        .args(["status", "--harness", "claude-code", "--project"])
        .arg(f.0.join("project"))
        .env("SUPERVISOR_STATE_DIR", f.0.join("state"))
        .output()
        .unwrap();
    let human = String::from_utf8_lossy(&human.stdout);
    let line = human
        .lines()
        .find(|l| l.starts_with("review: "))
        .unwrap_or_else(|| panic!("{human}"));
    assert!(line.contains("required, independent"), "{line}");
    assert!(line.contains("rejected by r1"), "{line}");
    assert!(line.contains("model not reported"), "{line}");
    assert!(line.contains("1 finding"), "{line}");
    assert!(!human.contains("secret-finding-text"), "{human}");
}
