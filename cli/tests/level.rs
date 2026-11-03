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
        self.start_scoped(json!(["src/"]))
    }
    fn start_scoped(&self, scope: Value) -> PathBuf {
        let contract = self.root.join("contract.json");
        fs::write(&contract, json!({"id":"level-fixture","goal":"exercise levels","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":scope,"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
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
    /// `supervisor <command> --json` for the project; its exit code is not asserted.
    fn report(&self, command: &str) -> Value {
        let out = self.supervisor(&[
            command,
            "--json",
            "--project",
            self.project.to_str().unwrap(),
        ]);
        serde_json::from_str(stdout(&out)).unwrap_or_else(|e| panic!("{command}: {e}: {out:?}"))
    }
}

fn stdout(out: &Output) -> &str {
    std::str::from_utf8(&out.stdout).unwrap()
}
fn denied(decision: &Value) -> bool {
    decision["hookSpecificOutput"]["permissionDecision"] == "deny"
}
fn decision_kind(v: &Value) -> &'static str {
    if denied(v) {
        "deny"
    } else if v["decision"] == "block" {
        "block"
    } else if v.get("systemMessage").is_some() {
        "system"
    } else if v.is_null() {
        "none"
    } else {
        "other"
    }
}
fn events(task: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(task.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
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
        (Some(0), "level: enforce (default)\n")
    );
    let out = f.level(&["lite", "--global"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        stdout(&out),
        format!(
            "level audit written (global): {}\n",
            f.config.join("level").display()
        )
    );
    assert_eq!(stdout(&f.level(&[])), "level: audit (global)\n");
    let out = f.level(&["ultra"]);
    assert_eq!(
        stdout(&out),
        format!(
            "level strict written (project): {}\n",
            f.state_root().join("level").display()
        )
    );
    assert_eq!(stdout(&f.level(&[])), "level: strict (project)\n");
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
        events.contains("level audit: reported, not blocked"),
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
            .contains("outside the task's contract scope (level strict)"),
        "{outside}"
    );
    for (id, path) in [
        ("t3", f.project.join("src/lib.rs")),
        ("t4", PathBuf::from("src/new.rs")),
    ] {
        let inside = f.tool(id, "Write", json!({"file_path":path,"content":"x"}));
        assert!(!denied(&inside), "{path:?}: {inside}");
    }
    // A new directory under another spelling of the project (a symlink here; an 8.3 short
    // name on Windows runners) is still inside.
    #[cfg(unix)]
    {
        let link = f.root.join("link");
        std::os::unix::fs::symlink(&f.project, &link).unwrap();
        let path = link.join("src/deep/new.rs");
        let inside = f.tool("t6", "Write", json!({"file_path":path,"content":"x"}));
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

#[test]
fn agents_cannot_change_the_level_at_lite_or_off() {
    for word in ["lite", "off"] {
        let f = Fixture::new(&format!("kguard-{word}"));
        f.start_task();
        f.set(&[word]);
        let attempts = [
            ("Bash", json!({"command":"supervisor level full"})),
            (
                "Write",
                json!({"file_path":f.config.join("level"),"content":"off"}),
            ),
        ];
        for (i, (tool, input)) in attempts.into_iter().enumerate() {
            let decision = f.tool(&format!("t{i}"), tool, input.clone());
            assert!(denied(&decision), "at {word}: {tool} {input}: {decision}");
        }
    }
}

#[test]
fn showing_the_level_creates_no_state() {
    let f = Fixture::new("kshow");
    let out = f.level(&[]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(0), "level: enforce (default)\n")
    );
    assert!(!f.state.exists(), "show created {}", f.state.display());
}

#[test]
fn lite_reports_a_pretooluse_refusal_instead_of_denying_it() {
    let f = Fixture::new("klitepre");
    let _ = f.hook(json!({"hook_event_name":"UserPromptSubmit","prompt":"change the code"}));
    let write = json!({"file_path":f.project.join("README"),"content":"x"});
    let full = f.tool("t1", "Write", write.clone());
    assert!(denied(&full), "control at full: {full}");
    f.set(&["lite"]);
    let lite = f.tool("t2", "Write", write);
    assert_eq!(decision_kind(&lite), "system", "{lite}");
}

#[test]
fn lite_reports_every_handback_refusal_without_counting_it() {
    let f = Fixture::new("klitesub");
    let task = f.start_task();
    f.set(&["lite"]);
    let handback = json!({"hook_event_name":"SubagentStop","agent_id":"w1","agent_type":"worker","last_assistant_message":"done","stop_hook_active":false});
    for round in 1..=2 {
        let lite = f.hook(handback.clone());
        assert_eq!(decision_kind(&lite), "system", "round {round}: {lite}");
    }
    let gate: Value =
        serde_json::from_str(&fs::read_to_string(task.join("gate.json")).unwrap()).unwrap();
    assert!(gate["handback_refusals"].get("w1").is_none(), "{gate}");
}

#[test]
fn ultra_file_scope_is_exact_not_a_prefix() {
    let f = Fixture::new("kultrafile");
    f.start_scoped(json!(["src/lib.rs"]));
    f.set(&["ultra"]);
    let inside = f.tool(
        "t1",
        "Write",
        json!({"file_path":"src/lib.rs","content":"x"}),
    );
    assert!(!denied(&inside), "{inside}");
    let sibling = f.tool(
        "t2",
        "Write",
        json!({"file_path":"src/lib.rs.bak","content":"x"}),
    );
    assert!(denied(&sibling), "{sibling}");
}

#[test]
fn every_record_stamps_the_effective_level_and_its_source() {
    let f = Fixture::new("stamp");
    let task = f.start_task();
    let assert_stamped = |level: &str, source: &str| {
        let before = fs::read_to_string(task.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .count();
        f.stop();
        let lines = events(&task);
        if level == "off" {
            assert_eq!(lines.len(), before, "off journals nothing");
        } else {
            let last = &lines[lines.len() - 1];
            assert_eq!(
                (&last["effective_level"], &last["level_source"]),
                (&json!(level), &json!(source)),
                "{last}"
            );
        }
        let verdict = f.report("verdict");
        assert_eq!(
            (&verdict["effective_level"], &verdict["level_source"]),
            (&json!(level), &json!(source)),
            "{verdict}"
        );
        // M2: status reports the requested level; with no hooks configured in this fixture
        // nothing can refuse, so a refusing level runs as audit there.
        let status = f.report("status");
        let effective = if matches!(level, "enforce" | "strict") {
            "audit"
        } else {
            level
        };
        assert_eq!(
            (
                &status["requested_level"],
                &status["effective_level"],
                &status["level_source"]
            ),
            (&json!(level), &json!(effective), &json!(source)),
            "{status}"
        );
        assert_eq!(status["schema_version"], 1);
    };
    assert_stamped("enforce", "default");
    fs::create_dir_all(&f.config).unwrap();
    fs::write(f.config.join("level"), "lite\n").unwrap();
    assert_stamped("audit", "global");
    f.set(&["ultra"]);
    assert_stamped("strict", "project");
    f.set(&["off"]);
    assert_stamped("off", "project");
    // An unreadable file falls back to `full`, never to `off` or `lite`: not to the project's
    // previous `off`, not to the global `lite` beneath it, and not from an unreadable global.
    let project = f.state_root().join("level");
    fs::write(&project, "nope\n").unwrap();
    assert_stamped("enforce", "unreadable");
    fs::remove_file(&project).unwrap();
    fs::write(f.config.join("level"), "nope\n").unwrap();
    assert_stamped("enforce", "unreadable");
}

/// Every file under `dir`, with its bytes.
fn snapshot(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn stop_at_off_writes_no_observed_receipt() {
    let f = Fixture::new("offreceipt");
    let task = f.start_task();
    // An agent runs the mandatory check and claims it, then stops.
    let run_and_claim = |id: &str| {
        let _ = f.tool(id, "Bash", json!({"command":"cargo test"}));
        let _ = f.hook(json!({"hook_event_name":"PostToolUse","tool_use_id":id,"tool_name":"Bash",
            "tool_input":{"command":"cargo test"},"tool_response":{"stdout":"ok","stderr":"","interrupted":false}}));
        f.hook(json!({"hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"VERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | cargo test | exit 0"}))
    };
    let c2 = |verdict: &Value| {
        verdict["criteria"]
            .as_array()
            .and_then(|c| c.iter().find(|c| c["id"] == "C2"))
            .cloned()
            .unwrap_or_else(|| panic!("{verdict}"))
    };
    f.set(&["off"]);
    let before = snapshot(&f.state);
    let stop = run_and_claim("t1");
    assert_eq!(stop, Value::Null, "off answers nothing: {stop}");
    // Off writes nothing: no receipt in gate.json, no line in events.jsonl, no run record.
    assert_eq!(
        snapshot(&f.state),
        before,
        "off changed the state directory"
    );
    assert!(!task.join("runs.jsonl").exists(), "off wrote a run record");
    // The consumer gate sees C2 as missing: neither asserted nor observed.
    let verdict = f.report("verdict");
    let off = c2(&verdict);
    assert!(off["receipt"].is_null(), "{verdict}");
    assert!(
        !matches!(off["state"].as_str(), Some("observed" | "asserted")),
        "{verdict}"
    );
    assert_ne!(verdict["verdict"], "pass", "{verdict}");
    // The same run and claim at full is observed, so the payloads above are ones the hook certifies.
    f.set(&["full"]);
    let _ = run_and_claim("t2");
    let verdict = f.report("verdict");
    assert_eq!(c2(&verdict)["state"], "observed", "{verdict}");
}

fn journal_lines(task: &std::path::Path) -> usize {
    fs::read_to_string(task.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .count()
}

/// Every entry under `dir`, directories included, with the bytes of each file.
fn tree(dir: &std::path::Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.push((p.clone(), None));
                stack.push(p);
            } else {
                out.push((p.clone(), Some(fs::read(&p).unwrap())));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn unresolvable_state_root_refuses_with_no_record() {
    let f = Fixture::new("unresolvable");
    let task = f.start_task();
    let root = f.state_root();
    let project = f.project.to_str().unwrap();
    // Control: with a resolvable root the same Stop is refused and journalled, and verdict and
    // status answer, so the assertions below are not true of a fixture that records nothing.
    let before = journal_lines(&task);
    let stop = f.stop();
    assert_eq!(stop["decision"], "block", "control: {stop}");
    assert_eq!(journal_lines(&task), before + 1, "control: Stop journals");
    let verdict = f.supervisor(&["verdict", "--json", "--project", project]);
    assert!(
        serde_json::from_str::<Value>(stdout(&verdict)).is_ok(),
        "control: {verdict:?}"
    );
    for args in [
        &["status", "--project", project][..],
        &["status", "--json", "--project", project],
    ] {
        let status = f.supervisor(args);
        assert!(status.status.success(), "control: {args:?}: {status:?}");
    }
    // The state root now names another worktree, so it cannot be resolved for this project.
    fs::write(
        root.join("project.json"),
        format!(
            "{}\n",
            json!({"schema":2,"canonical_worktree":"/elsewhere"})
        ),
    )
    .unwrap();
    let state = tree(&f.state);
    let lines = journal_lines(&task);
    let stop = f.stop();
    assert_eq!(stop["decision"], "block", "{stop}");
    assert!(
        stop["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("state unavailable"),
        "{stop}"
    );
    let verdict = f.supervisor(&["verdict", "--json", "--project", project]);
    assert_eq!(
        (verdict.status.code(), stdout(&verdict)),
        (Some(1), ""),
        "{verdict:?}"
    );
    for args in [
        &["status", "--project", project][..],
        &["status", "--json", "--project", project],
    ] {
        let status = f.supervisor(args);
        assert!(!status.status.success(), "{args:?}: {status:?}");
    }
    // No record anywhere: no journal line, the state tree byte-identical, no fallback state.
    assert_eq!(journal_lines(&task), lines, "refusal journalled");
    assert!(
        tree(&f.state) == state,
        "refusal changed the state directory"
    );
    assert!(
        !f.project.join(".supervisor").exists(),
        "fallback state created"
    );
}

#[test]
fn agents_cannot_reset_rounds_or_amend_the_floor() {
    let f = Fixture::new("operator");
    f.start_task();
    let attempts = [
        (
            "Bash",
            json!({"command":"supervisor task reset-rounds --task t --reason x"}),
        ),
        (
            "PowerShell",
            json!({"command":"& C:\\bin\\supervisor.exe task amend-floor --criterion C2 --command \"cargo test\" --reason x"}),
        ),
        (
            "mcp__terminal_commander__run_and_watch",
            json!({"argv":["supervisor","task","reset-rounds","--task","t","--reason","x"]}),
        ),
        // K17: ending a stuck task is a person's escape too.
        (
            "Bash",
            json!({"command":"supervisor task cancel --task t --reason x"}),
        ),
        // A person's contract skips the prompt's floor; an agent must propose instead.
        (
            "Bash",
            json!({"command":"supervisor task start --contract c.json"}),
        ),
    ];
    for (i, (tool, input)) in attempts.into_iter().enumerate() {
        let decision = f.tool(&format!("op{i}"), tool, input.clone());
        assert!(denied(&decision), "{tool} {input}: {decision}");
    }
}

/// M1: the modes are off, audit, enforce and strict; lite, full and ultra stay accepted aliases,
/// and a level file written by 0.5.x still reads.
#[test]
fn modes_have_new_names_and_old_names_still_work() {
    let f = Fixture::new("modes");
    assert_eq!(stdout(&f.level(&[])), "level: enforce (default)\n");
    for (word, name) in [
        ("audit", "audit"),
        ("lite", "audit"),
        ("strict", "strict"),
        ("ultra", "strict"),
        ("full", "enforce"),
        ("enforce", "enforce"),
        ("off", "off"),
    ] {
        assert_eq!(f.level(&[word]).status.code(), Some(0), "{word}");
        assert_eq!(stdout(&f.level(&[])), format!("level: {name} (project)\n"));
    }
    fs::write(f.state_root().join("level"), "ultra\n").unwrap();
    assert_eq!(stdout(&f.level(&[])), "level: strict (project)\n");
}

/// M3: `supervisor level status` shows the mode like the bare command, so the plugin's
/// `/supervisor status` can pass its argument straight through.
#[test]
fn level_status_shows_the_mode() {
    let f = Fixture::new("level-status");
    let out = f.level(&["status"]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(0), "level: enforce (default)\n")
    );
}

/// M7 with K6: in strict, a bug fix (a mandatory baseline criterion) completes only when the
/// baseline failed on the starting candidate and the fix passes now; in enforce the baseline
/// stays an ordinary check that must pass.
#[test]
fn strict_bug_fix_needs_the_failure_reproduced_before_the_fix() {
    for (level, baseline_exit, completes) in [
        ("strict", 1, true),
        ("strict", 0, false),
        ("enforce", 1, false),
    ] {
        let f = Fixture::new(&format!("bugfix-{level}-{baseline_exit}"));
        let contract = f.root.join("contract.json");
        fs::write(&contract, json!({"id":"bug-fix","goal":"fix the reported bug","criteria":[
            {"id":"C1","check":"baseline_reproduces_reported_failure","mandatory":true,"command":"cargo test repro"},
            {"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],
            "scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let out = f.supervisor(&[
            "task",
            "start",
            "--contract",
            contract.to_str().unwrap(),
            "--project",
            f.project.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "{out:?}");
        f.set(&[level]);
        let run = |id: &str, command: &str, exit: i32| {
            let _ = f.tool(id, "Bash", json!({ "command": command }));
            let _ = if exit == 0 {
                f.hook(json!({"hook_event_name":"PostToolUse","tool_use_id":id,"tool_name":"Bash",
                    "tool_input":{"command":command},"tool_response":{"stdout":"ok","stderr":"","interrupted":false}}))
            } else {
                f.hook(json!({"hook_event_name":"PostToolUseFailure","tool_use_id":id,"tool_name":"Bash",
                    "tool_input":{"command":command},"error":format!("Exit code {exit}")}))
            };
        };
        run("repro", "cargo test repro", baseline_exit);
        fs::create_dir_all(f.project.join("src")).unwrap();
        fs::write(f.project.join("src/lib.rs"), "// the fix\n").unwrap();
        run("fixed", "cargo test", 0);
        let stop = f.stop();
        let case = format!("{level} baseline exit {baseline_exit}: {stop}");
        assert_eq!(stop.get("decision").is_none(), completes, "{case}");
        if level == "strict" && !completes {
            assert!(
                stop["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("starting candidate"),
                "{case}"
            );
        }
    }
}

/// M5: in strict, typed forbidden effects are refused before they happen; untyped ones and
/// every effect at enforce are only recorded.
#[test]
fn strict_refuses_typed_forbidden_effects() {
    let f = Fixture::new("forbidden");
    let contract = f.root.join("contract.json");
    fs::write(&contract, json!({"id":"forbidden","goal":"respect forbidden effects","criteria":[
        {"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],
        "scope":["src/","docs/"],
        "forbidden_effects":["deny_path:docs/private/","deny_git:push","deny_argv_prefix:npm publish","no network"],
        "max_repair_rounds":2}).to_string()).unwrap();
    let out = f.supervisor(&[
        "task",
        "start",
        "--contract",
        contract.to_str().unwrap(),
        "--project",
        f.project.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{out:?}");
    let attempts = [
        (
            "w1",
            "Write",
            json!({"file_path":"docs/private/notes.md","content":"x"}),
            true,
        ),
        (
            "w2",
            "Write",
            json!({"file_path":"docs/public.md","content":"x"}),
            false,
        ),
        (
            "b1",
            "Bash",
            json!({"command":"git -C . push origin main"}),
            true,
        ),
        (
            "b2",
            "Bash",
            json!({"command":"git log --grep push"}),
            false,
        ),
        (
            "b3",
            "Bash",
            json!({"command":"cd npm && npm publish --tag next"}),
            true,
        ),
        ("b4", "Bash", json!({"command":"npm pack"}), false),
    ];
    for level in ["strict", "enforce"] {
        f.set(&[level]);
        for (id, tool, input, forbidden) in &attempts {
            let decision = f.tool(&format!("{level}-{id}"), tool, input.clone());
            let refused = denied(&decision) && decision.to_string().contains("forbids");
            assert_eq!(
                refused,
                *forbidden && level == "strict",
                "{level} {id}: {decision}"
            );
        }
    }
}

/// M6: in strict, a criterion's `min_tests` is met only by an observed run whose test summary
/// shows at least that many passing tests; enforce ignores it.
#[test]
fn strict_needs_the_minimum_number_of_passing_tests() {
    for (level, first_completes) in [("strict", false), ("enforce", true)] {
        let f = Fixture::new(&format!("min-tests-{level}"));
        let contract = f.root.join("contract.json");
        fs::write(&contract, json!({"id":"min-tests","goal":"enough tests","criteria":[
            {"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test","min_tests":5}],
            "scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let out = f.supervisor(&[
            "task",
            "start",
            "--contract",
            contract.to_str().unwrap(),
            "--project",
            f.project.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "{out:?}");
        f.set(&[level]);
        let run = |id: &str, passed: u32| {
            let _ = f.tool(id, "Bash", json!({"command":"cargo test"}));
            let _ = f.hook(json!({"hook_event_name":"PostToolUse","tool_use_id":id,"tool_name":"Bash",
                "tool_input":{"command":"cargo test"},"tool_response":{"interrupted":false,"stderr":"",
                "stdout":format!("running {passed} tests\ntest result: ok. {passed} passed; 0 failed; 0 ignored\n")}}));
            f.stop()
        };
        let few = run("few", 3);
        assert_eq!(
            few.get("decision").is_none(),
            first_completes,
            "{level}: {few}"
        );
        if level == "strict" {
            let reason = few["reason"].as_str().unwrap_or_default();
            assert!(
                reason.contains("at least 5") && reason.contains('3'),
                "{few}"
            );
            let enough = run("enough", 7);
            assert!(enough.get("decision").is_none(), "{enough}");
        }
    }
}
