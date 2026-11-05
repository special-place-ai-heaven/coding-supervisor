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
        let linked = f.linked.to_str().unwrap().to_owned();
        for args in [
            &["init", "-q"][..],
            &["add", "README"],
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
        payload["session_id"] = "worktree-session".into();
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
