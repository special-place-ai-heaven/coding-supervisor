//! `supervisor verdict`: recomputed from receipts, the hook run journal and the live git HEAD.
//! CSUP-VERDICT-01 / HOUSE CSUP-ASSESS-01 finding H1.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use coding_supervisor::{CandidateId, Checkpoint, CriterionId, TaskState, VerificationReceipt};
use serde_json::{Value, json};
use supervisor_claude_code::candidate::{CandidateProvider, GitCandidate};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_checked};
use supervisor_claude_code::state::{Gate, read_contract, save_gate};

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
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-verdict-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        Self {
            project: root.join("project"),
            state: root.join("state"),
            root,
        }
    }
    /// A git repository with one commit, so HEAD is known.
    fn git(name: &str) -> Self {
        let f = Self::new(name);
        f.run_git(&["init", "-q"]);
        fs::write(f.project.join("README"), "one\n").unwrap();
        f.commit("one");
        f
    }
    fn run_git(&self, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.project)
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
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn commit(&self, message: &str) {
        self.run_git(&["add", "-A"]);
        self.run_git(&["commit", "-q", "-m", message]);
    }
    fn head(&self) -> CandidateId {
        GitCandidate {
            cwd: self.project.clone(),
        }
        .observe()
        .unwrap()
        .id
    }
    fn verdict(&self, extra: &[&str]) -> Output {
        Command::new(BIN)
            .arg("verdict")
            .args(extra)
            .arg("--project")
            .arg(&self.project)
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .output()
            .unwrap()
    }
    fn verdict_json(&self, extra: &[&str]) -> (i32, Value) {
        let mut args = vec!["--json"];
        args.extend_from_slice(extra);
        let out = self.verdict(&args);
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "verdict JSON ({e}): {} / {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap(), value)
    }
    /// Start a task with one mandatory criterion C2 (`fixture-check`) and store `receipts`, each
    /// with `exit_code`; `runs` are the hook-observed completions backing some of them.
    fn task_with(
        &self,
        receipts: &[(&str, CandidateId)],
        runs: &[(&str, CandidateId)],
        exit_code: i32,
    ) -> String {
        let contract = json!({
            "id":"verdict-fixture","goal":"exercise verdict",
            "criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"fixture-check"}],
            "scope":[],"forbidden_effects":[],"max_repair_rounds":1
        });
        let state = self.state.to_string_lossy().into_owned();
        let (root, _) = resolve_state_dir_checked(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| state.clone())
        })
        .unwrap();
        lifecycle::start(&root, &contract, false, None, None).unwrap();
        let task = lifecycle::active_task(&root).unwrap().unwrap();
        let (_, digest) = read_contract(&task.dir).unwrap();
        let cwd = canonical_project(&self.project);
        let mut gate = Gate::new(
            "verdict-fixture".to_owned(),
            Checkpoint {
                // Deliberately "completed": the verdict must not read the stored gate state.
                state: TaskState::Completed,
                candidate: receipts.last().map(|(_, c)| c.clone()),
                open_findings: Vec::new(),
                consumed_repair_rounds: 0,
                next_action: "none".to_owned(),
            },
        );
        gate.receipts = receipts
            .iter()
            .enumerate()
            .map(|(i, (output_ref, candidate))| VerificationReceipt {
                criterion: CriterionId("C2".to_owned()),
                candidate: candidate.clone(),
                command: "fixture-check".to_owned(),
                cwd: cwd.clone(),
                exit_code: Some(exit_code),
                output_ref: output_ref.replace("{task}", &task.task_id),
                claim_supported: true,
                started_at: 10 * i as u64 + 1,
                finished_at: 10 * i as u64 + 2,
            })
            .collect();
        save_gate(&task.dir, &gate, &digest).unwrap();
        let journal: Vec<String> = runs
            .iter()
            .map(|(dispatch, candidate)| {
                let i = receipts
                    .iter()
                    .position(|(r, _)| r.ends_with(&format!(":{dispatch}")))
                    .unwrap() as u64;
                json!({
                    "kind":"completed","completion_trust":"observed",
                    "execution_project":cwd,"execution_root":cwd,
                    "execution_profile":{"name":"local-default-v1","approved":true,"cargo_target_sha256":null},
                    "cwd":cwd,"task_id":task.task_id,"agent_id":"main","identity_schema":3,
                    "tool":"mcp__terminal_commander__run_and_watch",
                    "identity":{"argv":["fixture-check"]},"command":"fixture-check",
                    "candidate":{"base_commit":candidate.base_commit,"dirty_digest":candidate.dirty_digest},
                    "exit_code":exit_code,"dispatch_id":dispatch,"t":10 * i + 1,"finished":10 * i + 2
                })
                .to_string()
            })
            .collect();
        if !journal.is_empty() {
            fs::write(task.dir.join("runs.jsonl"), journal.join("\n") + "\n").unwrap();
        }
        task.task_id
    }
}

