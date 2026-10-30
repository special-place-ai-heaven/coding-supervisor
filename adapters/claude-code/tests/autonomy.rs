use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use supervisor_claude_code::{autonomy, lifecycle, state};

static ID: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    project: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let project = std::env::temp_dir().join(format!(
            "supervisor-autonomy-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Self {
            root: project.join("state"),
            project,
        }
    }
    fn prompt(&self, id: &str, text: &str) -> Value {
        autonomy::record_prompt(
            &self.root,
            &self.project,
            &json!({"session_id":"session","prompt_id":id,"prompt":text}),
        )
        .unwrap()
    }
    fn apply(
        &self,
        event: &Value,
        action: &str,
        task: Option<&str>,
        contract: Option<Value>,
    ) -> Result<Value, String> {
        autonomy::propose(
            &self.root,
            &self.project,
            &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,"task_id":task,"contract":contract}),
        )
    }
    fn start(&self) -> String {
        self.apply(
            &self.prompt("first", "Implement a small change"),
            "start",
            None,
            Some(contract()),
        )
        .unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.project);
    }
}
fn contract() -> Value {
    json!({"id":"ordinary-task","goal":"implement requested change","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test -p fixture"}],"forbidden_effects":["publish"],"scope":["src/"],"max_repair_rounds":2})
}

#[test]
fn git_only_discovery_retains_previously_approved_custom_checks() {
    let f = Fixture::new();
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    fs::create_dir(f.project.join(".git")).unwrap();
    let mut approved = contract();
    approved["criteria"][0]["command"] = "python verify.py".into();
    lifecycle::start(&f.root, &approved, false, None, None).unwrap();
    let event = f.prompt("next", "Continue work with the approved checks");
    let checks = event["required_criteria"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["id"] == "AUTO_DIFF"));
    assert!(
        checks.iter().any(|c| c["id"] == "C2"),
        "git-only discovery lost previously approved custom verification"
    );
}

#[test]
fn cancelled_git_only_task_retains_approved_custom_floor_for_ordinary_successor() {
    let f = Fixture::new();
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    fs::create_dir(f.project.join(".git")).unwrap();
    let mut approved = contract();
    approved["criteria"][0]["command"] = "python verify.py".into();
    let original = lifecycle::start(&f.root, &approved, false, None, None).unwrap();
    let id = original["task_id"].as_str().unwrap();
    f.apply(
        &f.prompt("followup", "Continue the approved work"),
        "continue",
        Some(id),
        None,
    )
    .unwrap();
    let cancel = f.prompt("cancel", "cancel task");
    let cancelled = f.apply(&cancel, "cancel", Some(id), None).unwrap();
    assert!(lifecycle::active_task(&f.root).unwrap().is_none());
    let archived = f.root.join("archives").join(format!("{id}.json"));
    let archive_bytes = fs::read(&archived).unwrap();
    let old_task = f.root.join("tasks").join(id);
    let old_contract = fs::read(old_task.join("contract.json")).unwrap();
    let old_metadata = fs::read(old_task.join("autonomy.json")).unwrap();

    let event = f.prompt("successor", "Implement the next requested change");
    assert!(
        event["required_criteria"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "C2" && c["command"] == json!(["python verify.py"])),
        "cancellation lost the previously approved custom check floor"
    );
    let mut weakened = approved.clone();
    weakened["criteria"][0]["command"] = "echo skipped".into();
    assert!(
        f.apply(&event, "start", None, Some(weakened))
            .unwrap_err()
            .contains("immutable repository requirement floor")
    );
    let mut optional = approved.clone();
    optional["criteria"][0]["mandatory"] = false.into();
    optional["criteria"].as_array_mut().unwrap().push(json!({
        "id":"extra","check":"extra_check","command":"python extra.py","mandatory":true
    }));
    assert!(
        f.apply(&event, "start", None, Some(optional))
            .unwrap_err()
            .contains("immutable repository requirement floor")
    );
    // The approved parsed floor uses a list; the same command string remains equivalent.
    let mut next_contract = approved;
    next_contract["id"] = "successor-task".into();
    assert!(next_contract["criteria"][0]["command"].is_string());
    let successor = f.apply(&event, "start", None, Some(next_contract)).unwrap();
    assert_ne!(successor["task_id"], original["task_id"]);
    assert_eq!(
        f.apply(&cancel, "cancel", Some(id), None).unwrap(),
        cancelled
    );
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        successor["task_id"].as_str().unwrap()
    );
    assert_eq!(fs::read(archived).unwrap(), archive_bytes);
    assert_eq!(
        fs::read(old_task.join("contract.json")).unwrap(),
        old_contract
    );
    assert_eq!(
        fs::read(old_task.join("autonomy.json")).unwrap(),
        old_metadata
    );
    let successor_id = successor["task_id"].as_str().unwrap();
    f.apply(
        &f.prompt("cancel-successor", "cancel task"),
        "cancel",
        Some(successor_id),
        None,
    )
    .unwrap();
    let third = f.prompt("third", "Implement another requested change");
    let mut third_contract = contract();
    third_contract["criteria"][0]["command"] = "python verify.py".into();
    f.apply(&third, "start", None, Some(third_contract))
        .unwrap();
}

#[test]
fn discussion_stays_inactive_and_prompt_replay_is_stable_without_storing_prompt() {
    let f = Fixture::new();
    let event = f.prompt("discussion", "Explain ownership without editing");
    assert_eq!(
        f.prompt("discussion", "Explain ownership without editing"),
        event
    );
    assert!(autonomy::pending(&f.root, "session").unwrap().is_some());
    let response = f.apply(&event, "discussion", None, None).unwrap();
    assert!(lifecycle::active_task(&f.root).unwrap().is_none());
    assert!(autonomy::pending(&f.root, "session").unwrap().is_none());
    assert!(autonomy::discussion_turn(&f.root, "session").unwrap());
    assert_eq!(f.apply(&event, "discussion", None, None).unwrap(), response);
    assert!(
        !fs::read_to_string(f.root.join("intents.json"))
            .unwrap()
            .contains("Explain ownership")
    );
    assert!(
        autonomy::record_prompt(
            &f.root,
            &f.project,
            &json!({"session_id":"session","prompt_id":"discussion","prompt":"changed"})
        )
        .is_err()
    );
}

