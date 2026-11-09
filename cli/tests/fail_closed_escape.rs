//! WP1 (spec §9 case 18): while the hooks fail closed on an adapter error, a person can still
//! lower the level from a plain terminal, and `--failure open` lets calls go ahead with the
//! advisory message. Runs the real `supervisor` binary.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use supervisor_claude_code::{lifecycle, location};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    /// A git repository with one commit and an active task at the default level, `enforce`.
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-escape-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        let f = Self {
            project: root.join("project"),
            state: root.join("state"),
            root,
        };
        fs::write(f.project.join("README"), "one\n").unwrap();
        for args in [
            &["init", "-q"][..],
            &["add", "README"],
            &["commit", "-q", "-m", "one"],
        ] {
            let out = f
                .git()
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
        let contract = f.root.join("contract.json");
        fs::write(&contract, json!({"id":"escape","goal":"fixture","criteria":[{"id":"C1","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let out = f.supervisor(&["task", "start", "--contract", contract.to_str().unwrap()]);
        assert!(out.status.success(), "{out:?}");
        f
    }
    /// A person's terminal: no `CLAUDE_*` variable; state, user config and git configuration
    /// inside the fixture.
    fn isolated(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("CLAUDE_") {
                cmd.env_remove(key);
            }
        }
        cmd.env("HOME", &self.root)
            .env("SUPERVISOR_STATE_DIR", &self.state)
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
    fn command(&self) -> Command {
        self.isolated(BIN)
    }
    fn git(&self) -> Command {
        let mut cmd = self.isolated("git");
        cmd.arg("-C").arg(&self.project);
        cmd
    }
    fn supervisor(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .arg("--project")
            .arg(&self.project)
            .output()
            .unwrap()
    }
    fn hook(&self, payload: &Value) -> Value {
        let mut child = self
            .command()
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        }
    }
    fn write(&self) -> Value {
        json!({"cwd":self.project,"session_id":"session-1","hook_event_name":"PreToolUse","tool_name":"Write","tool_use_id":"w1","tool_input":{"file_path":"src/lib.rs","content":"x"}})
    }
    fn stop(&self) -> Value {
        json!({"cwd":self.project,"session_id":"session-1","hook_event_name":"Stop","stop_hook_active":false})
    }
    /// The task's cached answer for the Write no longer holds a decision: replaying that call
    /// is an adapter error while the state directory still resolves.
    fn break_cached_answer(&self) {
        let healthy = self.hook(&self.write());
        assert_ne!(
            healthy["hookSpecificOutput"]["permissionDecision"], "deny",
            "{healthy}"
        );
        assert!(healthy.get("systemMessage").is_none(), "{healthy}");
        let root = location::resolve_state_dir(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| self.state.to_string_lossy().into_owned())
        })
        .0;
        let task = lifecycle::active_task(&root).unwrap().unwrap();
        let path = task.dir.join("applied-events.json");
        let mut events: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for event in events.as_object_mut().unwrap().values_mut() {
            event["response"] = json!({"unexpected":true});
        }
        fs::write(&path, events.to_string()).unwrap();
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn level_off_from_a_plain_terminal_unblocks_a_project_that_fails_closed() {
    let f = Fixture::new("off");
    f.break_cached_answer();
    let denied = f.hook(&f.write());
    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecision"], "deny",
        "{denied}"
    );
    let reason = denied["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(reason.contains("fails closed"), "{reason}");

    let out = f.supervisor(&["level", "off"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let write = f.hook(&f.write());
    assert_ne!(
        write["hookSpecificOutput"]["permissionDecision"], "deny",
        "{write}"
    );
    let stop = f.hook(&f.stop());
    assert_ne!(stop["decision"], "block", "{stop}");
}

#[test]
fn failure_open_lets_the_call_go_ahead_and_is_shown_as_advisory() {
    let f = Fixture::new("open");
    f.break_cached_answer();
    let out = f.supervisor(&["level", "enforce", "--failure", "open"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let decision = f.hook(&f.write());
    let message = decision["systemMessage"].as_str().unwrap_or_default();
    assert!(message.starts_with("GATE adapter error: "), "{decision}");

    let shown = stdout(&f.supervisor(&["level"]));
    assert!(shown.contains("failure: open (advisory)"), "{shown}");
    let status = stdout(&f.supervisor(&["status"]));
    assert!(status.contains("failure: open (advisory)"), "{status}");
    let json: Value = serde_json::from_str(&stdout(&f.supervisor(&["status", "--json"]))).unwrap();
    assert_eq!(json["failure_mode"], "open", "{json}");

    let out = f.supervisor(&["level", "enforce", "--failure", "closed"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        stdout(&f.supervisor(&["level"])).contains("failure: closed"),
        "closed again"
    );
    let out = f.supervisor(&["level", "enforce", "--failure", "ajar"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}
