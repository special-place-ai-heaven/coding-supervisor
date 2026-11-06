//! G9: evidence transfers across `git commit` only when git says the tree content is identical.
//! These tests run real git; the state directory lives outside the repository.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::GitCandidate;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    project: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(project: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repository with two committed source files, both then modified, and a one-criterion task.
fn setup() -> (Fixture, Env) {
    let root = std::env::temp_dir().join(format!(
        "supervisor-tree-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let (project, state) = (root.join("project"), root.join("state"));
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(project.join("src/a.rs"), "a1\n").unwrap();
    fs::write(project.join("src/b.rs"), "b1\n").unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    fs::write(project.join("src/a.rs"), "a2\n").unwrap();
    fs::write(project.join("src/b.rs"), "b2\n").unwrap();
    fs::write(
        state.join("contract.json"),
        json!({"id":"tree", "goal":"transfer", "scope":["src/"], "forbidden_effects":[],
            "max_repair_rounds":2,
            "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]})
        .to_string(),
    )
    .unwrap();
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(GitCandidate {
            cwd: project.clone(),
        }),
        clock: || 100,
        level: Default::default(),
    };
    (Fixture { root, project }, env)
}

fn payload(f: &Fixture, kind: &str) -> Value {
    json!({"hook_event_name":kind, "cwd":f.project, "session_id":"session", "agent_id":"main",
        "tool_use_id":"run", "tool_name":"Bash", "tool_input":{"command":"cargo test"}})
}

fn passing_run(f: &Fixture, env: &mut Env) {
    assert_eq!(handle(&payload(f, "PreToolUse"), env), Decision::None);
    let mut post = payload(f, "PostToolUse");
    post["tool_response"] = json!({"interrupted":false});
    assert_eq!(handle(&post, env), Decision::None);
}

fn stop(f: &Fixture, env: &mut Env) -> Decision {
    handle(
        &json!({"hook_event_name":"Stop","cwd":f.project,"session_id":"session",
            "agent_id":"main","last_assistant_message":"Done.","stop_hook_active":false}),
        env,
    )
}

fn state_of(env: &Env) -> String {
    let dir = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .map_or_else(|| env.state_dir.clone(), |task| task.dir);
    let gate: Value = serde_json::from_slice(&fs::read(dir.join("gate.json")).unwrap()).unwrap();
    gate["state"].as_str().unwrap().to_owned()
}

fn accepted(f: &Fixture, env: &mut Env) {
    assert_eq!(stop(f, env), Decision::None);
    assert_eq!(state_of(env), "completed");
    // The second Stop reads the transferred receipt back through the journal.
    assert_eq!(stop(f, env), Decision::None);
}

fn refused(f: &Fixture, env: &mut Env) {
    let Decision::Block(why) = stop(f, env) else {
        panic!("expected a refusal");
    };
    assert_ne!(state_of(env), "completed", "{why}");
}

#[test]
fn commit_all_after_a_passing_run_keeps_the_evidence() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    accepted(&f, &mut env);
}

#[test]
fn an_amend_that_only_changes_the_message_keeps_the_evidence() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    git(
        &f.project,
        &["commit", "-q", "--amend", "-m", "better words"],
    );
    accepted(&f, &mut env);
}

#[test]
fn an_edit_after_the_commit_makes_the_evidence_stale() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    refused(&f, &mut env);
}

/// The worktree content is what is identified, not the commit: committing only `a.rs` while
/// `b.rs` stays dirty leaves the same bytes on disk, so the evidence still holds.
#[test]
fn a_partial_commit_that_leaves_the_rest_dirty_keeps_the_evidence() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(
        &f.project,
        &["commit", "-q", "-m", "half", "--", "src/a.rs"],
    );
    accepted(&f, &mut env);
}

#[test]
fn a_partial_commit_whose_rest_is_discarded_makes_the_evidence_stale() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(
        &f.project,
        &["commit", "-q", "-m", "half", "--", "src/a.rs"],
    );
    git(&f.project, &["checkout", "--", "src/b.rs"]);
    refused(&f, &mut env);
}

#[test]
fn a_dispatch_row_without_a_tree_keeps_the_exact_rule() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    let task = supervisor_claude_code::lifecycle::active_task(&env.state_dir).unwrap();
    let dir = task.map_or_else(|| env.state_dir.clone(), |t| t.dir);
    let path = env.state_dir.join("runs.jsonl");
    let path = if path.exists() {
        path
    } else {
        dir.join("runs.jsonl")
    };
    let rows: Vec<String> = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).unwrap();
            row.as_object_mut().unwrap().remove("tree");
            row["identity_schema"] = 3.into();
            row.to_string()
        })
        .collect();
    fs::write(&path, rows.join("\n") + "\n").unwrap();
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    refused(&f, &mut env);
}

/// G30: the advisory view (`task_state`) sees what Stop will count: a passing run follows a
/// commit of identical content and is gone once the content changes.
#[test]
fn observed_criteria_follow_the_tree_after_a_commit() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    let observed = |env: &Env| {
        supervisor_claude_code::adapter::observed_criteria(&env.state_dir, &f.project).unwrap()
    };
    assert_eq!(observed(&env), vec!["C2".to_owned()]);
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    assert!(observed(&env).is_empty());
}