#[test]
fn start_freezes_checks_before_manifest_edits_and_cannot_drop_them() {
    let f = Fixture::new();
    let event = f.prompt("first", "Implement feature");
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    let response = f.apply(&event, "start", None, Some(contract())).unwrap();
    let id = response["task_id"].as_str().unwrap();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    assert!(autonomy::is_autonomous_task(&active.dir).unwrap());
    let text = fs::read_to_string(active.dir.join("contract.json")).unwrap();
    assert!(text.contains("cargo test --workspace"));
    let mut weakened: Value = serde_json::from_str(&text).unwrap();
    weakened["criteria"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "AUTO_TEST");
    assert!(
        f.apply(
            &f.prompt("weaken", "Continue working"),
            "revise",
            Some(id),
            Some(weakened)
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(active.dir.join("contract.json")).unwrap(),
        text
    );
}

#[test]
fn unfinished_task_is_retained_followups_preserve_id_and_cancel_needs_user_control() {
    let f = Fixture::new();
    let id = f.start();
    assert!(
        f.apply(
            &f.prompt("new", "Implement another feature"),
            "start",
            None,
            Some(contract())
        )
        .is_err()
    );
    assert_eq!(
        f.apply(
            &f.prompt("followup", "Also fix its test"),
            "continue",
            Some(&id),
            None
        )
        .unwrap()["task_id"],
        id
    );
    assert!(
        f.apply(
            &f.prompt("ambiguous", "This is difficult"),
            "cancel",
            Some(&id),
            None
        )
        .is_err()
    );
    f.apply(&f.prompt("pause", "pause task"), "pause", Some(&id), None)
        .unwrap();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    assert!(autonomy::is_paused(&active.dir).unwrap());
    f.apply(
        &f.prompt("resume", "resume task"),
        "resume",
        Some(&id),
        None,
    )
    .unwrap();
    assert!(!autonomy::is_paused(&active.dir).unwrap());
    f.apply(
        &f.prompt("cancel", "cancel task"),
        "cancel",
        Some(&id),
        None,
    )
    .unwrap();
    assert!(lifecycle::active_task(&f.root).unwrap().is_none());
    assert!(active.dir.join("autonomy.json").exists());
    assert!(f.root.join("archives").join(format!("{id}.json")).exists());
}

#[test]
fn scope_paths_and_untrusted_mutation_are_rejected() {
    let f = Fixture::new();
    let id = f.start();
    for path in [
        "../escape",
        "/absolute",
        "C:/absolute",
        "src/../escape",
        "src\\..\\escape",
    ] {
        let mut c = contract();
        c["scope"] = json!([path]);
        assert!(
            f.apply(
                &f.prompt(&format!("bad-{path}"), "Change scope"),
                "revise",
                Some(&id),
                Some(c)
            )
            .is_err()
        );
    }
    let pending = f.prompt("settle", "Explain current state");
    f.apply(&pending, "discussion", None, None).unwrap();
    let asserted = autonomy::record_intent(
        &f.root,
        &f.project,
        &json!({"session_id":"asserted-session","prompt_id":"forged","prompt":"cancel task"}),
    )
    .unwrap();
    assert!(f.apply(&asserted, "cancel", Some(&id), None).is_err());
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        id
    );
    assert!(lifecycle::start(&f.root, &contract(), true, Some("cancelled"), Some(&id)).is_err());
    assert!(lifecycle::deactivate(&f.root, &id, Some("cancelled")).is_err());
    assert!(lifecycle::rebind(&f.root, &id, "intruder").is_err());
}