fn criterion<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()
}

/// Minimal MCP stdio client: one JSON-RPC line out, one line back for requests.
struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
}
impl Mcp {
    fn start(f: &Fixture) -> Self {
        let mut child = Command::new(BIN)
            .arg("mcp")
            .arg("--project")
            .arg(&f.project)
            .env("SUPERVISOR_STATE_DIR", &f.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Self {
            child,
            out,
            next: 1,
        };
        mcp.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"verdict-test","version":"0"}}));
        mcp.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        mcp
    }
    fn send(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(&json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}));
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    /// Tool reply as JSON; panics on a refusal.
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.request("tools/call", json!({"name":name,"arguments":arguments}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["isError"], false, "{name} refused: {text}");
        serde_json::from_str(text).unwrap()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// HOUSE's csup-logs/mcp-asserted-receipt.log, replayed: an MCP caller asserts exit 0 for the
/// mandatory check `false` (it never ran) and drives the gate to `completed`.
fn replay_fabricated_receipt(f: &Fixture) -> String {
    let mut mcp = Mcp::start(f);
    let started = mcp.tool(
        "task_start",
        json!({"id":"probe","goal":"probe that a failing check can be asserted green",
               "criteria":[{"id":"C1","check":"always_fails","command":"false","mandatory":true}],
               "forbidden_effects":[],"scope":["src/"],"max_repair_rounds":1}),
    );
    let task = started["task_id"].as_str().unwrap().to_owned();
    mcp.tool("candidate_observe", json!({"task_id":task}));
    let project = f.project.to_string_lossy().into_owned();
    mcp.tool(
        "receipt_record",
        json!({"task_id":task,"criterion":"C1","command":"false","cwd":project,
               "exit_code":0,"output_ref":"never-ran","claim_supported":true}),
    );
    mcp.tool("accept_try", json!({"task_id":task}));
    let done = mcp.tool("task_complete", json!({"task_id":task}));
    // The hole itself: the gate believes the caller.
    assert_eq!(done["checkpoint"]["state"], "completed");
    task
}

#[test]
fn fabricated_mcp_receipt_is_asserted_not_observed_and_fails_strict_verdict() {
    let f = Fixture::git("fabricated");
    let task = replay_fabricated_receipt(&f);

    let out = f.verdict(&["--require", "observed"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(3), "{text}");
    assert!(
        text.lines().any(|l| l == "C1: asserted, not observed"),
        "{text}"
    );

    let (code, report) = f.verdict_json(&["--require", "observed", "--task", &task]);
    assert_eq!(code, 3);
    assert_eq!(report["verdict"], "fail");
    assert_eq!(report["exit_code"], 3);
    assert_eq!(report["task"]["task_id"], task.as_str());
    assert_eq!(report["head"]["sha"], f.head().base_commit.as_str());
    let c1 = criterion(&report, "C1");
    assert_eq!(c1["state"], "asserted_not_observed");
    assert_eq!(c1["provenance"], "mcp");
    assert_eq!(c1["mandatory"], true);
    assert_eq!(c1["receipt"]["head_sha"], f.head().base_commit.as_str());
    assert_eq!(report["unmet"], json!(["C1"]));
    assert_eq!(report["flags"], json!(["C1: asserted, not observed"]));
}

#[test]
fn lenient_require_accepts_an_asserted_receipt_but_still_flags_it() {
    let f = Fixture::git("lenient");
    replay_fabricated_receipt(&f);
    let (code, report) = f.verdict_json(&["--require", "asserted"]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["verdict"], "pass");
    assert_eq!(criterion(&report, "C1")["state"], "asserted_not_observed");
    assert_eq!(report["flags"], json!(["C1: asserted, not observed"]));
}

#[test]
fn hook_observed_receipt_at_current_head_passes() {
    let f = Fixture::git("observed");
    let head = f.head();
    let task = f.task_with(
        &[("observed:{task}:live", head.clone())],
        &[("live", head.clone())],
        0,
    );
    let (code, report) = f.verdict_json(&["--require", "observed", "--task", &task]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["verdict"], "pass");
    let c2 = criterion(&report, "C2");
    assert_eq!(c2["state"], "observed");
    assert_eq!(c2["provenance"], "hook");
    assert_eq!(c2["receipt"]["head_sha"], head.base_commit.as_str());
    let out = f.verdict(&[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("C2: observed (hook)"));
}

#[test]
fn failing_observed_receipt_fails() {
    let f = Fixture::git("failing");
    let head = f.head();
    f.task_with(
        &[("observed:{task}:live", head.clone())],
        &[("live", head)],
        1,
    );
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{report}");
    let c2 = criterion(&report, "C2");
    assert_eq!(c2["state"], "failed");
    assert_eq!(c2["reason"], "exit_nonzero");
    let text = String::from_utf8_lossy(&f.verdict(&[]).stdout).into_owned();
    assert!(text.contains("C2: failed (exit 1)"), "{text}");
}

#[test]
fn receipt_observed_at_an_older_head_is_stale() {
    let f = Fixture::git("older-head");
    let old = f.head();
    f.task_with(
        &[("observed:{task}:live", old.clone())],
        &[("live", old.clone())],
        0,
    );
    fs::write(f.project.join("README"), "two\n").unwrap();
    f.commit("two");
    let new = f.head();
    assert_ne!(old.base_commit, new.base_commit);
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{report}");
    assert_eq!(report["head"]["sha"], new.base_commit.as_str());
    let c2 = criterion(&report, "C2");
    assert_eq!(c2["state"], "stale");
    assert_eq!(c2["reason"], "receipt_at_other_head");
    assert_eq!(c2["receipt"]["head_sha"], old.base_commit.as_str());
    let text = String::from_utf8_lossy(&f.verdict(&[]).stdout).into_owned();
    assert!(
        text.contains(&format!(
            "C2: stale (receipt at other head {})",
            &old.base_commit[..12]
        )),
        "{text}"
    );
}

#[test]
fn receipt_from_before_a_worktree_edit_is_stale() {
    let f = Fixture::git("dirty");
    let head = f.head();
    f.task_with(
        &[("observed:{task}:live", head.clone())],
        &[("live", head)],
        0,
    );
    fs::write(f.project.join("README"), "edited\n").unwrap();
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3);
    assert_eq!(criterion(&report, "C2")["state"], "stale");
    assert_eq!(criterion(&report, "C2")["reason"], "worktree_changed");
}

#[test]
fn legacy_receipt_without_provenance_fails_closed() {
    let f = Fixture::git("legacy");
    let head = f.head();
    // A pre-provenance receipt: no recognised prefix, nothing in the run journal.
    f.task_with(&[("cargo-test-output.log", head.clone())], &[], 0);
    let (code, report) = f.verdict_json(&["--require", "asserted"]);
    assert_eq!(code, 3, "{report}");
    let c2 = criterion(&report, "C2");
    assert_eq!(c2["state"], "unverified");
    assert_eq!(c2["provenance"], "unverified");
    assert_eq!(f.verdict_json(&[]).0, 3);
}

#[test]
fn observed_prefix_without_a_run_record_is_not_observed() {
    let f = Fixture::git("forged-prefix");
    let head = f.head();
    f.task_with(&[("observed:{task}:forged", head)], &[], 0);
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3);
    assert_eq!(criterion(&report, "C2")["state"], "unverified");
}

#[test]
fn missing_receipt_fails() {
    let f = Fixture::git("missing");
    f.task_with(&[], &[], 0);
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3);
    let c2 = criterion(&report, "C2");
    assert_eq!(c2["state"], "missing");
    assert_eq!(c2["provenance"], Value::Null);
    assert_eq!(c2["receipt"], Value::Null);
}

