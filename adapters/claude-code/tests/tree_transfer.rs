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

/// WP7: a receipt bundle under `supervisor-receipts/`, written beside the work and not
/// committed, is outside the candidate: no stale evidence and no out-of-scope path.
#[test]
fn an_uncommitted_receipt_bundle_keeps_the_evidence() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    fs::create_dir_all(f.project.join("supervisor-receipts")).unwrap();
    fs::write(f.project.join("supervisor-receipts/t.json"), "{}\n").unwrap();
    accepted(&f, &mut env);
}

/// WP7: committing the work and then the bundle leaves the tree identity unchanged, in a project
/// with no documentation list (no other exclusion is in play).
#[test]
fn committing_a_receipt_bundle_keeps_the_evidence() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    git(&f.project, &["commit", "-q", "-a", "-m", "work"]);
    fs::create_dir_all(f.project.join("supervisor-receipts")).unwrap();
    fs::write(f.project.join("supervisor-receipts/t.json"), "{}\n").unwrap();
    git(&f.project, &["add", "supervisor-receipts"]);
    git(&f.project, &["commit", "-q", "-m", "receipt"]);
    accepted(&f, &mut env);
}

/// WP7: the exclusion is the folder only; a file beside it still changes the candidate.
#[test]
fn a_file_named_like_the_receipts_folder_still_makes_the_evidence_stale() {
    let (f, mut env) = setup();
    passing_run(&f, &mut env);
    fs::write(f.project.join("supervisor-receipts.json"), "{}\n").unwrap();
    git(&f.project, &["add", "-A"]);
    git(&f.project, &["commit", "-q", "-m", "work"]);
    refused(&f, &mut env);
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
    // Review: a later failing run on the same content wins over the earlier pass.
    let mut pre = payload(&f, "PreToolUse");
    pre["tool_use_id"] = "run-2".into();
    assert_eq!(handle(&pre, &mut env), Decision::None);
    let mut failed = payload(&f, "PostToolUseFailure");
    failed["tool_use_id"] = "run-2".into();
    failed["error"] = "Exit code 1".into();
    handle(&failed, &mut env);
    assert!(observed(&env).is_empty());
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    assert!(observed(&env).is_empty());
}

fn docs_scope(env: &Env) {
    let path = env.state_dir.join("contract.json");
    let contract = fs::read_to_string(&path).unwrap().replace(
        r#""scope":["src/"]"#,
        r#""scope":["src/","docs/","README.md"]"#,
    );
    fs::write(&path, contract).unwrap();
}

fn docs_edit(f: &Fixture) {
    fs::create_dir_all(f.project.join("docs")).unwrap();
    fs::write(f.project.join("docs/LEDGER.md"), "row\n").unwrap();
    fs::write(f.project.join("README.md"), "readme\n").unwrap();
}

/// G28: an edit, even a committed one, to a path the project's own CI treats as documentation
/// only keeps the evidence (owner, 2026-11-06); principle 2 in CLAUDE.md says so.
#[test]
fn a_docs_only_edit_keeps_the_evidence() {
    let (f, mut env) = setup();
    docs_scope(&env);
    let workflows = f.project.join(".github/workflows");
    fs::create_dir_all(&workflows).unwrap();
    fs::write(
        workflows.join("ci.yml"),
        "on:\n  push:\n    paths-ignore:\n      - \"docs/**\"\n      - \"README.md\"\njobs: {}\n",
    )
    .unwrap();
    passing_run(&f, &mut env);
    docs_edit(&f);
    git(&f.project, &["add", "-A"]);
    git(&f.project, &["commit", "-q", "-m", "docs"]);
    accepted(&f, &mut env);
}

/// The one documentation list both the gate and CI read: `.github/docs-paths.txt`, with a CI
/// workflow that has no `paths-ignore` (as this repository's, since every PR reports its checks).
fn docs_paths_file(f: &Fixture) {
    let github = f.project.join(".github");
    fs::create_dir_all(github.join("workflows")).unwrap();
    fs::write(
        github.join("workflows/ci.yml"),
        "on:\n  push:\n    branches: [main]\n  pull_request:\njobs: {}\n",
    )
    .unwrap();
    fs::write(
        github.join("docs-paths.txt"),
        "# Paths CI and the gate treat as documentation only.\ndocs/**\n\nREADME.md\n",
    )
    .unwrap();
}

#[test]
fn a_docs_only_edit_listed_in_docs_paths_keeps_the_evidence() {
    let (f, mut env) = setup();
    docs_scope(&env);
    docs_paths_file(&f);
    passing_run(&f, &mut env);
    docs_edit(&f);
    git(&f.project, &["add", "-A"]);
    git(&f.project, &["commit", "-q", "-m", "docs"]);
    accepted(&f, &mut env);
}