#[test]
fn completed_rollover_keeps_old_bytes_and_rebinds_session() {
    let f = Fixture::new();
    let initial = autonomy::record_intent(
        &f.root,
        &f.project,
        &json!({"session_id":"session","prompt_id":"first","prompt":"Implement a small change"}),
    )
    .unwrap();
    let first = f.apply(&initial, "start", None, Some(contract())).unwrap();
    let id = first["task_id"].as_str().unwrap().to_owned();
    lifecycle::with_task(&f.root, Some(&id), None, None, |dir| {
        let (_, digest) = state::read_contract(dir)?;
        let gate = state::Gate::new(
            "ordinary-task".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Completed,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 1,
                next_action: "done".into(),
            },
        );
        state::save_gate(dir, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
    let old = lifecycle::active_task(&f.root).unwrap().unwrap().dir;
    let before = fs::read(old.join("gate.json")).unwrap();
    let event = f.prompt("second", "Implement next feature");
    let result = f.apply(&event, "start", None, Some(contract())).unwrap();
    let second = result["task_id"].as_str().unwrap();
    assert_ne!(second, id);
    assert_eq!(fs::read(old.join("gate.json")).unwrap(), before);
    assert_eq!(
        f.apply(&event, "start", None, Some(contract())).unwrap(),
        result
    );
    lifecycle::with_task(&f.root, Some(second), Some("session"), None, |_| {
        Ok(Value::Null)
    })
    .unwrap();
    assert_eq!(
        f.apply(&initial, "start", None, Some(contract())).unwrap(),
        first
    );
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        second
    );
    let bindings: Value =
        serde_json::from_str(&fs::read_to_string(f.root.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(bindings["session"], second);
}

#[test]
fn root_success_then_nested_success_allows_observed_completion_and_rollover() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Fixed;
    impl CandidateProvider for Fixed {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "candidate".into(),
                },
                provenance: "rollover context fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    fn execute(program: &str, args: &[&str], cwd: &std::path::Path) -> std::process::Output {
        let mut command = std::process::Command::new(program);
        command.args(args).current_dir(cwd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.output().unwrap()
    }
    let f = Fixture::new();
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    assert!(
        execute("git", &["init", "--quiet"], &f.project)
            .status
            .success()
    );
    let nested = f.project.join("src");
    fs::create_dir_all(&nested).unwrap();
    for cwd in [&f.project, &nested] {
        fs::write(
            cwd.join("check.py"),
            "from pathlib import Path\nPath('executed-cwd.txt').write_text(str(Path.cwd()))\n",
        )
        .unwrap();
    }
    assert!(
        execute("git", &["add", "check.py", "src/check.py"], &f.project)
            .status
            .success()
    );
    assert!(
        execute(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture"
            ],
            &f.project
        )
        .status
        .success()
    );
    let linked = f.project.join("linked");
    assert!(
        execute(
            "git",
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                linked.to_str().unwrap()
            ],
            &f.project
        )
        .status
        .success()
    );
    let mut approved = contract();
    approved["criteria"][0]["command"] = "python check.py".into();
    let first = lifecycle::start(&f.root, &approved, false, None, None).unwrap();
    let first_id = first["task_id"].as_str().unwrap();
    f.apply(
        &f.prompt("adopt", "Continue implementing the requested behavior"),
        "continue",
        Some(first_id),
        None,
    )
    .unwrap();
    let mut env = Env {
        state_dir: f.root.clone(),
        state_location: "test",
        candidate: Box::new(Fixed),
        clock: || 100,
    };
    for (id, cwd) in [
        ("linked", &linked),
        ("root", &f.project),
        ("nested", &nested),
    ] {
        let mut event = json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","agent_id":"main","tool_use_id":id,
            "tool_name":"mcp__terminal_commander__run_and_watch","tool_input":{"argv":["python","check.py"],"cwd":cwd}});
        assert_eq!(handle(&event, &mut env), Decision::None);
        let actual = execute("python", &["check.py"], cwd);
        assert!(actual.status.success());
        assert_eq!(
            PathBuf::from(fs::read_to_string(cwd.join("executed-cwd.txt")).unwrap())
                .canonicalize()
                .unwrap(),
            cwd.canonicalize().unwrap()
        );
        event["hook_event_name"] = "PostToolUse".into();
        event["tool_response"] = json!({"job_id":format!("job-{id}"),"bucket_id":"bucket","state":"exited","exit_code":actual.status.code(),"complete":true,"outcome_trust":"observed","degraded":false,"pipeline_exit_masked":false});
        assert_eq!(handle(&event, &mut env), Decision::None);
        if id == "linked" {
            assert!(matches!(
                handle(
                    &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","agent_id":"main","last_assistant_message":"Done."}),
                    &mut env
                ),
                Decision::Block(_)
            ));
            let active = lifecycle::active_task(&f.root).unwrap().unwrap();
            let gate: Value =
                serde_json::from_slice(&fs::read(active.dir.join("gate.json")).unwrap()).unwrap();
            assert_ne!(gate["state"], "completed");
            assert!(gate["receipts"].as_array().unwrap().is_empty());
        }
    }
    assert_eq!(
        handle(
            &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","agent_id":"main","last_assistant_message":"Done."}),
            &mut env
        ),
        Decision::None
    );
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let gate: Value =
        serde_json::from_slice(&fs::read(active.dir.join("gate.json")).unwrap()).unwrap();
    assert_eq!(gate["state"], "completed");
    assert!(
        matches!(handle(&json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","agent_id":"main","tool_use_id":"nested-after-completion",
        "tool_name":"mcp__terminal_commander__run_and_watch","tool_input":{"argv":["python","check.py"],"cwd":nested}}), &mut env), Decision::Deny(reason) if reason.contains("completed"))
    );
    let rows = fs::read_to_string(active.dir.join("runs.jsonl")).unwrap();
    let nested_row = rows
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| {
            row["kind"] == "completed"
                && row["cwd"].as_str().is_some_and(|cwd| cwd.ends_with("/src"))
        })
        .unwrap();
    assert_eq!(
        nested_row["execution_root"],
        supervisor_claude_code::location::canonical_project(&f.project)
    );
    let before = fs::read(active.dir.join("gate.json")).unwrap();
    let next = f
        .apply(
            &f.prompt("successor", "Implement the next ordinary task"),
            "start",
            None,
            Some(approved),
        )
        .unwrap();
    assert_ne!(next["task_id"], first["task_id"]);
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        next["task_id"].as_str().unwrap()
    );
    assert!(
        f.root
            .join("archives")
            .join(format!("{first_id}.json"))
            .is_file()
    );
    assert_eq!(fs::read(active.dir.join("gate.json")).unwrap(), before);
}

