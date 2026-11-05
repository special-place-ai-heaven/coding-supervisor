//! A session whose prompt is registered in a repository's main checkout and that then works in a
//! linked worktree of the same repository (Claude Code's `EnterWorktree`, or a hook `cwd` that
//! follows the agent). Linked worktrees keep their own gate state, so the session has no
//! registered prompt there; that must not make the worktree an ungated place to edit.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

struct Fixture {
    root: PathBuf,
    main: PathBuf,
    linked: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-worktree-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("main")).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        let f = Self {
            main: root.join("main"),
            linked: root.join("linked"),
            root,
        };
        fs::write(f.main.join("README"), "one\n").unwrap();
        fs::write(
            f.main.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let linked = f.linked.to_str().unwrap().to_owned();
        for args in [
            &["init", "-q"][..],
            &["add", "README", "Cargo.toml"],
            &["commit", "-q", "-m", "one"],
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "fixture-linked",
                &linked,
                "HEAD",
            ],
        ] {
            let out = f
                .command("git")
                .arg("-C")
                .arg(&f.main)
                .args([
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        }
        f
    }
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("HOME", &self.root)
            .env("SUPERVISOR_STATE_DIR", self.root.join("state"))
            .env("SUPERVISOR_CONFIG_DIR", self.root.join("config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY");
        cmd
    }
    fn hook(&self, cwd: &Path, mut payload: Value) -> Value {
        payload["cwd"] = cwd.to_str().unwrap().into();
        if payload.get("session_id").is_none() {
            payload["session_id"] = "worktree-session".into();
        }
        let mut child = self
            .command(BIN)
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        }
    }
    fn write(&self, cwd: &Path, id: &str) -> Value {
        let file = self.linked.join("src").join("lib.rs");
        self.hook(
            cwd,
            json!({"hook_event_name":"PreToolUse","tool_use_id":id,"tool_name":"Write",
                "tool_input":{"file_path":file.to_str().unwrap(),"content":"// edit\n"}}),
        )
    }
}

fn denied(decision: &Value) -> bool {
    decision["hookSpecificOutput"]["permissionDecision"] == "deny"
}