#[test]
fn with_a_docs_paths_list_a_code_edit_makes_the_evidence_stale() {
    let (f, mut env) = setup();
    docs_scope(&env);
    docs_paths_file(&f);
    passing_run(&f, &mut env);
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    refused(&f, &mut env);
}

/// G46: a clean checkout whose committed CI list names the documentation paths, and the task as
/// a writer stores it: v2, based on that commit, with the discovered floor plus `extra`.
fn docs_task(extra: &[Value]) -> (Fixture, Env) {
    let (f, env) = setup();
    git(&f.project, &["checkout", "-q", "--", "."]);
    docs_paths_file(&f);
    git(&f.project, &["add", "-A"]);
    git(&f.project, &["commit", "-q", "-m", "list"]);
    let head = Command::new("git")
        .arg("-C")
        .arg(&f.project)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let floor = |id: &str, command: &str| json!({"id":id, "check":"repository_validation", "mandatory":true, "command":command});
    let mut criteria = vec![
        floor("AUTO_TEST", "cargo test"),
        floor("AUTO_DIFF", "git diff --check"),
    ];
    criteria.extend(extra.iter().cloned());
    fs::write(
        env.state_dir.join("contract.json"),
        json!({"schema_version":2, "isolation":"none",
            "base_sha":String::from_utf8_lossy(&head.stdout).trim(),
            "id":"tree", "goal":"docs", "scope":["src/","docs/","README.md",".github/"],
            "forbidden_effects":[], "max_repair_rounds":2, "criteria":criteria})
        .to_string(),
    )
    .unwrap();
    (f, env)
}

fn diff_check(f: &Fixture, env: &mut Env) {
    let mut pre = payload(f, "PreToolUse");
    pre["tool_use_id"] = "diff".into();
    pre["tool_input"]["command"] = "git diff --check".into();
    assert_eq!(handle(&pre, env), Decision::None);
    let mut post = pre.clone();
    post["hook_event_name"] = "PostToolUse".into();
    post["tool_response"] = json!({"interrupted":false});
    assert_eq!(handle(&post, env), Decision::None);
}

/// G46 (field brief 2026-11-09, change 4a): a candidate that differs from the task's base only
/// in what the base's CI list calls documentation needs no build or test, committed or not.
#[test]
fn a_docs_only_candidate_needs_only_the_diff_check() {
    let (f, mut env) = docs_task(&[]);
    docs_edit(&f);
    diff_check(&f, &mut env);
    accepted(&f, &mut env);
    git(&f.project, &["add", "-A"]);
    git(&f.project, &["commit", "-q", "-m", "docs"]);
    assert_eq!(stop(&f, &mut env), Decision::None);
}

#[test]
fn a_docs_only_candidate_still_needs_the_diff_check() {
    let (f, mut env) = docs_task(&[]);
    docs_edit(&f);
    refused(&f, &mut env);
}

#[test]
fn a_source_edit_beside_the_docs_keeps_the_full_floor() {
    let (f, mut env) = docs_task(&[]);
    docs_edit(&f);
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    diff_check(&f, &mut env);
    refused(&f, &mut env);
}

/// The list is read from the task's base commit, so a candidate cannot widen it.
#[test]
fn a_candidate_that_widens_the_docs_list_keeps_the_full_floor() {
    let (f, mut env) = docs_task(&[]);
    fs::write(
        f.project.join(".github/docs-paths.txt"),
        "docs/**\nsrc/**\n",
    )
    .unwrap();
    fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
    diff_check(&f, &mut env);
    refused(&f, &mut env);
}

/// Review of G46: an index entry marked skip-worktree (or assume-unchanged) hides its edit from
/// the tree, so a project with one keeps every check.
#[test]
fn a_source_edit_hidden_from_the_index_keeps_the_full_floor() {
    for flag in ["--skip-worktree", "--assume-unchanged"] {
        let (f, mut env) = docs_task(&[]);
        git(&f.project, &["update-index", flag, "src/a.rs"]);
        fs::write(f.project.join("src/a.rs"), "a3\n").unwrap();
        docs_edit(&f);
        diff_check(&f, &mut env);
        refused(&f, &mut env);
    }
}

/// Only the discovered floor scales; a criterion the contract's author wrote stays required.
#[test]
fn a_docs_only_candidate_keeps_a_hand_written_criterion() {
    let (f, mut env) = docs_task(&[
        json!({"id":"C2", "check":"repository_validation", "mandatory":true, "command":"make check"}),
    ]);
    docs_edit(&f);
    diff_check(&f, &mut env);
    refused(&f, &mut env);
}

/// Review of G28: a project whose CI names no documentation paths keeps the exact rule.
#[test]
fn without_a_ci_docs_list_a_docs_edit_invalidates() {
    let (f, mut env) = setup();
    docs_scope(&env);
    passing_run(&f, &mut env);
    docs_edit(&f);
    refused(&f, &mut env);
}