#[test]
fn nonce_worktree_session_and_superseded_events_cannot_authorize_actions() {
    let f = Fixture::new();
    let event = f.prompt("first", "Implement feature");
    let other = Fixture::new();
    assert!(autonomy::propose(&f.root, &other.project, &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":"start","contract":contract()})).is_err());
    assert!(autonomy::propose(&f.root, &f.project, &json!({"event_id":event["event_id"],"nonce":"forged","prompt_hash":event["prompt_hash"],"action":"start","contract":contract()})).is_err());
    let newer = f.prompt("second", "Implement current request");
    assert!(f.apply(&event, "start", None, Some(contract())).is_err());
    assert_eq!(f.prompt("first", "Implement feature"), event);
    assert_eq!(
        autonomy::pending(&f.root, "session").unwrap().unwrap()["event_id"],
        newer["event_id"]
    );
    let result = f.apply(&newer, "start", None, Some(contract())).unwrap();
    let intruder = autonomy::record_prompt(
        &f.root,
        &f.project,
        &json!({"session_id":"other","prompt_id":"intruder","prompt":"cancel task"}),
    )
    .unwrap();
    assert!(
        f.apply(&intruder, "cancel", result["task_id"].as_str(), None)
            .is_err()
    );
}

#[test]
fn legacy_nested_pass_receipt_and_failing_root_check_cannot_complete_or_roll_over() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Fixed;
    impl CandidateProvider for Fixed {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "candidate".into(),
                },
                provenance: "legacy context fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    fn execute(program: &str, args: &[&str], cwd: &std::path::Path) -> std::process::Output {
        let mut command = std::process::Command::new(program);
        command.args(args).current_dir(cwd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.output().unwrap()
    }
    let f = Fixture::new();
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    assert!(
        execute("git", &["init", "--quiet"], &f.project)
            .status
            .success()
    );
    let nested = f.project.join("src");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        nested.join("check.py"),
        "from pathlib import Path\nPath('nested-passed.txt').write_text('passed')\n",
    )
    .unwrap();
    fs::write(f.project.join("root_check.py"), "from pathlib import Path\nPath('root-ran.txt').write_text('failed')\nraise SystemExit(23)\n").unwrap();
    let nested_result = execute("python", &["check.py"], &nested);
    assert_eq!(nested_result.status.code(), Some(0));
    assert!(nested.join("nested-passed.txt").is_file());
    let mut approved = contract();
    approved["criteria"][0]["command"] = "python check.py".into();
    approved["criteria"].as_array_mut().unwrap().push(json!({"id":"ROOT","check":"root_validation","mandatory":true,"command":"python root_check.py"}));
    let first = lifecycle::start(&f.root, &approved, false, None, None).unwrap();
    let first_id = first["task_id"].as_str().unwrap();
    f.apply(
        &f.prompt("adopt", "Continue implementing"),
        "continue",
        Some(first_id),
        None,
    )
    .unwrap();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let path = active.dir.join("autonomy.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    metadata["project"] = nested
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    // Recreate an old schema receipt snapshot in this disposable fixture, retaining its cwd.
    let nested_cwd = format!(
        "{}/src",
        supervisor_claude_code::location::canonical_project(&f.project)
    );
    let old_receipt = coding_supervisor::VerificationReceipt {
        criterion: coding_supervisor::CriterionId("C2".into()),
        candidate: coding_supervisor::CandidateId {
            base_commit: "base".into(),
            dirty_digest: "candidate".into(),
        },
        command: "python check.py".into(),
        cwd: nested_cwd.clone(),
        exit_code: nested_result.status.code(),
        output_ref: format!("observed:{first_id}:old-nested"),
        claim_supported: true,
        started_at: 1,
        finished_at: 2,
    };
    let mut old_rows = String::new();
    for kind in ["dispatch", "completed"] {
        old_rows.push_str(&format!("{}\n", json!({"kind":kind,"identity_schema":2,"dispatch_id":"old-nested","task_id":first_id,"agent_id":"main","command":"python check.py","cwd":nested_cwd,"candidate":{"base_commit":"base","dirty_digest":"candidate"},"exit_code":0,"t":1,"finished":2})));
    }
    let (_, digest) = state::read_contract(&active.dir).unwrap();
    let mut gate = state::Gate::new(
        "ordinary-task".into(),
        coding_supervisor::Checkpoint {
            state: coding_supervisor::TaskState::Verifying,
            candidate: Some(old_receipt.candidate.clone()),
            open_findings: vec![],
            consumed_repair_rounds: 0,
            next_action: "verify root context".into(),
        },
    );
    gate.receipts.push(old_receipt.clone());
    fs::write(active.dir.join("runs.jsonl"), &old_rows).unwrap();
    state::save_gate(&active.dir, &gate, &digest).unwrap();
    let mut env = Env {
        state_dir: f.root.clone(),
        state_location: "test",
        candidate: Box::new(Fixed),
        clock: || 100,
    };
    let mut event = json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","agent_id":"main","tool_use_id":"root-failure","tool_name":"mcp__terminal_commander__run_and_watch","tool_input":{"argv":["python","root_check.py"],"cwd":f.project}});
    assert_eq!(handle(&event, &mut env), Decision::None);
    let failed = execute("python", &["root_check.py"], &f.project);
    assert_eq!(failed.status.code(), Some(23));
    assert!(f.project.join("root-ran.txt").is_file());
    event["hook_event_name"] = "PostToolUse".into();
    event["tool_response"] = json!({"job_id":"root-failure","bucket_id":"bucket","state":"exited","exit_code":failed.status.code(),"complete":true,"outcome_trust":"observed","degraded":false,"pipeline_exit_masked":false});
    assert_eq!(handle(&event, &mut env), Decision::None);
    assert!(matches!(
        handle(
            &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","agent_id":"main","last_assistant_message":"Done."}),
            &mut env
        ),
        Decision::Block(_)
    ));
    let state::Stored::Valid(gate) = state::load_gate(&active.dir, &digest).unwrap() else {
        panic!("invalid gate")
    };
    assert_ne!(
        gate.checkpoint.state,
        coding_supervisor::TaskState::Completed
    );
    assert!(gate.receipts.iter().any(|receipt| receipt == &old_receipt));
    assert!(
        fs::read_to_string(active.dir.join("runs.jsonl"))
            .unwrap()
            .starts_with(&old_rows)
    );
    assert!(
        f.apply(
            &f.prompt("successor", "Implement next task"),
            "start",
            None,
            Some(approved)
        )
        .is_err()
    );
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        first_id
    );
    assert!(!f.root.join("archives").exists());
}

#[test]
fn monotonic_revision_preserves_gate_repair_rounds_and_full_contract_history() {
    let f = Fixture::new();
    let id = f.start();
    lifecycle::with_task(&f.root, Some(&id), None, None, |dir| {
        let (_, digest) = state::read_contract(dir)?;
        let gate = state::Gate::new(
            "ordinary-task".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Verifying,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 1,
                next_action: "verify".into(),
            },
        );
        state::save_gate(dir, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let original: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("contract.json")).unwrap())
            .unwrap();
    for field in ["criteria", "forbidden_effects", "max_repair_rounds", "goal"] {
        let mut changed = original.clone();
        match field {
            "criteria" => changed[field][0]["command"] = "true".into(),
            "forbidden_effects" => changed[field] = json!([]),
            "max_repair_rounds" => changed[field] = 99.into(),
            _ => changed[field] = "easier goal".into(),
        }
        assert!(
            f.apply(
                &f.prompt(field, "Continue implementing"),
                "revise",
                Some(&id),
                Some(changed)
            )
            .is_err()
        );
    }
    let mut expanded = original.clone();
    expanded["criteria"].as_array_mut().unwrap().push(json!({"id":"ADDED","check":"regression_suite_passes","mandatory":true,"command":"cargo test -p fixture --test regression"}));
    expanded["scope"] = json!(["src/", "tests/"]);
    f.apply(
        &f.prompt("scope", "Also add regression tests"),
        "revise",
        Some(&id),
        Some(expanded.clone()),
    )
    .unwrap();
    let (_, digest) = state::read_contract(&active.dir).unwrap();
    let state::Stored::Valid(gate) = state::load_gate(&active.dir, &digest).unwrap() else {
        panic!("revision invalidated authentic gate");
    };
    assert_eq!(gate.checkpoint.consumed_repair_rounds, 1);
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("autonomy.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["revisions"][1]["previous_contract"], original);
    assert_eq!(metadata["revisions"][1]["contract"], expanded);
    assert!(
        f.apply(
            &f.prompt("remove-added", "Continue"),
            "revise",
            Some(&id),
            Some(original)
        )
        .is_err()
    );
}

#[test]
fn concurrent_duplicate_delivery_creates_one_task_and_restart_replays_it() {
    let f = Fixture::new();
    let event = f.prompt("once", "Implement feature");
    let responses: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| f.apply(&event, "start", None, Some(contract()))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect()
    });
    assert!(responses.iter().all(|r| r == &responses[0]));
    assert_eq!(fs::read_dir(f.root.join("tasks")).unwrap().count(), 1);
    assert_eq!(
        f.apply(&event, "start", None, Some(contract())).unwrap(),
        responses[0]
    );
    assert!(f.apply(&event, "discussion", None, None).is_err());
}