#[test]
fn unknown_head_is_explicit_and_fails_closed() {
    let f = Fixture::new("no-git");
    let anywhere = CandidateId {
        base_commit: "0".repeat(40),
        dirty_digest: "fixture".to_owned(),
    };
    f.task_with(
        &[("observed:{task}:live", anywhere.clone())],
        &[("live", anywhere)],
        0,
    );
    for require in ["observed", "asserted"] {
        let (code, report) = f.verdict_json(&["--require", require]);
        assert_eq!(code, 3, "{report}");
        assert_eq!(report["head"]["sha"], Value::Null);
        assert!(
            report["head"]["reason"]
                .as_str()
                .is_some_and(|r| !r.is_empty())
        );
        assert_eq!(report["reason"], "head_unknown");
        assert_eq!(criterion(&report, "C2")["reason"], "head_unknown");
    }
}

#[test]
fn no_active_task_fails_and_wrong_task_is_an_error() {
    let f = Fixture::git("inactive");
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3);
    assert_eq!(report["task"], Value::Null);
    assert_eq!(report["reason"], "no_active_task");
    f.task_with(&[], &[], 0);
    let out = f.verdict(&["--task", "task-not-this-one"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn bad_require_is_a_usage_error() {
    let f = Fixture::git("usage");
    assert_eq!(f.verdict(&["--require", "vibes"]).status.code(), Some(2));
    assert_eq!(f.verdict(&["--strict"]).status.code(), Some(2));
}
