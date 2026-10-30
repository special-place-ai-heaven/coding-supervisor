use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use supervisor_claude_code::{autonomy, lifecycle, location};

static ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "supervisor-registration-discovery-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("project/src")).unwrap();
        fs::write(
            dir.join("project/Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            dir.join("project/package.json"),
            r#"{"scripts":{"test":"node check.js"}}"#,
        )
        .unwrap();
        git(&dir.join("project"), &["init", "--quiet"]);
        git(&dir.join("project"), &["add", "."]);
        git(
            &dir.join("project"),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "git failed: {:?}", output.status);
}

fn prompt(root: &Path, project: &Path, id: &str) -> Value {
    autonomy::record_prompt(
        root,
        project,
        &json!({"session_id":"session","prompt_id":id,"prompt":"Implement feature"}),
    )
    .unwrap()
}

fn proposal(event: &Value, action: &str) -> Value {
    json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,
        "contract":{"id":"fixture","goal":"Implement feature","criteria":event["required_criteria"],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}})
}

fn assert_context_pair(f: &Fixture, project: &Path, hook_nested: bool) {
    let package = fs::read(project.join("package.json")).unwrap();
    let nested = project.join("src");
    let root = f.0.join(if hook_nested {
        "state-nested"
    } else {
        "state-root"
    });
    let (hook, mcp) = if hook_nested {
        (nested.as_path(), project)
    } else {
        (project, nested.as_path())
    };
    let event = prompt(&root, hook, "request");
    let checks = event["required_criteria"].as_array().unwrap();
    for id in [
        "AUTO_FMT",
        "AUTO_LINT",
        "AUTO_TEST",
        "AUTO_DIFF",
        "AUTO_NPM_TEST",
    ] {
        assert!(
            checks.iter().any(|check| check["id"] == id),
            "missing {id} from {hook:?}"
        );
    }
    let response = autonomy::propose(&root, mcp, &proposal(&event, "start")).unwrap();
    let active = lifecycle::active_task(&root).unwrap().unwrap();
    assert_eq!(response["task_id"], active.task_id);
    autonomy::validate_entrypoints(&active.dir, hook).unwrap();
    autonomy::validate_entrypoints(&active.dir, mcp).unwrap();
    let replay = prompt(&root, mcp, "request");
    assert_eq!(replay["event_id"], event["event_id"]);
    assert_eq!(replay["nonce"], event["nonce"]);
    fs::write(
        project.join("package.json"),
        r#"{"scripts":{"test":"node changed.js"}}"#,
    )
    .unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, hook).is_err());
    assert!(autonomy::validate_entrypoints(&active.dir, mcp).is_err());
    fs::write(project.join("package.json"), package).unwrap();
}

#[test]
fn root_hook_and_nested_mcp_share_project_context() {
    let f = Fixture::new();
    assert_context_pair(&f, &f.0.join("project"), false);
}

#[test]
fn nested_hook_and_root_mcp_discover_the_root_check_floor() {
    let f = Fixture::new();
    assert_context_pair(&f, &f.0.join("project"), true);
}

#[test]
fn nested_hook_and_nested_mcp_discover_root_checks_and_entrypoints() {
    let f = Fixture::new();
    let project = f.0.join("project");
    let nested = project.join("src");
    let root = f.0.join("state");
    let event = prompt(&root, &nested, "nested");
    assert_eq!(event["required_criteria"].as_array().unwrap().len(), 5);
    autonomy::propose(&root, &nested, &proposal(&event, "start")).unwrap();
    let active = lifecycle::active_task(&root).unwrap().unwrap();
    autonomy::validate_entrypoints(&active.dir, &project).unwrap();
    autonomy::validate_entrypoints(&active.dir, &nested).unwrap();
}