#[test]
fn transcript_identity_distinguishes_identical_turns_and_fallback_refuses_ambiguity() {
    let f = Fixture::new();
    let transcript = f.project.join("transcript.jsonl");
    let payload =
        json!({"session_id":"session","prompt":"Explain ownership","transcript_path":transcript});
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({"type":"user","uuid":"turn-one","message":{"content":"Explain ownership"}})
        ),
    )
    .unwrap();
    let first = autonomy::record_prompt(&f.root, &f.project, &payload).unwrap();
    f.apply(&first, "discussion", None, None).unwrap();
    assert_eq!(
        autonomy::record_prompt(&f.root, &f.project, &payload).unwrap(),
        first
    );
    fs::write(&transcript, format!("{}\n{}\n", json!({"type":"user","uuid":"turn-one","message":{"content":"Explain ownership"}}), json!({"type":"user","uuid":"turn-two","message":{"content":[{"type":"text","text":"Explain ownership"}]}}))).unwrap();
    let second = autonomy::record_prompt(&f.root, &f.project, &payload).unwrap();
    assert_ne!(first["event_id"], second["event_id"]);
    f.apply(&second, "discussion", None, None).unwrap();
    let fallback = json!({"session_id":"fallback","prompt":"Explain ownership"});
    let anonymous = autonomy::record_prompt(&f.root, &f.project, &fallback).unwrap();
    assert_eq!(
        autonomy::record_prompt(&f.root, &f.project, &fallback).unwrap(),
        anonymous
    );
    f.apply(&anonymous, "discussion", None, None).unwrap();
    assert!(
        autonomy::record_prompt(&f.root, &f.project, &fallback)
            .unwrap_err()
            .contains("ambiguous")
    );
}

#[test]
fn git_diff_only_floor_cannot_certify_an_unsupported_functional_project() {
    let f = Fixture::new();
    fs::remove_file(f.project.join("Cargo.toml")).unwrap();
    fs::create_dir(f.project.join(".git")).unwrap();
    let event = f.prompt("unsupported", "Implement a service");
    assert!(f.apply(&event, "start", None, Some(contract())).is_err());
    assert!(lifecycle::active_task(&f.root).unwrap().is_none());
}

