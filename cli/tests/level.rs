//! `supervisor level` and the hook at each level, run as child processes with the state root
//! and the user config directory inside a temp directory.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::resolve_state_dir_read_only;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
    config: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    /// A git repository with one commit, no level files and no state yet.
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-level-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        let f = Self {
            project: root.join("project"),
            state: root.join("state"),
            config: root.join("config"),
            root,
        };
        fs::write(f.project.join("README"), "one\n").unwrap();
        for args in [
            &["init", "-q"][..],
            &["add", "README"],
            &["commit", "-q", "-m", "one"],
        ] {
            let out = f
                .command("git")
                .arg("-C")
                .arg(&f.project)
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
    /// No user, system or inherited git configuration; no repository above the fixture root.
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("HOME", &self.root)
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .env("SUPERVISOR_CONFIG_DIR", &self.config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY");
        cmd
    }
    fn supervisor(&self, args: &[&str]) -> Output {
        self.command(BIN).args(args).output().unwrap()
    }
    fn level(&self, args: &[&str]) -> Output {
        let mut all = vec!["level"];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--project", self.project.to_str().unwrap()]);
        self.supervisor(&all)
    }
    fn set(&self, args: &[&str]) {
        let out = self.level(args);
        assert_eq!(out.status.code(), Some(0), "level {args:?}: {out:?}");
    }
    fn state_root(&self) -> PathBuf {
        let state = self.state.to_string_lossy().into_owned();
        resolve_state_dir_read_only(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| state.clone())
        })
        .unwrap()
        .0
    }
    /// An explicitly started task whose scope is `src/` and whose one criterion has no receipt.
    fn start_task(&self) -> PathBuf {
        let contract = self.root.join("contract.json");
        fs::write(&contract, json!({"id":"level-fixture","goal":"exercise levels","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let out = self.supervisor(&[
            "task",
            "start",
            "--contract",
            contract.to_str().unwrap(),
            "--project",
            self.project.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "task start: {out:?}");
        lifecycle::active_task(&self.state_root())
            .unwrap()
            .unwrap()
            .dir
    }
    /// One hook call; the hook always exits 0 and prints at most one decision.
    fn hook(&self, mut payload: Value) -> Value {
        payload["cwd"] = self.project.to_str().unwrap().into();
        payload["session_id"] = "level-session".into();
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
    fn stop(&self) -> Value {
        self.hook(json!({"hook_event_name":"Stop","last_assistant_message":"Done, all tests pass.","stop_hook_active":false}))
    }
    fn tool(&self, id: &str, tool: &str, input: Value) -> Value {
        self.hook(json!({"hook_event_name":"PreToolUse","tool_use_id":id,"tool_name":tool,"tool_input":input}))
    }
}

fn stdout(out: &Output) -> &str {
    std::str::from_utf8(&out.stdout).unwrap()
}
fn denied(decision: &Value) -> bool {
    decision["hookSpecificOutput"]["permissionDecision"] == "deny"
}
fn continuations(task: &std::path::Path) -> Value {
    let gate: Value =
        serde_json::from_str(&fs::read_to_string(task.join("gate.json")).unwrap()).unwrap();
    gate["continuations"].clone()
}

#[test]
fn level_command_shows_and_writes_with_documented_output_and_exit_codes() {
    let f = Fixture::new("cli");
    let out = f.level(&[]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(0), "level: full (default)\n")
    );
    let out = f.level(&["lite", "--global"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        stdout(&out),
        format!(
            "level lite written (global): {}\n",
            f.config.join("level").display()
        )
    );
    assert_eq!(stdout(&f.level(&[])), "level: lite (global)\n");
    let out = f.level(&["ultra"]);
    assert_eq!(
        stdout(&out),
        format!(
            "level ultra written (project): {}\n",
            f.state_root().join("level").display()
        )
    );
    assert_eq!(stdout(&f.level(&[])), "level: ultra (project)\n");
    for usage in [&["bogus"][..], &["--global"], &["full", "--bogus"]] {
        let out = f.level(usage);
        assert_eq!(out.status.code(), Some(2), "{usage:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{usage:?}: {out:?}");
    }
    fs::write(f.state_root().join("level"), "nope\n").unwrap();
    let out = f.level(&[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown level \"nope\""),
        "{out:?}"
    );
}

#[test]
fn hook_at_off_writes_no_state_and_still_denies_gate_state_writes() {
    let f = Fixture::new("off");
    fs::create_dir_all(&f.config).unwrap();
    fs::write(f.config.join("level"), "off\n").unwrap();
    let prompt = f.hook(json!({"hook_event_name":"UserPromptSubmit","prompt":"change the code"}));
    assert_eq!(prompt, Value::Null);
    assert_eq!(f.stop(), Value::Null);
    let edit = f.tool(
        "t1",
        "Write",
        json!({"file_path":f.project.join("README"),"content":"x"}),
    );
    assert_eq!(edit, Value::Null);
    let state = f.tool(
        "t2",
        "Write",
        json!({"file_path":f.state_root().join("active.json"),"content":"{}"}),
    );
    assert!(denied(&state), "{state}");
    assert!(
        !f.state.exists(),
        "off created state under {}",
        f.state.display()
    );
}

#[test]
fn hook_at_lite_reports_a_stop_refusal_without_blocking_or_counting_it() {
    let f = Fixture::new("lite");
    let task = f.start_task();
    let full = f.stop();
    assert_eq!(full["decision"], "block", "control at full: {full}");
    assert_eq!(continuations(&task), 1);
    f.set(&["lite"]);
    let lite = f.stop();
    let message = lite["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("GATE refuses completion") && message.contains("missing C2"),
        "{lite}"
    );
    assert!(lite.get("decision").is_none(), "{lite}");
    assert_eq!(continuations(&task), 1, "a reported refusal is not counted");
    let events = fs::read_to_string(task.join("events.jsonl")).unwrap();
    assert!(
        events.contains("level lite: reported, not blocked"),
        "{events}"
    );
}

#[test]
fn hook_at_ultra_denies_writes_outside_the_contract_scope() {
    let f = Fixture::new("ultra");
    f.start_task();
    let readme = json!({"file_path":f.project.join("README"),"content":"x"});
    let full = f.tool("t1", "Write", readme.clone());
    assert!(!denied(&full), "control at full: {full}");
    f.set(&["ultra"]);
    let outside = f.tool("t2", "Write", readme);
    assert!(denied(&outside), "{outside}");
    assert!(
        outside["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("outside the task's contract scope (level ultra)"),
        "{outside}"
    );
    for (id, path) in [
        ("t3", f.project.join("src/lib.rs")),
        ("t4", PathBuf::from("src/new.rs")),
    ] {
        let inside = f.tool(id, "Write", json!({"file_path":path,"content":"x"}));
        assert!(!denied(&inside), "{path:?}: {inside}");
    }
    let edit = f.tool(
        "t5",
        "MultiEdit",
        json!({"file_path":f.project.join("README"),"edits":[]}),
    );
    assert!(denied(&edit), "{edit}");
}

#[test]
fn agents_cannot_change_the_level() {
    let f = Fixture::new("guard");
    f.start_task();
    let show = f.tool("t0", "Bash", json!({"command":"supervisor level"}));
    assert!(!denied(&show), "showing the level stays allowed: {show}");
    let attempts = [
        ("Bash", json!({"command":"supervisor level off"})),
        (
            "Bash",
            json!({"command":"cd x && \"./target/debug/supervisor.exe\" level lite --global"}),
        ),
        (
            "PowerShell",
            json!({"command":"& C:\\bin\\supervisor.exe level off"}),
        ),
        (
            "mcp__terminal_commander__run_and_watch",
            json!({"argv":["supervisor","level","off"]}),
        ),
        (
            "Write",
            json!({"file_path":f.config.join("level"),"content":"off"}),
        ),
        (
            "Bash",
            json!({"command":format!("echo off > {}", f.config.join("level").display())}),
        ),
        (
            "MultiEdit",
            json!({"file_path":f.state_root().join("level"),"edits":[]}),
        ),
    ];
    for (i, (tool, input)) in attempts.into_iter().enumerate() {
        let decision = f.tool(&format!("t{}", i + 1), tool, input.clone());
        assert!(denied(&decision), "{tool} {input}: {decision}");
    }
}

#[test]
fn unreadable_level_file_runs_the_hook_at_full_and_journals_why() {
    let f = Fixture::new("bad");
    let task = f.start_task();
    fs::create_dir_all(&f.config).unwrap();
    fs::write(f.config.join("level"), "off-ish\n").unwrap();
    let stop = f.stop();
    assert_eq!(stop["decision"], "block", "{stop}");
    let events = fs::read_to_string(task.join("events.jsonl")).unwrap();
    assert!(events.contains("unknown level \\\"off-ish\\\""), "{events}");
}