#[test]
fn linked_worktree_context_is_shared_only_with_its_own_subdirectory() {
    let f = Fixture::new();
    let project = f.0.join("project");
    let linked = f.0.join("linked");
    git(
        &project,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    fs::create_dir_all(linked.join("src")).unwrap();
    assert_ne!(
        location::project_key(&project),
        location::project_key(&linked)
    );
    assert_context_pair(&f, &linked, false);
    assert_context_pair(&f, &linked, true);
    let root = f.0.join("isolated-state");
    let event = prompt(&root, &project, "foreign");
    assert!(autonomy::propose(&root, &linked, &proposal(&event, "discussion")).is_err());
    let external = f.0.join("external");
    fs::create_dir_all(&external).unwrap();
    fs::copy(project.join("Cargo.toml"), external.join("Cargo.toml")).unwrap();
    assert_ne!(
        location::project_key(&project),
        location::project_key(&external)
    );
    assert!(autonomy::propose(&root, &external, &proposal(&event, "discussion")).is_err());
}

#[test]
fn concurrent_duplicate_registrations_keep_one_event_and_nonce() {
    let f = Fixture::new();
    let root = f.0.join("state");
    let project = f.0.join("project");
    let nested = project.join("src");
    let events = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let root = &root;
                let project = if index % 2 == 0 { &project } else { &nested };
                scope.spawn(move || autonomy::record_prompt(root, project, &json!({"session_id":"session","prompt_id":"same","prompt":"Implement feature"})))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let first = events.iter().find_map(|event| event.as_ref().ok()).unwrap();
    for event in events.iter().filter_map(|event| event.as_ref().ok()) {
        assert_eq!(event["event_id"], first["event_id"]);
        assert_eq!(event["nonce"], first["nonce"]);
    }
    let newer = prompt(&root, &project, "newer");
    prompt(&root, &project, "same");
    assert_eq!(
        autonomy::pending(&root, "session").unwrap().unwrap()["event_id"],
        newer["event_id"]
    );
    autonomy::propose(&root, &project, &proposal(&newer, "discussion")).unwrap();
}

#[test]
fn historical_raw_project_metadata_normalizes_without_accepting_foreign_worktrees() {
    let f = Fixture::new();
    let root = f.0.join("state");
    let project = f.0.join("project");
    let nested = project.join("src");
    let event = prompt(&root, &project, "historical");
    let raw = fs::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let raw = if cfg!(windows) {
        raw.to_uppercase()
    } else {
        raw
    };
    let path = root.join("intents.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["events"][event["event_id"].as_str().unwrap()]["project"] = raw.clone().into();
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert_eq!(
        prompt(&root, &nested, "historical")["nonce"],
        event["nonce"]
    );
    autonomy::propose(&root, &nested, &proposal(&event, "start")).unwrap();
    let active = lifecycle::active_task(&root).unwrap().unwrap();
    let path = active.dir.join("autonomy.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    metadata["project"] = raw.into();
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    autonomy::validate_entrypoints(&active.dir, &nested).unwrap();
    let external = f.0.join("external");
    fs::create_dir_all(&external).unwrap();
    fs::copy(project.join("package.json"), external.join("package.json")).unwrap();
    assert!(autonomy::validate_entrypoints(&active.dir, &external).is_err());
}

#[test]
fn non_git_directory_does_not_inherit_parent_discovery_or_identity() {
    let f = Fixture::new();
    let external = f.0.join("external");
    let nested = external.join("src");
    fs::create_dir_all(&nested).unwrap();
    fs::copy(
        f.0.join("project/package.json"),
        external.join("package.json"),
    )
    .unwrap();
    let root = f.0.join("external-state");
    let event = prompt(&root, &external, "external");
    assert!(
        event["required_criteria"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "AUTO_NPM_TEST")
    );
    assert_ne!(
        location::project_key(&external),
        location::project_key(&nested)
    );
    assert!(autonomy::propose(&root, &nested, &proposal(&event, "discussion")).is_err());
    let nested_event = prompt(
        &f.0.join("nested-external-state"),
        &nested,
        "nested-external",
    );
    assert!(
        nested_event["required_criteria"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn legacy_nested_event_cannot_certify_its_nested_only_floor_as_root_checks() {
    let f = Fixture::new();
    let root = f.0.join("state");
    let project = f.0.join("project");
    let nested = project.join("src");
    fs::copy(project.join("package.json"), nested.join("package.json")).unwrap();
    let event = prompt(&root, &project, "legacy");
    let path = root.join("intents.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let legacy = &mut registry["events"][event["event_id"].as_str().unwrap()];
    legacy["project"] = fs::canonicalize(&nested)
        .unwrap()
        .to_string_lossy()
        .into_owned()
        .into();
    legacy["required_criteria"] = json!([{"id":"AUTO_NPM_TEST","check":"repository_validation","mandatory":true,"command":"npm run test"}]);
    let legacy = legacy.clone();
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let replay = autonomy::record_prompt(
        &root,
        &project,
        &json!({"session_id":"session","prompt_id":"legacy","prompt":"Implement feature"}),
    );
    let start = autonomy::propose(&root, &project, &proposal(&legacy, "start"));
    assert!(
        replay.is_err() && start.is_err(),
        "legacy replay accepted: {}; legacy start accepted: {}",
        replay.is_ok(),
        start.is_ok()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(lifecycle::active_task(&root).unwrap().is_none());
}

#[test]
fn legacy_nested_task_refuses_continue_and_accept_but_explicit_cancel_recovers() {
    let f = Fixture::new();
    let root = f.0.join("state");
    let project = f.0.join("project");
    let nested = project.join("src");
    let initial = prompt(&root, &project, "initial");
    autonomy::propose(&root, &project, &proposal(&initial, "start")).unwrap();
    let active = lifecycle::active_task(&root).unwrap().unwrap();
    let path = active.dir.join("autonomy.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    metadata["project"] = fs::canonicalize(&nested)
        .unwrap()
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let validation = autonomy::validate_entrypoints(&active.dir, &project);
    let next = prompt(&root, &project, "continue");
    let mut continuation = proposal(&next, "continue");
    continuation["task_id"] = active.task_id.clone().into();
    let continued = autonomy::propose(&root, &project, &continuation);
    assert!(
        validation.is_err() && continued.is_err(),
        "legacy validation accepted: {}; legacy continue accepted: {}",
        validation.is_ok(),
        continued.is_ok()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let contract_before = fs::read(active.dir.join("contract.json")).unwrap();
    let cancel = autonomy::record_prompt(
        &root,
        &project,
        &json!({"session_id":"session","prompt_id":"cancel","prompt":"cancel task"}),
    )
    .unwrap();
    let mut cancellation = proposal(&cancel, "cancel");
    cancellation["task_id"] = active.task_id.clone().into();
    autonomy::propose(&root, &project, &cancellation).unwrap();
    let registry: Value =
        serde_json::from_slice(&fs::read(root.join("intents.json")).unwrap()).unwrap();
    assert_eq!(registry["project_floor"]["project"], metadata["project"]);
    assert!(lifecycle::active_task(&root).unwrap().is_none());
    assert!(
        root.join("archives")
            .join(format!("{}.json", active.task_id))
            .is_file()
    );
    assert_eq!(
        fs::read(active.dir.join("contract.json")).unwrap(),
        contract_before
    );
    let fresh = prompt(&root, &project, "fresh");
    let response = autonomy::propose(&root, &project, &proposal(&fresh, "start")).unwrap();
    assert_ne!(response["task_id"], active.task_id);
    assert_eq!(
        lifecycle::active_task(&root).unwrap().unwrap().task_id,
        response["task_id"].as_str().unwrap()
    );
    assert_eq!(fresh["required_criteria"].as_array().unwrap().len(), 5);
}

#[test]
fn legacy_approved_floor_requires_root_discovery_provenance() {
    for nested_capture in [false, true] {
        let f = Fixture::new();
        let root = f.0.join("state");
        let project = f.0.join("project");
        fs::remove_file(project.join("Cargo.toml")).unwrap();
        fs::remove_file(project.join("package.json")).unwrap();
        let captured = if nested_capture {
            project.join("src")
        } else {
            project.clone()
        };
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("intents.json"), json!({"schema":1,"events":{},"latest":{},"project_floor":{"project":captured.canonicalize().unwrap(),"required_criteria":[{"id":"ROOT","check":"root_validation","mandatory":true,"command":"python root_check.py"}]}}).to_string()).unwrap();
        let response = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":"floor","prompt":"Implement feature"}),
        );
        if nested_capture {
            assert!(
                response
                    .unwrap_err()
                    .contains("legacy subdirectory discovery")
            );
            assert!(lifecycle::active_task(&root).unwrap().is_none());
        } else {
            assert!(
                response.unwrap()["required_criteria"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|criterion| criterion["id"] == "ROOT")
            );
        }
    }
}

#[test]
fn completed_legacy_nested_task_can_be_explicitly_archived_without_rewriting_history() {
    fn snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        let mut result = std::collections::BTreeMap::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                result.extend(snapshot(&path));
            } else {
                result.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
        result
    }
    for legacy in [false, true] {
        let f = Fixture::new();
        let root = f.0.join("state");
        let project = f.0.join("project");
        let initial = prompt(&root, &project, "initial");
        autonomy::propose(&root, &project, &proposal(&initial, "start")).unwrap();
        let active = lifecycle::active_task(&root).unwrap().unwrap();
        let (_, digest) = supervisor_claude_code::state::read_contract(&active.dir).unwrap();
        let mut gate = supervisor_claude_code::state::Gate::new(
            "fixture".into(),
            coding_supervisor::Checkpoint {
                state: coding_supervisor::TaskState::Completed,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 0,
                next_action: "historical completed snapshot".into(),
            },
        );
        gate.receipts.push(coding_supervisor::VerificationReceipt {
            criterion: coding_supervisor::CriterionId("AUTO_NPM_TEST".into()),
            candidate: coding_supervisor::CandidateId {
                base_commit: "historical".into(),
                dirty_digest: "snapshot".into(),
            },
            command: "npm run test".into(),
            cwd: project.join("src").to_string_lossy().into_owned(),
            exit_code: Some(0),
            output_ref: "legacy-snapshot".into(),
            claim_supported: true,
            started_at: 1,
            finished_at: 2,
        });
        supervisor_claude_code::state::save_gate(&active.dir, &gate, &digest).unwrap();
        if legacy {
            let path = active.dir.join("autonomy.json");
            let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            metadata["project"] = project
                .join("src")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
                .into();
            fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
            let next = prompt(&root, &project, "premature");
            assert!(autonomy::propose(&root, &project, &proposal(&next, "start")).is_err());
        }
        let before = snapshot(&active.dir);
        let cancel = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":"cancel","prompt":"cancel task"}),
        )
        .unwrap();
        let mut cancellation = proposal(&cancel, "cancel");
        cancellation["task_id"] = active.task_id.clone().into();
        let result = autonomy::propose(&root, &project, &cancellation);
        if !legacy {
            assert!(result.unwrap_err().contains("completed task is immutable"));
            assert_eq!(
                lifecycle::active_task(&root).unwrap().unwrap().task_id,
                active.task_id
            );
        } else {
            assert!(result.unwrap()["inactive"].as_bool().unwrap());
            assert!(lifecycle::active_task(&root).unwrap().is_none());
            let archive: Value = serde_json::from_slice(
                &fs::read(
                    root.join("archives")
                        .join(format!("{}.json", active.task_id)),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(archive["disposition"], "cancelled");
            assert_eq!(archive["unverified_legacy_context"], true);
            let fresh = prompt(&root, &project, "fresh");
            let successor = autonomy::propose(&root, &project, &proposal(&fresh, "start")).unwrap();
            assert_ne!(successor["task_id"], active.task_id);
            assert_eq!(
                lifecycle::active_task(&root).unwrap().unwrap().task_id,
                successor["task_id"].as_str().unwrap()
            );
            assert_eq!(fresh["required_criteria"].as_array().unwrap().len(), 5);
        }
        assert_eq!(snapshot(&active.dir), before);
    }
}