#[test]
fn revision_invalidates_receipts_and_dispatches_without_erasing_pending_linkage() {
    let f = Fixture::new();
    let id = f.start();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let pending = format!(
        "{}\n",
        json!({"kind":"dispatch","dispatch_id":"pending:job","task_id":id,"job_id":"job-preserved"})
    );
    let original: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("contract.json")).unwrap())
            .unwrap();
    lifecycle::with_task(&f.root, Some(&id), None, None, |dir| {
        let (_, digest) = state::read_contract(dir)?;
        let mut gate = state::Gate::new(
            "ordinary-task".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Verifying,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 1,
                next_action: "verify".into(),
            },
        );
        gate.receipts.push(coding_supervisor::VerificationReceipt {
            criterion: coding_supervisor::CriterionId("C2".into()),
            candidate: coding_supervisor::CandidateId {
                base_commit: "base".into(),
                dirty_digest: "dirty".into(),
            },
            command: "cargo test -p fixture".into(),
            cwd: f.project.to_string_lossy().into_owned(),
            exit_code: Some(0),
            output_ref: format!("observed:{id}:pending:job"),
            claim_supported: true,
            started_at: 1,
            finished_at: 2,
        });
        fs::write(dir.join("runs.jsonl"), &pending).unwrap();
        state::save_gate(dir, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
    let mut revised = original;
    revised["scope"] = json!(["src/", "tests/"]);
    f.apply(
        &f.prompt("revise", "Add regression tests too"),
        "revise",
        Some(&id),
        Some(revised),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(active.dir.join("runs.jsonl")).unwrap(),
        pending
    );
    assert!(!autonomy::dispatch_is_current(&active.dir, "pending:job").unwrap());
    assert!(autonomy::dispatch_is_current(&active.dir, "new:job").unwrap());
    let (_, digest) = state::read_contract(&active.dir).unwrap();
    let state::Stored::Valid(gate) = state::load_gate(&active.dir, &digest).unwrap() else {
        panic!("invalid gate");
    };
    assert!(gate.receipts.is_empty());
    assert_eq!(gate.checkpoint.consumed_repair_rounds, 1);
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("autonomy.json")).unwrap())
            .unwrap();
    assert_eq!(
        metadata["revisions"][1]["previous_gate"]["receipts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn asserted_completed_state_cannot_authorize_hook_task_rollover() {
    let f = Fixture::new();
    let id = f.start();
    lifecycle::with_task(&f.root, Some(&id), None, None, |dir| {
        let (_, digest) = state::read_contract(dir)?;
        let gate = state::Gate::new(
            "ordinary-task".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Completed,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 0,
                next_action: "MCP asserted completion".into(),
            },
        );
        state::save_gate(dir, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
    let next = f.prompt("next", "Implement another request");
    assert!(f.apply(&next, "start", None, Some(contract())).is_err());
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        id
    );
    assert!(!f.root.join("archives").join(format!("{id}.json")).exists());
}

#[test]
fn registration_error_is_durable_and_cannot_fall_back_to_inactive_or_asserted_intent() {
    let f = Fixture::new();
    fs::write(f.project.join("package.json"), "malformed manifest").unwrap();
    assert!(
        autonomy::record_prompt(
            &f.root,
            &f.project,
            &json!({"session_id":"session","prompt_id":"bad-manifest","prompt":"Implement feature"})
        )
        .is_err()
    );
    assert!(f.root.join("intents.json").exists());
    assert!(autonomy::pending(&f.root, "session").is_err());
    assert!(
        autonomy::record_intent(
            &f.root,
            &f.project,
            &json!({"session_id":"forged","prompt_id":"bypass","prompt":"Implement feature"})
        )
        .is_err()
    );
    fs::remove_file(f.project.join("package.json")).unwrap();
    f.prompt("valid-manifest", "Implement feature");
    assert!(autonomy::pending(&f.root, "session").unwrap().is_some());
}

#[test]
fn interrupted_activation_recovers_prompt_consumption_and_does_not_start_twice() {
    let f = Fixture::new();
    let event = f.prompt("once", "Implement feature");
    let before = fs::read_to_string(f.root.join("intents.json")).unwrap();
    let response = f.apply(&event, "start", None, Some(contract())).unwrap();
    let active = fs::read_to_string(f.root.join("active.json")).unwrap();
    let after = fs::read_to_string(f.root.join("intents.json")).unwrap();
    fs::remove_file(f.root.join("active.json")).unwrap();
    fs::write(f.root.join("intents.json"), before).unwrap();
    let journal = json!({"schema":1,"before_active":supervisor_claude_code::sha256::hex(b""),"after_active":supervisor_claude_code::sha256::hex(active.as_bytes()),"writes":[{"path":"active.json","text":active},{"path":"intents.json","text":after}]});
    fs::write(f.root.join("transaction.json"), journal.to_string()).unwrap();
    assert!(autonomy::pending(&f.root, "session").unwrap().is_none());
    assert_eq!(
        f.apply(&event, "start", None, Some(contract())).unwrap(),
        response
    );
    assert_eq!(fs::read_dir(f.root.join("tasks")).unwrap().count(), 1);
    assert!(!f.root.join("transaction.json").exists());
}

#[test]
fn explicit_resume_in_new_session_preserves_task_and_revokes_incidental_sibling_control() {
    let f = Fixture::new();
    let id = f.start();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let pending = "{\"kind\":\"job\",\"job_id\":\"preserve-me\"}\n";
    lifecycle::with_task(&f.root, Some(&id), None, None, |dir| {
        let (_, digest) = state::read_contract(dir)?;
        let gate = state::Gate::new(
            "ordinary-task".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Verifying,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 1,
                next_action: "await job".into(),
            },
        );
        state::save_gate(dir, &gate, &digest)?;
        fs::write(dir.join("runs.jsonl"), pending).unwrap();
        Ok(Value::Null)
    })
    .unwrap();
    let original_contract = fs::read(active.dir.join("contract.json")).unwrap();
    let original_gate = fs::read(active.dir.join("gate.json")).unwrap();
    for (event_id, prompt, action) in [
        ("incidental", "Continue working", "continue"),
        ("cancel", "cancel task", "cancel"),
        ("new", "Start another task", "start"),
    ] {
        let event = autonomy::record_prompt(
            &f.root,
            &f.project,
            &json!({"session_id":"replacement","prompt_id":event_id,"prompt":prompt}),
        )
        .unwrap();
        assert!(
            f.apply(
                &event,
                action,
                Some(&id),
                if action == "start" {
                    Some(contract())
                } else {
                    None
                }
            )
            .is_err()
        );
    }
    let forged = autonomy::record_intent(
        &f.root,
        &f.project,
        &json!({"session_id":"asserted","prompt_id":"resume","prompt":"resume task"}),
    )
    .unwrap();
    assert!(f.apply(&forged, "resume", Some(&id), None).is_err());
    let resume = autonomy::record_prompt(
        &f.root,
        &f.project,
        &json!({"session_id":"replacement","prompt_id":"resume","prompt":"resume task"}),
    )
    .unwrap();
    f.apply(&resume, "resume", Some(&id), None).unwrap();
    assert_eq!(
        lifecycle::active_task(&f.root).unwrap().unwrap().task_id,
        id
    );
    assert_eq!(
        fs::read(active.dir.join("contract.json")).unwrap(),
        original_contract
    );
    assert_eq!(
        fs::read(active.dir.join("gate.json")).unwrap(),
        original_gate
    );
    assert_eq!(
        fs::read_to_string(active.dir.join("runs.jsonl")).unwrap(),
        pending
    );
    assert!(!f.root.join("archives").exists());
    let metadata: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("autonomy.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["session_id"], "replacement");
    assert_eq!(metadata["revisions"][1]["from_session"], "session");
    lifecycle::with_task(&f.root, Some(&id), Some("replacement"), None, |_| {
        Ok(Value::Null)
    })
    .unwrap();
    let old = f.prompt("old-control", "cancel task");
    assert!(f.apply(&old, "cancel", Some(&id), None).is_err());
}

#[test]
fn resumed_owner_can_edit_while_original_owner_retains_only_read_access() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Fixed;
    impl CandidateProvider for Fixed {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "one".into(),
                },
                provenance: "fixture".into(),
                changed_paths: vec!["src/lib.rs".into()],
            })
        }
    }
    let f = Fixture::new();
    let id = f.start();
    let event = autonomy::record_prompt(
        &f.root,
        &f.project,
        &json!({"session_id":"replacement","prompt_id":"resume","prompt":"resume task"}),
    )
    .unwrap();
    f.apply(&event, "resume", Some(&id), None).unwrap();
    let mut env = Env {
        state_dir: f.root.clone(),
        state_location: "test",
        candidate: Box::new(Fixed),
        clock: || 100,
    };
    let old_write = json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","tool_use_id":"old-write","tool_name":"Write","tool_input":{"file_path":f.project.join("src/lib.rs"),"content":"changed"}});
    assert!(matches!(handle(&old_write, &mut env), Decision::Deny(_)));
    let mut read = old_write.clone();
    read["tool_use_id"] = "old-read".into();
    read["tool_name"] = "Read".into();
    assert_eq!(handle(&read, &mut env), Decision::None);
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    assert!(
        !active.dir.join("gate.json").exists(),
        "read-only old-owner hooks must not initialize gate state"
    );
    let mut current = old_write;
    current["session_id"] = "replacement".into();
    current["tool_use_id"] = "current-write".into();
    assert_eq!(handle(&current, &mut env), Decision::None);
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let before = fs::read(active.dir.join("gate.json")).unwrap();
    for event in ["Stop", "SubagentStop", "PostToolUse", "PostToolUseFailure"] {
        assert_eq!(
            handle(
                &json!({"hook_event_name":event,"cwd":f.project,"session_id":"session","last_assistant_message":"Done"}),
                &mut env
            ),
            Decision::None
        );
        assert_eq!(fs::read(active.dir.join("gate.json")).unwrap(), before);
    }
}

