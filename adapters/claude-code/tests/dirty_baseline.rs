//! G48: a task is not charged for files that were already changed when it started. The start
//! records those files with their content; at a stop, one that still has that content is not
//! this task's edit. Real git; replay of hook payloads, not a live harness run.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::autonomy;
use supervisor_claude_code::candidate::GitCandidate;
use supervisor_claude_code::level::{Level, Setting};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    env: Env,
    task: String,
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
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repository whose tree already holds another task's uncommitted work outside `src/` (a
/// modified tracked file and a new one), and a task with scope `src/` started on top of it.
fn started_on_a_dirty_tree() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "supervisor-baseline-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("project"), root.join("state"));
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("other")).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(project.join("src/lib.rs"), "one\n").unwrap();
    fs::write(project.join("other/notes.txt"), "one\n").unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    fs::write(project.join("other/notes.txt"), "earlier task\n").unwrap();
    fs::write(project.join("other/new.txt"), "earlier task\n").unwrap();
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(GitCandidate {
            cwd: project.clone(),
        }),
        clock: || 1_000,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    };
    let event = autonomy::record_prompt(
        &env.state_dir,
        &project,
        &json!({"session_id":"session","prompt_id":"p1","prompt":"Implement a small change"}),
    )
    .unwrap();
    let started = autonomy::propose(
        &env.state_dir,
        &project,
        &json!({"event_id":event["event_id"],"nonce":event["nonce"],
            "prompt_hash":event["prompt_hash"],"action":"start",
            "contract":{"id":"baseline","goal":"edit src","scope":["src/"],
                "forbidden_effects":[],"max_repair_rounds":2,
                "criteria":[{"id":"C2","check":"test","mandatory":true,
                    "command":"cargo test -p fixture"}]}}),
    )
    .unwrap();
    let task = started["task_id"].as_str().unwrap().to_owned();
    Fixture {
        root,
        project,
        env,
        task,
    }
}

impl Fixture {
    fn hook(&mut self, mut payload: Value) -> Decision {
        payload["cwd"] = json!(self.project);
        payload["session_id"] = "session".into();
        handle(&payload, &mut self.env)
    }

    /// The task's own edit, in scope, then every mandatory check once with exit 0.
    fn work_and_checks(&mut self) {
        fs::write(self.project.join("src/lib.rs"), "two\n").unwrap();
        let dir = self.env.state_dir.join("tasks").join(&self.task);
        let contract: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("contract.json")).unwrap()).unwrap();
        for (n, criterion) in contract["criteria"].as_array().unwrap().iter().enumerate() {
            if criterion["mandatory"] != true {
                continue;
            }
            let command = match &criterion["command"] {
                Value::Array(list) => list[0].clone(),
                one => one.clone(),
            };
            for kind in ["PreToolUse", "PostToolUse"] {
                let d = self.hook(json!({"hook_event_name":kind,"agent_id":"main",
                    "tool_use_id":format!("c-{n}"),"tool_name":"Bash",
                    "tool_input":{"command":command},"tool_response":{"interrupted":false}}));
                assert_eq!(d, Decision::None, "{kind} {command}");
            }
        }
    }

    fn stop(&mut self) -> Decision {
        self.hook(json!({"hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"Done."}))
    }
}

/// SymForge report 2026-11-10, item 3: the stop gate listed an earlier task's uncommitted files
/// as this task's out-of-scope edits and charged a repair round.
#[test]
fn files_changed_before_the_task_started_are_not_its_out_of_scope_edits() {
    let mut f = started_on_a_dirty_tree();
    f.work_and_checks();
    assert_eq!(f.stop(), Decision::None);
}

#[test]
fn a_further_edit_to_an_inherited_file_is_still_out_of_scope() {
    let mut f = started_on_a_dirty_tree();
    fs::write(f.project.join("other/notes.txt"), "this task\n").unwrap();
    f.work_and_checks();
    let Decision::Block(why) = f.stop() else {
        panic!("an out-of-scope edit made by this task must be refused");
    };
    assert!(
        why.contains("other/notes.txt") && !why.contains("other/new.txt"),
        "{why}"
    );
}

#[test]
fn a_file_first_changed_after_the_start_is_still_out_of_scope() {
    let mut f = started_on_a_dirty_tree();
    fs::write(f.project.join("other/later.txt"), "this task\n").unwrap();
    f.work_and_checks();
    let Decision::Block(why) = f.stop() else {
        panic!("an out-of-scope edit made by this task must be refused");
    };
    assert!(why.contains("other/later.txt"), "{why}");
}

/// Deleting an inherited file is an edit of it too.
#[test]
fn deleting_an_inherited_file_is_still_out_of_scope() {
    let mut f = started_on_a_dirty_tree();
    fs::remove_file(f.project.join("other/notes.txt")).unwrap();
    f.work_and_checks();
    let Decision::Block(why) = f.stop() else {
        panic!("an out-of-scope edit made by this task must be refused");
    };
    assert!(why.contains("other/notes.txt"), "{why}");
}