#[test]
fn a_prompt_registered_in_the_main_checkout_does_not_leave_a_linked_worktree_ungated() {
    let f = Fixture::new("ungated");
    let prompt = f.hook(
        &f.main,
        json!({"hook_event_name":"UserPromptSubmit","prompt_id":"p1","prompt":"Implement a change"}),
    );
    assert!(
        prompt.to_string().contains("intent_apply"),
        "the main checkout registers the prompt: {prompt}"
    );
    // The unclassified prompt already denies edits from the main checkout.
    assert!(denied(&f.write(&f.main, "from-main")));
    // The same session, now working in the linked worktree, must not be able to edit there.
    let decision = f.write(&f.linked, "from-linked");
    assert!(
        denied(&decision),
        "linked worktree edit was allowed: {decision}"
    );
    let reason = decision["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("worktree"), "{reason}");
}

/// The guard only covers a session registered in the main checkout: a prompt registered in the
/// worktree itself is governed there, and a session with no prompt anywhere is untouched.
#[test]
fn the_linked_worktree_guard_leaves_worktree_and_unregistered_sessions_alone() {
    let f = Fixture::new("scoped");
    f.hook(
        &f.linked,
        json!({"hook_event_name":"UserPromptSubmit","prompt_id":"p1","prompt":"Implement a change"}),
    );
    let decision = f.write(&f.linked, "own-prompt");
    let reason = decision["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default();
    assert!(
        denied(&decision),
        "the pending prompt still gates: {decision}"
    );
    assert!(!reason.contains("main checkout"), "{reason}");
    let mut other = json!({"hook_event_name":"PreToolUse","tool_use_id":"other","tool_name":"Write",
        "tool_input":{"file_path":f.linked.join("x.rs").to_str().unwrap(),"content":""}});
    other["session_id"] = "unrelated-session".into();
    let reason = f.hook(&f.linked, other)["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!reason.contains("main checkout"), "{reason}");
}

// G5: a task bound to a linked worktree of the same repository.

impl Fixture {
    /// The state root holding the main checkout's registered prompts and tasks.
    fn state_root(&self) -> PathBuf {
        fs::read_dir(self.root.join("state"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.join("intents.json").exists())
            .expect("a registered state root")
    }

    /// Register a work prompt in the main checkout and apply `start` with `worktree`, when given.
    fn start(&self, worktree: Option<&Path>) -> std::process::Output {
        self.hook(
            &self.main,
            json!({"hook_event_name":"UserPromptSubmit","prompt_id":"p1","prompt":"Implement a change"}),
        );
        let store: Value = serde_json::from_str(
            &fs::read_to_string(self.state_root().join("intents.json")).unwrap(),
        )
        .unwrap();
        let (id, event) = store["events"].as_object().unwrap().iter().next().unwrap();
        let mut proposal = json!({"event_id":id,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"start","task_id":null,
            "contract":{"id":"g5","goal":"work in a worktree","scope":["src/"],"forbidden_effects":[],
                "max_repair_rounds":2,
                "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]}});
        if let Some(worktree) = worktree {
            proposal["worktree"] = worktree.to_str().unwrap().into();
        }
        self.command(BIN)
            .args(["intent", "apply", "--project"])
            .arg(&self.main)
            .args(["--proposal", &proposal.to_string()])
            .output()
            .unwrap()
    }

    /// Run every mandatory check of the task, the way the contract names it.
    fn run_check(&self, cwd: &Path, id: &str) {
        let task = fs::read_dir(self.state_root().join("tasks"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let contract: Value =
            serde_json::from_str(&fs::read_to_string(task.join("contract.json")).unwrap()).unwrap();
        for (n, criterion) in contract["criteria"].as_array().unwrap().iter().enumerate() {
            let command = match &criterion["command"] {
                Value::Array(list) => list[0].as_str().unwrap(),
                other => other.as_str().unwrap(),
            };
            let mut payload = json!({"hook_event_name":"PreToolUse","tool_use_id":format!("{id}-{n}"),
                "agent_id":"main","tool_name":"Bash","tool_input":{"command":command}});
            assert!(!denied(&self.hook(cwd, payload.clone())));
            payload["hook_event_name"] = "PostToolUse".into();
            payload["tool_response"] = json!({"interrupted":false});
            self.hook(cwd, payload);
        }
    }

    fn stop(&self, cwd: &Path) -> Value {
        self.hook(
            cwd,
            json!({"hook_event_name":"Stop","stop_hook_active":false}),
        )
    }

    fn edit_in_linked(&self) {
        assert!(!denied(&self.write(&self.linked, "edit")));
        fs::create_dir_all(self.linked.join("src")).unwrap();
        fs::write(self.linked.join("src").join("lib.rs"), "// edit\n").unwrap();
    }

    fn completed(&self) -> bool {
        fs::read_dir(self.state_root().join("tasks"))
            .unwrap()
            .any(|t| {
                fs::read_to_string(t.unwrap().path().join("gate.json"))
                    .is_ok_and(|g| g.contains("\"state\":\"completed\""))
            })
    }
}

fn blocked(decision: &Value) -> bool {
    decision["decision"] == "block"
}

#[test]
fn a_task_bound_to_a_linked_worktree_completes_from_it() {
    let f = Fixture::new("bound");
    let applied = f.start(Some(&f.linked));
    assert!(applied.status.success(), "{applied:?}");
    f.edit_in_linked();
    f.run_check(&f.linked, "run-1");
    let stop = f.stop(&f.linked);
    assert!(!blocked(&stop), "{stop}");
    assert!(f.completed());
}

#[test]
fn a_worktree_of_another_repository_is_refused_at_start() {
    let f = Fixture::new("bound-foreign");
    let other = Fixture::new("bound-other");
    let applied = f.start(Some(&other.linked));
    assert!(!applied.status.success());
    let why = String::from_utf8_lossy(&applied.stderr);
    assert!(why.contains("not a worktree of this repository"), "{why}");
}

#[test]
fn an_unbound_task_still_denies_edits_in_a_linked_worktree() {
    let f = Fixture::new("unbound");
    let applied = f.start(None);
    assert!(applied.status.success(), "{applied:?}");
    let decision = f.write(&f.linked, "edit");
    assert!(denied(&decision), "{decision}");
    let reason = decision["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("worktree"), "{reason}");
}

#[test]
fn a_check_run_in_the_main_checkout_does_not_count_for_a_bound_task() {
    let f = Fixture::new("bound-main-run");
    assert!(f.start(Some(&f.linked)).status.success());
    f.edit_in_linked();
    f.run_check(&f.main, "run-main");
    let stop = f.stop(&f.main);
    assert!(blocked(&stop), "{stop}");
    assert!(!f.completed());
}

#[test]
fn a_revise_naming_the_main_checkout_clears_the_binding() {
    let f = Fixture::new("bound-cleared");
    assert!(f.start(Some(&f.linked)).status.success());
    let intents = || -> Value {
        serde_json::from_str(&fs::read_to_string(f.state_root().join("intents.json")).unwrap())
            .unwrap()
    };
    let before: Vec<String> = intents()["events"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    f.hook(
        &f.main,
        json!({"hook_event_name":"UserPromptSubmit","prompt_id":"p2","prompt":"Work in the main checkout instead"}),
    );
    let store = intents();
    let (id, event) = store["events"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(id, _)| !before.contains(id))
        .unwrap();
    let task = fs::read_dir(f.state_root().join("tasks"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contract: Value =
        serde_json::from_str(&fs::read_to_string(task.join("contract.json")).unwrap()).unwrap();
    let proposal = json!({"event_id":id,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
        "action":"revise","task_id":task.file_name().unwrap().to_str().unwrap(),
        "contract":contract,"worktree":f.main.to_str().unwrap()});
    let out = f
        .command(BIN)
        .args(["intent", "apply", "--project"])
        .arg(&f.main)
        .args(["--proposal", &proposal.to_string()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let metadata = fs::read_to_string(task.join("autonomy.json")).unwrap();
    assert!(!metadata.contains("\"worktree\""), "{metadata}");
}

#[test]
fn a_worktree_that_does_not_exist_yet_binds_when_it_is_created() {
    let f = Fixture::new("bound-later");
    let later = f.root.join("later");
    assert!(f.start(Some(&later)).status.success());
    let stop = f.stop(&f.main);
    assert!(blocked(&stop), "{stop}");
    let out = f
        .command("git")
        .arg("-C")
        .arg(&f.main)
        .args(["worktree", "add", "-q", "-b", "fixture-later"])
        .arg(&later)
        .arg("HEAD")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    fs::create_dir_all(later.join("src")).unwrap();
    fs::write(later.join("src").join("lib.rs"), "// edit\n").unwrap();
    f.run_check(&later, "run-later");
    let stop = f.stop(&later);
    assert!(!blocked(&stop), "{stop}");
    assert!(f.completed());
}