#[test]
fn npm_entrypoints_are_frozen_while_unrelated_manifest_fields_remain_editable() {
    let f = Fixture::new();
    fs::write(
        f.project.join("package.json"),
        json!({"name":"fixture","scripts":{"test":"node tests.js","other":"node other.js"}})
            .to_string(),
    )
    .unwrap();
    let id = f.start();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_ok());
    fs::write(f.project.join("package.json"), json!({"name":"renamed","description":"ordinary edit","scripts":{"test":"node tests.js","other":"node changed.js"}}).to_string()).unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_ok());
    fs::write(
        f.project.join("package.json"),
        json!({"name":"renamed","scripts":{"test":"node -e process.exit(0)"}}).to_string(),
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_err());
    struct Current;
    impl supervisor_claude_code::candidate::CandidateProvider for Current {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "current".into(),
                },
                provenance: "fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    let mut env = supervisor_claude_code::adapter::Env {
        state_dir: f.root.clone(),
        state_location: "test",
        candidate: Box::new(Current),
        clock: || 100,
    };
    let stopped = supervisor_claude_code::adapter::handle(
        &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","last_assistant_message":"Done"}),
        &mut env,
    );
    assert!(
        matches!(stopped, supervisor_claude_code::adapter::Decision::Block(ref message) if message.contains("entrypoint"))
    );
    let (_, digest) = state::read_contract(&active.dir).unwrap();
    let state::Stored::Valid(gate) = state::load_gate(&active.dir, &digest).unwrap() else {
        panic!("invalid gate");
    };
    assert_eq!(gate.checkpoint.consumed_repair_rounds, 1);
    let old: Value =
        serde_json::from_str(&fs::read_to_string(active.dir.join("contract.json")).unwrap())
            .unwrap();
    f.apply(
        &f.prompt("revise-runner", "Continue working"),
        "revise",
        Some(&id),
        Some(old),
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_err());
    fs::write(
        f.project.join("package.json"),
        json!({"scripts":{"test":"node tests.js"}}).to_string(),
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_ok());
    fs::write(
        f.project.join("package.json"),
        json!({"scripts":{}}).to_string(),
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_err());
    fs::write(
        f.project.join("package.json"),
        json!({"scripts":{"test":"node tests.js","pretest":"node prepare.js"}}).to_string(),
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_err());
    assert!(
        !fs::read_to_string(active.dir.join("autonomy.json"))
            .unwrap()
            .contains("node tests.js")
    );
    let resume = autonomy::record_prompt(
        &f.root,
        &f.project,
        &json!({"session_id":"replacement","prompt_id":"resume","prompt":"resume task"}),
    )
    .unwrap();
    f.apply(&resume, "resume", Some(&id), None).unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &f.project).is_err());
}

#[test]
fn npm_entrypoint_changes_between_prompt_and_start_cannot_redefine_the_floor() {
    let f = Fixture::new();
    fs::write(
        f.project.join("package.json"),
        json!({"scripts":{"test":"node tests.js"}}).to_string(),
    )
    .unwrap();
    let event = f.prompt("initial", "Implement feature");
    fs::write(
        f.project.join("package.json"),
        json!({"scripts":{"test":"node -e process.exit(0)"}}).to_string(),
    )
    .unwrap();
    assert!(f.apply(&event, "start", None, Some(contract())).is_err());
    assert!(lifecycle::active_task(&f.root).unwrap().is_none());
}

#[test]
fn ownership_transfer_between_fast_check_and_locked_hook_cannot_mutate_task() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Transfer {
        root: PathBuf,
        project: PathBuf,
        proposal: Value,
        once: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl CandidateProvider for Transfer {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            if !self.once.swap(true, Ordering::SeqCst) {
                autonomy::propose(&self.root, &self.project, &self.proposal)?;
            }
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "one".into(),
                },
                provenance: "fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    for event in ["PreToolUse", "Stop", "SubagentStop"] {
        let f = Fixture::new();
        let id = f.start();
        let resume = autonomy::record_prompt(
            &f.root,
            &f.project,
            &json!({"session_id":"replacement","prompt_id":"resume","prompt":"resume task"}),
        )
        .unwrap();
        let proposal = json!({"event_id":resume["event_id"],"nonce":resume["nonce"],"prompt_hash":resume["prompt_hash"],"action":"resume","task_id":id});
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut env = Env {
            state_dir: f.root.clone(),
            state_location: "test",
            clock: || 100,
            candidate: Box::new(Transfer {
                root: f.root.clone(),
                project: f.project.clone(),
                proposal,
                once: once.clone(),
            }),
        };
        let payload = json!({"hook_event_name":event,"cwd":f.project,"session_id":"session","tool_use_id":"racing-hook","tool_name":"Write","tool_input":{"file_path":f.project.join("src/lib.rs"),"content":"late"},"last_assistant_message":"Done"});
        if event == "PreToolUse" {
            assert_eq!(handle(&payload, &mut env), Decision::None);
            let active = lifecycle::active_task(&f.root).unwrap().unwrap();
            assert!(active.dir.join("applied-events.json").exists());
        }
        let active = lifecycle::active_task(&f.root).unwrap().unwrap();
        let before_gate = fs::read(active.dir.join("gate.json")).ok();
        once.store(false, Ordering::SeqCst);
        let decision = handle(&payload, &mut env);
        if event == "PreToolUse" {
            assert!(matches!(decision, Decision::Deny(_)));
        } else {
            assert_eq!(decision, Decision::None);
        }
        let active = lifecycle::active_task(&f.root).unwrap().unwrap();
        assert_eq!(
            fs::read(active.dir.join("gate.json")).ok(),
            before_gate,
            "obsolete hook changed the gate after ownership transfer"
        );
        assert!(autonomy::owns_session(&active.dir, "replacement").unwrap());
    }
}

