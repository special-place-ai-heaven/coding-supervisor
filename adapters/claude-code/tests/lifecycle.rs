use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use coding_supervisor::{
    Candidate, CandidateId, Checkpoint, CriterionId, TaskState, VerificationReceipt,
};
use serde_json::{Value, json};
use supervisor_claude_code::candidate::{CandidateProvider, GitCandidate};
use supervisor_claude_code::{lifecycle, location, sha256, state, transaction};

static ID: AtomicUsize = AtomicUsize::new(0);
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir() -> Dir {
    let root = std::env::temp_dir().join(format!(
        "sup-lifecycle-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    Dir(root)
}
fn contract() -> Value {
    json!({"id":"user-contract","goal":"lifecycle fixture","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"forbidden_effects":[],"scope":["src/"],"max_repair_rounds":2})
}
fn start(root: &Path) -> String {
    lifecycle::start(root, &contract(), false, None, None).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn set_state(root: &Path, id: &str, task_state: TaskState, rounds: u32) {
    lifecycle::with_task(root, Some(id), None, None, |stage| {
        let (_, digest) = state::read_contract(stage)?;
        let gate = state::Gate::new(
            "user-contract".to_owned(),
            Checkpoint {
                state: task_state,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: rounds,
                next_action: "fixture".to_owned(),
            },
        );
        state::save_gate(stage, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
}

#[test]
fn rollover_freezes_evidence_separates_ids_and_deactivates() {
    let d = dir();
    let one = start(&d.0);
    set_state(&d.0, &one, TaskState::Completed, 1);
    let old = lifecycle::active_task(&d.0).unwrap().unwrap().dir;
    let before = fs::read(old.join("gate.json")).unwrap();
    let two = lifecycle::start(&d.0, &contract(), true, None, Some(&one)).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(one, two);
    assert_eq!(fs::read(old.join("gate.json")).unwrap(), before);
    assert!(
        !lifecycle::active_task(&d.0)
            .unwrap()
            .unwrap()
            .dir
            .join("gate.json")
            .exists()
    );
    assert!(
        lifecycle::with_task(&d.0, Some(&one), None, None, |_| panic!(
            "late old task reached successor"
        ))
        .is_err()
    );
    assert!(lifecycle::deactivate(&d.0, &two, None).is_err());
    lifecycle::deactivate(&d.0, &two, Some("cancelled")).unwrap();
    assert!(lifecycle::active_task(&d.0).unwrap().is_none());
    assert_eq!(fs::read(old.join("gate.json")).unwrap(), before);
}

#[test]
fn replayed_task_start_after_deactivation_does_not_create_a_new_task() {
    let d = dir();
    let mut request = contract();
    request["event_id"] = "start-once".into();
    let first = lifecycle::start(&d.0, &request, false, None, None).unwrap();
    let first_id = first["task_id"].as_str().unwrap().to_owned();
    lifecycle::deactivate(&d.0, &first_id, Some("cancelled")).unwrap();

    let replay = lifecycle::start(&d.0, &request, false, None, None).unwrap();

    assert_eq!(replay["task_id"], first_id);
    assert_eq!(replay["replayed"], true);
    assert!(lifecycle::active_task(&d.0).unwrap().is_none());
    assert_eq!(fs::read_dir(d.0.join("tasks")).unwrap().count(), 1);
    assert!(lifecycle::start(&d.0, &json!({"id":"different","goal":"changed","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"forbidden_effects":[],"scope":["src/"],"max_repair_rounds":2,"event_id":"start-once"}), false, None, None).is_err());
}

#[test]
fn cwd_fallback_legacy_state_requires_explicit_migration_before_hook_use() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};

    let d = dir();
    let root = d.0.join(".supervisor");
    fs::create_dir_all(&root).unwrap();
    let original_contract = contract().to_string();
    fs::write(root.join("contract.json"), &original_contract).unwrap();
    let (resolved, location_name) = location::resolve_state_dir_checked(&d.0, &|_| None).unwrap();
    let root = resolved.clone();
    assert_eq!(location_name, "cwd-fallback");

    let result = handle(
        &json!({"cwd":d.0,"session_id":"legacy-session","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"src/lib.rs"}}),
        &mut Env {
            state_dir: resolved.clone(),
            state_location: location_name,
            candidate: Box::new(FixedCandidate),
            clock: || 2,
        },
    );

    assert!(
        matches!(result, Decision::Deny(ref reason) if reason.contains("migrate")),
        "{result:?}"
    );
    assert_eq!(
        fs::read(root.join("contract.json")).unwrap(),
        original_contract.as_bytes()
    );
    assert!(!root.join("gate.json").exists());
    assert!(!root.join("events.jsonl").exists());
    assert!(!root.join("active.json").exists());

    lifecycle::migrate(&root, &root).unwrap();
    assert_eq!(
        fs::read(root.join("contract.json")).unwrap(),
        original_contract.as_bytes()
    );
    assert!(lifecycle::active_task(&root).unwrap().is_some());
}

#[test]
fn resume_preserves_budget_and_recovery_requires_explicit_disposition() {
    for status in [TaskState::Blocked, TaskState::BudgetExhausted] {
        let d = dir();
        let id = start(&d.0);
        set_state(&d.0, &id, status, 2);
        let task = lifecycle::active_task(&d.0).unwrap().unwrap();
        let before = fs::read(task.dir.join("gate.json")).unwrap();
        lifecycle::resume(&d.0, &id).unwrap();
        assert_eq!(fs::read(task.dir.join("gate.json")).unwrap(), before);
        assert!(lifecycle::start(&d.0, &contract(), true, None, Some(&id)).is_err());
        let next = lifecycle::start(
            &d.0,
            &contract(),
            true,
            Some(state::state_name(status)),
            Some(&id),
        )
        .unwrap();
        assert_eq!(next["recovery_of"], id);
        assert_eq!(fs::read(task.dir.join("gate.json")).unwrap(), before);
    }
}

#[test]
fn duplicate_events_are_idempotent_and_stale_sessions_cannot_write() {
    let d = dir();
    let id = start(&d.0);
    let action = |stage: &Path| {
        state::append_line(&stage.join("runs.jsonl"), &json!({"one":true}))?;
        Ok(json!({"accepted":true}))
    };
    let first = lifecycle::with_task(
        &d.0,
        None,
        Some("old-session"),
        Some(("event-1", "input")),
        action,
    )
    .unwrap();
    let second = lifecycle::with_task(
        &d.0,
        None,
        Some("old-session"),
        Some(("event-1", "input")),
        |_| panic!("duplicate executed"),
    )
    .unwrap();
    assert_eq!(first, second);
    let old = lifecycle::active_task(&d.0).unwrap().unwrap();
    assert_eq!(
        fs::read_to_string(old.dir.join("runs.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    lifecycle::start(&d.0, &contract(), true, Some("cancelled"), Some(&id)).unwrap();
    assert!(
        lifecycle::with_task(&d.0, None, Some("old-session"), None, |_| panic!(
            "late session executed"
        ))
        .is_err()
    );
    let active = lifecycle::active_task(&d.0).unwrap().unwrap();
    lifecycle::rebind(&d.0, &active.task_id, "old-session").unwrap();
    lifecycle::with_task(&d.0, None, Some("old-session"), None, |_| Ok(Value::Null)).unwrap();
    assert!(
        lifecycle::with_task(&d.0, Some(&id), Some("old-session"), None, |_| panic!(
            "old explicit task executed"
        ))
        .is_err()
    );
}

#[test]
fn concurrent_updates_serialize_without_lost_journal_lines() {
    let d = dir();
    let id = start(&d.0);
    let mut workers = Vec::new();
    for n in 0..12 {
        let root = d.0.clone();
        let id = id.clone();
        workers.push(std::thread::spawn(move || {
            lifecycle::with_task(
                &root,
                Some(&id),
                None,
                Some((&format!("event-{n}"), "input")),
                |stage| {
                    state::append_line(&stage.join("runs.jsonl"), &json!({"n":n}))?;
                    Ok(Value::Null)
                },
            )
            .unwrap()
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let active = lifecycle::active_task(&d.0).unwrap().unwrap();
    assert_eq!(
        fs::read_to_string(active.dir.join("runs.jsonl"))
            .unwrap()
            .lines()
            .count(),
        12
    );
}

#[test]
fn redo_recovers_every_prefix_of_rollover_and_rejects_obsolete_journal() {
    for prefix in 0..=3 {
        let d = dir();
        let one = start(&d.0);
        set_state(&d.0, &one, TaskState::Completed, 0);
        let old = lifecycle::active_task(&d.0).unwrap().unwrap();
        let old_bytes = fs::read(old.dir.join("gate.json")).unwrap();
        let before = fs::read(d.0.join("active.json")).unwrap();
        let next = "task-recovered";
        let after = format!("{}\n", json!({"schema":2,"task_id":next}));
        let writes = vec![
            json!({"path":format!("archives/{one}.json"),"text":format!("{}\n",json!({"task_id":one,"disposition":"completed"}))}),
            json!({"path":format!("tasks/{next}/task.json"),"text":format!("{}\n",json!({"task_id":next,"contract_id":"user-contract"}))}),
            json!({"path":"active.json","text":after}),
        ];
        let journal = json!({"schema":1,"before_active":sha256::hex(&before),"after_active":sha256::hex(after.as_bytes()),"writes":writes});
        for write in writes.iter().take(prefix) {
            transaction::atomic_write(
                &d.0.join(write["path"].as_str().unwrap()),
                write["text"].as_str().unwrap().as_bytes(),
            )
            .unwrap();
        }
        fs::write(d.0.join("transaction.json"), journal.to_string()).unwrap();
        drop(transaction::Transaction::open(&d.0).unwrap());
        drop(transaction::Transaction::open(&d.0).unwrap());
        assert_eq!(lifecycle::active_task(&d.0).unwrap().unwrap().task_id, next);
        assert_eq!(fs::read(old.dir.join("gate.json")).unwrap(), old_bytes);
        let pointer = json!({"schema":2,"task_id":"task-successor"}).to_string();
        fs::write(d.0.join("active.json"), &pointer).unwrap();
        fs::write(d.0.join("transaction.json"), journal.to_string()).unwrap();
        assert!(transaction::Transaction::open(&d.0).is_err());
        assert_eq!(
            fs::read_to_string(d.0.join("active.json")).unwrap(),
            pointer
        );
    }
}

#[test]
fn project_identity_isolates_legacy_collisions_aliases_and_overrides() {
    let d = dir();
    let a = d.0.join("a-b");
    let b = d.0.join("a_b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    assert_eq!(
        location::legacy_project_key(&a),
        location::legacy_project_key(&b)
    );
    assert_ne!(location::project_key(&a), location::project_key(&b));
    assert_eq!(
        location::project_key(&a),
        location::project_key(&a.join("."))
    );
    #[cfg(windows)]
    assert_eq!(
        location::project_key(&a),
        location::project_key(&PathBuf::from(a.to_string_lossy().to_uppercase()))
    );
    let base = d.0.join("state");
    let lookup =
        |name: &str| (name == "SUPERVISOR_STATE_DIR").then(|| base.to_string_lossy().into_owned());
    let legacy = base.join(location::legacy_project_key(&a));
    fs::create_dir_all(&legacy).unwrap();
    let original = contract().to_string();
    fs::write(legacy.join("contract.json"), &original).unwrap();
    let (a_root, _) = location::resolve_state_dir_checked(&a, &lookup).unwrap();
    let (b_root, _) = location::resolve_state_dir_checked(&b, &lookup).unwrap();
    assert_ne!(a_root, b_root);
    assert!(lifecycle::start(&a_root, &contract(), false, None, None).is_err());
    assert!(lifecycle::start(&b_root, &contract(), false, None, None).is_err());
    lifecycle::migrate(&a_root, &legacy).unwrap();
    assert_eq!(
        fs::read_to_string(legacy.join("contract.json")).unwrap(),
        original
    );
    assert!(lifecycle::active_task(&b_root).unwrap().is_none());
    assert_eq!(
        fs::read_to_string(
            lifecycle::active_task(&a_root)
                .unwrap()
                .unwrap()
                .dir
                .join("contract.json")
        )
        .unwrap(),
        original
    );
}

fn git(root: &Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nested_candidate_observes_sibling_edits_and_linked_worktrees_are_distinct() {
    let d = dir();
    let main = d.0.join("main");
    fs::create_dir_all(main.join("nested")).unwrap();
    git(&main, &["init", "--quiet"]);
    fs::write(main.join("sibling.rs"), "before").unwrap();
    fs::write(main.join("nested/file.rs"), "nested").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "--quiet", "-m", "fixture"]);
    let provider = GitCandidate {
        cwd: main.join("nested"),
    };
    let before = provider.observe().unwrap();
    fs::write(main.join("sibling.rs"), "after").unwrap();
    let after = provider.observe().unwrap();
    assert_ne!(before.id, after.id);
    assert_eq!(after.changed_paths, vec!["sibling.rs"]);
    assert_eq!(
        location::project_key(&main),
        location::project_key(&main.join("nested"))
    );
    let linked = d.0.join("linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "fixture-linked",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    assert_ne!(location::project_key(&main), location::project_key(&linked));
    #[cfg(unix)]
    {
        let alias = d.0.join("alias");
        std::os::unix::fs::symlink(&main, &alias).unwrap();
        assert_eq!(location::project_key(&main), location::project_key(&alias));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let alias = d.0.join("alias");
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&alias)
            .arg(&main)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(location::project_key(&main), location::project_key(&alias));
    }
}

struct FixedCandidate;
impl CandidateProvider for FixedCandidate {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: "unchanged".into(),
            },
            provenance: "fixture".into(),
            changed_paths: vec![],
        })
    }
}

#[test]
fn migrated_legacy_success_is_preserved_but_cannot_authorize_hook_completion() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    let d = dir();
    let legacy = d.0.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("contract.json"), contract().to_string()).unwrap();
    let (_, digest) = state::read_contract(&legacy).unwrap();
    let candidate = FixedCandidate.observe().unwrap().id;
    let mut gate = state::Gate::new(
        "user-contract".into(),
        Checkpoint {
            state: TaskState::Completed,
            candidate: Some(candidate.clone()),
            open_findings: vec![],
            consumed_repair_rounds: 1,
            next_action: "done".into(),
        },
    );
    gate.receipts.push(VerificationReceipt {
        criterion: CriterionId("C2".into()),
        candidate,
        command: "cargo test".into(),
        cwd: d.0.to_string_lossy().into_owned(),
        exit_code: Some(0),
        output_ref: "old-unbound-run".into(),
        claim_supported: true,
        started_at: 1,
        finished_at: 1,
    });
    state::save_gate(&legacy, &gate, &digest).unwrap();
    let original = fs::read(legacy.join("gate.json")).unwrap();
    let root = d.0.join("new-state");
    lifecycle::migrate(&root, &legacy).unwrap();
    let mut env = Env {
        state_dir: root,
        state_location: "test",
        candidate: Box::new(FixedCandidate),
        clock: || 2,
    };
    let result = handle(
        &json!({"cwd":d.0,"session_id":"new-session","hook_event_name":"Stop","last_assistant_message":"Done.","stop_hook_active":false}),
        &mut env,
    );
    assert!(
        matches!(result,Decision::Block(ref reason) if reason.contains("missing C2")),
        "{result:?}"
    );
    assert_eq!(fs::read(legacy.join("gate.json")).unwrap(), original);
}