#[test]
fn newer_prompt_race_blocks_old_mutations_and_stop_without_advancing_gate() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Transfer {
        root: PathBuf,
        project: PathBuf,
        mode: &'static str,
        expected_gate: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
        once: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl CandidateProvider for Transfer {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            if !self.once.swap(true, Ordering::SeqCst) {
                let event = autonomy::record_prompt(
                    &self.root,
                    &self.project,
                    &json!({"session_id":"session","prompt_id":"newer","prompt":"Explain the current design"}),
                )?;
                if self.mode == "discussion" {
                    autonomy::propose(
                        &self.root,
                        &self.project,
                        &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":"discussion"}),
                    )?;
                } else if self.mode == "revise" {
                    let active = lifecycle::active_task(&self.root)?.ok_or("no active task")?;
                    let mut contract: Value = serde_json::from_slice(
                        &fs::read(active.dir.join("contract.json")).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    contract["scope"] = json!(["revised/"]);
                    autonomy::propose(
                        &self.root,
                        &self.project,
                        &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":"revise","task_id":active.task_id,"contract":contract}),
                    )?;
                }
                let active = lifecycle::active_task(&self.root)?.ok_or("no active task")?;
                *self.expected_gate.lock().unwrap() = fs::read(active.dir.join("gate.json")).ok();
            }
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "one".into(),
                },
                provenance: "fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    let mut failures = Vec::new();
    for event in ["PreToolUse", "Stop", "SubagentStop"] {
        for mode in ["pending", "discussion", "revise"] {
            for cached in [false, true] {
                let f = Fixture::new();
                f.start();
                let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                let expected_gate = std::sync::Arc::new(std::sync::Mutex::new(None));
                let mut env = Env {
                    state_dir: f.root.clone(),
                    state_location: "test",
                    clock: || 100,
                    candidate: Box::new(Transfer {
                        root: f.root.clone(),
                        project: f.project.clone(),
                        mode,
                        expected_gate: expected_gate.clone(),
                        once: once.clone(),
                    }),
                };
                let payload = json!({"hook_event_name":event,"cwd":f.project,"session_id":"session","tool_use_id":"racing-prompt-hook","tool_name":"Write","tool_input":{"file_path":f.project.join("src/lib.rs"),"content":"late"},"last_assistant_message":"Done"});
                if cached {
                    let seeded = handle(&payload, &mut env);
                    if event == "PreToolUse" {
                        assert_eq!(seeded, Decision::None);
                    }
                    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
                    assert!(active.dir.join("applied-events.json").exists());
                }
                let active = lifecycle::active_task(&f.root).unwrap().unwrap();
                *expected_gate.lock().unwrap() = fs::read(active.dir.join("gate.json")).ok();
                once.store(false, Ordering::SeqCst);
                let decision = handle(&payload, &mut env);
                let correct = match (&decision, event, mode == "discussion") {
                    (Decision::Deny(reason), "PreToolUse", _) => {
                        reason.contains("prompt") || reason.contains("cached response refused")
                    }
                    (Decision::Block(reason), _, false) => {
                        reason.contains("prompt") || reason.contains("cached response refused")
                    }
                    (Decision::None, _, true) if event != "PreToolUse" => true,
                    (Decision::Block(reason), _, true) if cached => {
                        reason.contains("cached response refused")
                    }
                    _ => false,
                };
                if !correct
                    || fs::read(active.dir.join("gate.json")).ok() != *expected_gate.lock().unwrap()
                {
                    failures.push(format!(
                        "{event}, mode={mode}, cached={cached}: {decision:?}"
                    ));
                }
                assert!(autonomy::owns_session(&active.dir, "session").unwrap());
                let before_read = fs::read(active.dir.join("gate.json")).ok();
                assert_eq!(
                    handle(
                        &json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","tool_use_id":"read-current","tool_name":"Read","tool_input":{"file_path":f.project.join("src/lib.rs")}}),
                        &mut env,
                    ),
                    Decision::None
                );
                assert_eq!(fs::read(active.dir.join("gate.json")).ok(), before_read);
                let fresh = f.prompt("fresh-discussion", "Explain the current design");
                f.apply(&fresh, "discussion", None, None).unwrap();
                assert_eq!(
                    handle(
                        &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","tool_use_id":"fresh-discussion-stop","last_assistant_message":"Explanation"}),
                        &mut env,
                    ),
                    Decision::None,
                    "a fresh discussion Stop remains usable"
                );
            }
        }
    }
    assert!(
        failures.is_empty(),
        "obsolete hooks retained authority: {failures:#?}"
    );
}

#[test]
fn pending_prompt_keeps_inflight_tc_job_and_accepts_its_later_observation() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    use supervisor_claude_code::candidate::CandidateProvider;
    struct Fixed;
    impl CandidateProvider for Fixed {
        fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
            Ok(coding_supervisor::Candidate {
                id: coding_supervisor::CandidateId {
                    base_commit: "base".into(),
                    dirty_digest: "one".into(),
                },
                provenance: "fixture".into(),
                changed_paths: vec![],
            })
        }
    }
    let f = Fixture::new();
    let id = f.start();
    let active = lifecycle::active_task(&f.root).unwrap().unwrap();
    let mut env = Env {
        state_dir: f.root.clone(),
        state_location: "test",
        clock: || 100,
        candidate: Box::new(Fixed),
    };
    let mut dispatch = json!({"hook_event_name":"PreToolUse","cwd":f.project,"session_id":"session","agent_id":"main","tool_use_id":"inflight","tool_name":"mcp__terminal_commander__run_and_watch","tool_input":{"argv":["cargo","test","-p","fixture"],"cwd":f.project}});
    assert_eq!(handle(&dispatch, &mut env), Decision::None);
    f.prompt("pending-during-job", "Explain the current implementation");
    dispatch["hook_event_name"] = "PostToolUse".into();
    dispatch["tool_response"] = json!({"job_id":"job-inflight","bucket_id":"bucket-inflight","state":"running","complete":false});
    assert_eq!(handle(&dispatch, &mut env), Decision::None);
    let jobs = autonomy::pending_jobs(&active.dir, &id).unwrap();
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    assert_eq!(jobs[0]["job_id"], "job-inflight");
    let before_gate = fs::read(active.dir.join("gate.json")).unwrap();
    assert!(matches!(
        handle(
            &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session","last_assistant_message":"Done"}),
            &mut env
        ),
        Decision::Block(_)
    ));
    assert_eq!(fs::read(active.dir.join("gate.json")).unwrap(), before_gate);
    assert_eq!(autonomy::pending_jobs(&active.dir, &id).unwrap(), jobs);
    let result = json!({"hook_event_name":"PostToolUse","cwd":f.project,"session_id":"session","agent_id":"main","tool_use_id":"inflight-status","tool_name":"mcp__terminal_commander__command_status","tool_input":{"job_id":"job-inflight"},"tool_response":{"job_id":"job-inflight","bucket_id":"bucket-inflight","state":"exited","complete":true,"exit_code":0,"outcome_trust":"observed","degraded":false,"pipeline_exit_masked":false}});
    assert_eq!(handle(&result, &mut env), Decision::None);
    assert!(
        autonomy::pending_jobs(&active.dir, &id)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let runs = fs::read_to_string(active.dir.join("runs.jsonl")).unwrap();
    let completed: Vec<Value> = runs
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["kind"] == "completed")
        .collect();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0]["command"], "cargo test -p fixture");
    assert_eq!(completed[0]["exit_code"], 0);
    assert_eq!(completed[0]["job_id"], "job-inflight");
    assert_eq!(completed[0]["completion_trust"], "observed");
    assert!(autonomy::pending(&f.root, "session").unwrap().is_some());
}
