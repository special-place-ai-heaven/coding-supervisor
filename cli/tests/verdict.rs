//! `supervisor verdict`: recomputed from receipts, the hook run journal and the live git HEAD.
//! Tests are named after contract r3 §11 (T01-T34); KENT's K01-K15 are folded in as noted, and
//! `kx` tests are KENT's additions. `observed` proves consistency with a hook-format journal
//! row, not authenticity (r3 §9): these tests show inconsistent evidence is rejected.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coding_supervisor::{CandidateId, Checkpoint, CriterionId, TaskState, VerificationReceipt};
use serde_json::{Value, json};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_checked};
use supervisor_claude_code::sha256;
use supervisor_claude_code::state::{Gate, read_contract, save_gate};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
/// The dirty digest of a clean worktree: sha256 of the empty listing.
const CLEAN: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

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
        fs::write(root.join("gitconfig"), "").unwrap();
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
    /// A child process that sees no user, system or inherited repository git configuration or
    /// global `level` file, and discovers no repository above the fixture root.
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("HOME", &self.root)
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
    fn git_output(&self, args: &[&str]) -> Output {
        let out = self
            .command("git")
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
        out
    }
    fn run_git(&self, args: &[&str]) {
        self.git_output(args);
    }
    fn commit(&self, message: &str) {
        self.run_git(&["add", "-A"]);
        self.run_git(&["commit", "-q", "-m", message]);
    }
    /// HEAD per git itself, not the code under test.
    fn rev_parse(&self) -> String {
        String::from_utf8(self.git_output(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }
    /// The candidate of the clean worktree at HEAD.
    fn head(&self) -> CandidateId {
        CandidateId {
            base_commit: self.rev_parse(),
            dirty_digest: CLEAN.to_owned(),
        }
    }
    fn verdict_command(&self, extra: &[&str]) -> Command {
        let mut cmd = self.command(BIN);
        cmd.arg("verdict")
            .args(extra)
            .arg("--project")
            .arg(&self.project)
            .env("SUPERVISOR_STATE_DIR", &self.state);
        cmd
    }
    fn verdict(&self, extra: &[&str]) -> Output {
        self.verdict_command(extra).output().unwrap()
    }
    fn verdict_json(&self, extra: &[&str]) -> (i32, Value) {
        let mut args = vec!["--json"];
        args.extend_from_slice(extra);
        parse(&self.verdict(&args))
    }
    fn state_root(&self) -> PathBuf {
        let state = self.state.to_string_lossy().into_owned();
        resolve_state_dir_checked(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| state.clone())
        })
        .unwrap()
        .0
    }
    /// [`Fixture::task_stored`] with the stored gate deliberately `completed`: the verdict must
    /// not read it.
    fn task(
        &self,
        criteria: &[(&str, bool)],
        receipts: &[Rc],
        runs: &[Run<'_>],
    ) -> (String, PathBuf) {
        self.task_stored(TaskState::Completed, criteria, receipts, runs)
    }
    /// Start a task with `criteria` (id, mandatory) in contract order and store `receipts`;
    /// receipt i ran at `T0 + 10i + 1` and finished at `T0 + 10i + 2`. Each run is the hook
    /// completion of receipt `index` under `dispatch`, matching it exactly before `mutate`.
    fn task_stored(
        &self,
        stored: TaskState,
        criteria: &[(&str, bool)],
        receipts: &[Rc],
        runs: &[Run<'_>],
    ) -> (String, PathBuf) {
        self.task_stored_with(stored, criteria, receipts, runs, &json!({}))
    }
    /// [`Self::task_stored`] with `extra` contract fields, e.g. `limits`.
    fn task_stored_with(
        &self,
        stored: TaskState,
        criteria: &[(&str, bool)],
        receipts: &[Rc],
        runs: &[Run<'_>],
        extra: &Value,
    ) -> (String, PathBuf) {
        let criteria: Vec<Value> = criteria
            .iter()
            .map(|(id, mandatory)| {
                json!({"id":id,"check":"candidate_passes_reported_test","mandatory":mandatory,"command":"fixture-check"})
            })
            .collect();
        let mut contract = json!({"id":"verdict-fixture","goal":"exercise verdict","criteria":criteria,
            "scope":[],"forbidden_effects":[],"max_repair_rounds":1});
        for (key, value) in extra.as_object().into_iter().flatten() {
            contract[key] = value.clone();
        }
        let root = self.state_root();
        lifecycle::start_as(
            &root,
            &supervisor_claude_code::state::Writer::task_start(None),
            &contract,
            false,
            None,
            None,
        )
        .unwrap();
        let task = lifecycle::active_task(&root).unwrap().unwrap();
        let (_, digest) = read_contract(&task.dir).unwrap();
        let cwd = canonical_project(&self.project);
        let mut gate = Gate::new(
            "verdict-fixture".to_owned(),
            Checkpoint {
                state: stored,
                candidate: receipts.last().map(|r| r.candidate.clone()),
                open_findings: Vec::new(),
                consumed_repair_rounds: 0,
                next_action: "none".to_owned(),
            },
        );
        gate.receipts = receipts
            .iter()
            .enumerate()
            .map(|(i, r)| VerificationReceipt {
                criterion: CriterionId(r.criterion.to_owned()),
                candidate: r.candidate.clone(),
                command: "fixture-check".to_owned(),
                cwd: cwd.clone(),
                exit_code: r.exit_code,
                output_ref: r.output_ref.replace("{task}", &task.task_id),
                claim_supported: r.claim_supported,
                started_at: T0 + 10 * i as u64 + 1,
                finished_at: T0 + 10 * i as u64 + 2,
            })
            .collect();
        save_gate(&task.dir, &gate, &digest).unwrap();
        let journal: Vec<String> = runs
            .iter()
            .map(|&(dispatch, i, mutate)| {
                let r = &receipts[i];
                let trust = if r.output_ref.starts_with("tc-reconstructed:") {
                    "tc-reconstructed"
                } else {
                    "observed"
                };
                let mut run = json!({
                    "kind":"completed","completion_trust":trust,
                    "execution_project":cwd,"execution_root":cwd,
                    "execution_profile":{"name":"local-default-v1","approved":true,"cargo_target_sha256":null},
                    "cwd":cwd,"task_id":task.task_id,"agent_id":"main","identity_schema":3,
                    "tool":"mcp__terminal_commander__run_and_watch",
                    "identity":{"argv":["fixture-check"]},"command":"fixture-check",
                    "candidate":{"base_commit":r.candidate.base_commit,"dirty_digest":r.candidate.dirty_digest},
                    "exit_code":r.exit_code,"dispatch_id":dispatch,
                    "t":T0 + 10 * i as u64 + 1,"finished":T0 + 10 * i as u64 + 2
                });
                mutate(&mut run);
                run.to_string()
            })
            .collect();
        if !journal.is_empty() {
            fs::write(task.dir.join("runs.jsonl"), journal.join("\n") + "\n").unwrap();
        }
        (task.task_id, task.dir)
    }
    /// One mandatory C2 with one hook-observed passing receipt at the current HEAD.
    fn observed_pass(&self) -> (String, PathBuf, CandidateId) {
        let head = self.head();
        let (task, dir) = self.task(
            &[("C2", true)],
            &[rc("C2", "observed:{task}:a", &head, 0)],
            &[("a", 0, &same)],
        );
        (task, dir, head)
    }
}

/// 2000-02-29T00:00:00Z: receipt times start on a leap day.
const T0: u64 = 951_782_400;

struct Rc {
    criterion: &'static str,
    output_ref: String,
    candidate: CandidateId,
    exit_code: Option<i32>,
    claim_supported: bool,
}
fn rc(criterion: &'static str, output_ref: &str, candidate: &CandidateId, exit_code: i32) -> Rc {
    Rc {
        criterion,
        output_ref: output_ref.to_owned(),
        candidate: candidate.clone(),
        exit_code: Some(exit_code),
        claim_supported: true,
    }
}
/// Changes one field of a run record.
type Mutate = dyn Fn(&mut Value);
type Run<'a> = (&'a str, usize, &'a Mutate);
fn same(_: &mut Value) {}

fn parse(out: &Output) -> (i32, Value) {
    let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "verdict JSON ({e}): {} / {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap(), value)
}
fn criterion<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()
}
fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("not an object: {v}"))
        .keys()
        .cloned()
        .collect()
}
fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|k| (*k).to_owned()).collect()
}
fn is_hex(v: &Value, len: usize) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}
fn sha256_file(path: &Path) -> String {
    sha256::hex(&fs::read(path).unwrap())
}
/// Exit 1 with nothing on stdout (r3 §7: could not answer).
fn assert_exit1_empty(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "{what}: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{what}: stdout must be empty, got {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Minimal MCP stdio client: one JSON-RPC line out, one line back for requests.
struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
}
impl Mcp {
    fn start(f: &Fixture) -> Self {
        let mut child = f
            .command(BIN)
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
        mcp.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"verdict-test","version":"0"}}),
        );
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
fn replay_fabricated_receipt(f: &Fixture) {
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
}

/// One criterion in every state, contract order not sorted by id:
/// C3 observed (hook), C1 asserted, C2 failed (observed exit 1), C6 unverified, C5 stale at
/// another head, C4 missing, C7 optional asserted, C0 observed (tc).
fn every_state(f: &Fixture) -> String {
    let head = f.head();
    let other = CandidateId {
        base_commit: "f".repeat(40),
        dirty_digest: CLEAN.to_owned(),
    };
    let criteria = [
        ("C3", true),
        ("C1", true),
        ("C2", true),
        ("C6", true),
        ("C5", true),
        ("C4", true),
        ("C7", false),
        ("C0", true),
    ];
    let receipts = [
        rc("C3", "observed:{task}:a", &head, 0),
        rc("C1", "mcp-asserted:c1", &head, 0),
        rc("C2", "observed:{task}:b", &head, 1),
        rc("C6", "legacy.log", &head, 0),
        rc("C5", "observed:{task}:c", &other, 0),
        rc("C7", "mcp-asserted:c7", &head, 0),
        rc("C0", "tc-reconstructed:{task}:d", &head, 0),
    ];
    let runs: [Run<'_>; 4] = [
        ("a", 0, &same),
        ("b", 2, &same),
        ("c", 4, &same),
        ("d", 6, &same),
    ];
    f.task(&criteria, &receipts, &runs).0
}

/// T01 [K09] + T11 + T15 + T16 + T17: every key present on every path, formats per §5.
#[test]
fn t01_verdict_json_every_key_present_on_every_path() {
    let top = set(&[
        "schema_version",
        "verdict",
        "reason",
        "exit_code",
        "require",
        "effective_level",
        "level_source",
        "task",
        "head",
        "criteria",
        "unmet",
        "flags",
    ]);
    let head_keys = set(&["sha", "dirty_digest", "reason"]);
    let crit = set(&[
        "id",
        "mandatory",
        "state",
        "reason",
        "provenance",
        "receipt",
        "satisfies",
    ]);
    let receipt = set(&[
        "head_sha",
        "dirty_digest",
        "command",
        "exit_code",
        "recorded_at",
        "output_ref",
    ]);
    let f = Fixture::git("t01");
    let (task, dir, head) = f.observed_pass();
    let (code, r) = f.verdict_json(&[]);
    assert_eq!(code, 0, "{r}");
    assert_eq!(keys(&r), top);
    assert_eq!(r["schema_version"], json!(1));
    assert_eq!(r["reason"], Value::Null);
    assert_eq!(
        (&r["effective_level"], &r["level_source"]),
        (&json!("enforce"), &json!("default"))
    );
    assert_eq!(
        keys(&r["task"]),
        set(&["task_id", "contract_id", "contract_digest"])
    );
    assert_eq!(r["task"]["task_id"], task.as_str());
    assert_eq!(r["task"]["contract_id"], "verdict-fixture");
    assert_eq!(
        r["task"]["contract_digest"],
        sha256_file(&dir.join("contract.json")).as_str(),
        "T16: digest of the contract.json bytes"
    );
    assert_eq!(keys(&r["head"]), head_keys);
    assert_eq!(
        r["head"]["sha"],
        f.rev_parse().as_str(),
        "T11: oracle is git"
    );
    assert!(is_hex(&r["head"]["sha"], 40) && is_hex(&r["head"]["dirty_digest"], 64));
    assert_eq!(r["head"]["reason"], Value::Null);
    let c = criterion(&r, "C2");
    assert_eq!(keys(c), crit);
    assert_eq!(keys(&c["receipt"]), receipt);
    assert_eq!(c["receipt"]["head_sha"], head.base_commit.as_str());
    assert_eq!(c["receipt"]["dirty_digest"], head.dirty_digest.as_str());
    assert_eq!(
        c["receipt"]["recorded_at"], "2000-02-29T00:00:02.000Z",
        "T15: finished_at as RFC 3339 UTC milliseconds Z"
    );
    assert_eq!(
        c["receipt"]["output_ref"],
        format!("observed:{task}:a"),
        "T17"
    );
    // Fail, no-task and head-unknown paths: same key sets, null rather than absent.
    let g = Fixture::git("t01-missing");
    g.task(&[("C2", true)], &[], &[]);
    let r = g.verdict_json(&[]).1;
    assert_eq!(keys(&r), top);
    assert_eq!(keys(criterion(&r, "C2")), crit);
    assert_eq!(criterion(&r, "C2")["receipt"], Value::Null);
    let h = Fixture::git("t01-no-task");
    let r = h.verdict_json(&[]).1;
    assert_eq!(keys(&r), top);
    assert_eq!(r["task"], Value::Null);
    assert_eq!(r["criteria"], json!([]));
    let u = Fixture::new("t01-no-repo");
    u.task(
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &head, 0)],
        &[("a", 0, &same)],
    );
    let r = u.verdict_json(&[]).1;
    assert_eq!(keys(&r), top);
    assert_eq!(keys(&r["head"]), head_keys);
    assert_eq!(keys(&criterion(&r, "C2")["receipt"]), receipt);
}

/// T03: every enum the JSON carries is from its closed snake_case set, in the §4 pairings.
#[test]
fn t03_verdict_enum_values_are_closed_snake_case_set() {
    let f = Fixture::git("t03");
    every_state(&f);
    let unknown = Fixture::new("t03-no-repo");
    unknown.task(
        &[("C2", true)],
        &[rc("C2", "legacy.log", &f.head(), 0)],
        &[],
    );
    let none = Fixture::git("t03-no-task");
    let reports = [
        f.verdict_json(&[]).1,
        f.verdict_json(&["--require", "asserted"]).1,
        unknown.verdict_json(&[]).1,
        none.verdict_json(&[]).1,
    ];
    let one_of = |v: &Value, allowed: &[Option<&str>]| {
        assert!(allowed.contains(&v.as_str()), "{v} not in {allowed:?}");
    };
    let mut states = BTreeSet::new();
    for r in &reports {
        one_of(&r["verdict"], &[Some("pass"), Some("fail")]);
        one_of(
            &r["reason"],
            &[
                None,
                Some("no_active_task"),
                Some("head_unknown"),
                Some("no_mandatory_criteria"),
                Some("mandatory_unmet"),
            ],
        );
        one_of(&r["require"], &[Some("observed"), Some("asserted")]);
        one_of(
            &r["head"]["reason"],
            &[
                None,
                Some("git_unavailable"),
                Some("git_failed"),
                Some("worktree_unreadable"),
                Some("head_unparseable"),
            ],
        );
        for flag in r["flags"].as_array().unwrap() {
            one_of(&flag["code"], &[Some("asserted_not_observed")]);
        }
        for c in r["criteria"].as_array().unwrap() {
            let (state, reason, provenance) = (&c["state"], &c["reason"], &c["provenance"]);
            let (reasons, provenances): (&[Option<&str>], &[Option<&str>]) =
                match state.as_str().unwrap() {
                    "observed" => (&[None], &[Some("hook"), Some("tc")]),
                    "asserted_not_observed" => (&[None], &[Some("mcp")]),
                    "failed" => (
                        &[
                            Some("exit_nonzero"),
                            Some("no_exit_code"),
                            Some("claim_unsupported"),
                        ],
                        &[Some("hook"), Some("tc"), Some("mcp")],
                    ),
                    "unverified" => (&[Some("no_observation_record")], &[Some("unverified")]),
                    "stale" => (
                        &[
                            Some("receipt_at_other_head"),
                            Some("worktree_changed"),
                            Some("head_unknown"),
                        ],
                        &[Some("hook"), Some("tc"), Some("mcp"), Some("unverified")],
                    ),
                    "missing" => (&[None], &[None]),
                    other => panic!("state {other} is not in the closed set"),
                };
            one_of(reason, reasons);
            one_of(provenance, provenances);
            states.insert(state.as_str().unwrap().to_owned());
        }
    }
    assert_eq!(
        states,
        set(&[
            "observed",
            "asserted_not_observed",
            "failed",
            "unverified",
            "stale",
            "missing"
        ]),
        "control: every state was exercised"
    );
}

fn decided(
    name: &str,
    receipts: &dyn Fn(&CandidateId) -> Vec<Rc>,
    runs: &[(&str, usize)],
    move_head: bool,
) -> String {
    let f = Fixture::git(name);
    let runs: Vec<Run<'_>> = runs.iter().map(|&(d, i)| (d, i, &same as _)).collect();
    f.task(&[("C2", true)], &receipts(&f.head()), &runs);
    if move_head {
        fs::write(f.project.join("README"), "two\n").unwrap();
        f.commit("two");
    }
    let (code, r) = f.verdict_json(&[]);
    assert!(
        code == 0 || code == 3,
        "{name}: an answer, not an error: {r}"
    );
    criterion(&r, "C2")["state"].as_str().unwrap().to_owned()
}

/// T04 [K06]: observed > asserted > unverified > stale; latest within a tier. Kills M2, M3.
#[test]
fn t04_verdict_one_state_per_criterion_priority_and_latest_wins() {
    let observed_then_asserted = |h: &CandidateId| {
        vec![
            rc("C2", "observed:{task}:a", h, 0),
            rc("C2", "mcp-asserted:x", h, 1),
        ]
    };
    assert_eq!(
        decided("t04a", &observed_then_asserted, &[("a", 0)], false),
        "observed"
    );
    let pass_then_fail = |h: &CandidateId| {
        vec![
            rc("C2", "observed:{task}:a", h, 0),
            rc("C2", "observed:{task}:b", h, 1),
        ]
    };
    assert_eq!(
        decided("t04b", &pass_then_fail, &[("a", 0), ("b", 1)], false),
        "failed"
    );
    let fail_then_pass = |h: &CandidateId| {
        vec![
            rc("C2", "observed:{task}:a", h, 1),
            rc("C2", "observed:{task}:b", h, 0),
        ]
    };
    assert_eq!(
        decided("t04c", &fail_then_pass, &[("a", 0), ("b", 1)], false),
        "observed"
    );
    let asserted_then_legacy = |h: &CandidateId| {
        vec![
            rc("C2", "mcp-asserted:x", h, 0),
            rc("C2", "legacy.log", h, 0),
        ]
    };
    assert_eq!(
        decided("t04d", &asserted_then_legacy, &[], false),
        "asserted_not_observed"
    );
    let legacy = |h: &CandidateId| vec![rc("C2", "legacy.log", h, 0)];
    assert_eq!(decided("t04e", &legacy, &[], false), "unverified");
    let observed = |h: &CandidateId| vec![rc("C2", "observed:{task}:a", h, 0)];
    assert_eq!(decided("t04f", &observed, &[("a", 0)], true), "stale");
}

/// T05: a current observed failure beats a later passing assertion (r3 §4 note).
#[test]
fn t05_verdict_failed_observed_beats_passing_asserted() {
    let receipts = |h: &CandidateId| {
        vec![
            rc("C2", "observed:{task}:a", h, 1),
            rc("C2", "mcp-asserted:x", h, 0),
        ]
    };
    assert_eq!(decided("t05", &receipts, &[("a", 0)], false), "failed");
}

/// T07: one structured flag per asserted criterion, mandatory or not, under either policy.
#[test]
fn t07_verdict_flags_are_structured_and_mark_mandatory() {
    let f = Fixture::git("t07");
    every_state(&f);
    let want = json!([
        {"criterion":"C1","code":"asserted_not_observed","mandatory":true},
        {"criterion":"C7","code":"asserted_not_observed","mandatory":false}
    ]);
    assert_eq!(f.verdict_json(&[]).1["flags"], want);
    assert_eq!(f.verdict_json(&["--require", "asserted"]).1["flags"], want);
}

/// T08: missing, unverified, stale and failed stay unmet under both policies, each with its
/// contracted reason.
#[test]
fn t08_verdict_missing_unverified_stale_unmet_under_both_policies() {
    let no_exit = |h: &CandidateId| Rc {
        exit_code: None,
        ..rc("C2", "mcp-asserted:x", h, 0)
    };
    let unsupported = |h: &CandidateId| Rc {
        claim_supported: false,
        ..rc("C2", "mcp-asserted:x", h, 0)
    };
    type Case<'a> = (&'a str, &'a dyn Fn(&Fixture), &'a str, Option<&'a str>);
    let cases: [Case<'_>; 7] = [
        (
            "missing",
            &|f| drop(f.task(&[("C2", true)], &[], &[])),
            "missing",
            None,
        ),
        (
            "unverified",
            &|f| {
                drop(f.task(
                    &[("C2", true)],
                    &[rc("C2", "legacy.log", &f.head(), 0)],
                    &[],
                ))
            },
            "unverified",
            Some("no_observation_record"),
        ),
        (
            "other-head",
            &|f| {
                f.observed_pass();
                fs::write(f.project.join("README"), "two\n").unwrap();
                f.commit("two");
            },
            "stale",
            Some("receipt_at_other_head"),
        ),
        (
            "worktree",
            &|f| {
                f.observed_pass();
                fs::write(f.project.join("README"), "edited\n").unwrap();
            },
            "stale",
            Some("worktree_changed"),
        ),
        (
            "exit-nonzero",
            &|f| {
                drop(f.task(
                    &[("C2", true)],
                    &[rc("C2", "observed:{task}:a", &f.head(), 1)],
                    &[("a", 0, &same)],
                ))
            },
            "failed",
            Some("exit_nonzero"),
        ),
        (
            "no-exit-code",
            &|f| drop(f.task(&[("C2", true)], &[no_exit(&f.head())], &[])),
            "failed",
            Some("no_exit_code"),
        ),
        (
            "claim-unsupported",
            &|f| drop(f.task(&[("C2", true)], &[unsupported(&f.head())], &[])),
            "failed",
            Some("claim_unsupported"),
        ),
    ];
    for (name, setup, state, reason) in cases {
        let f = Fixture::git(&format!("t08-{name}"));
        setup(&f);
        for require in ["observed", "asserted"] {
            let (code, r) = f.verdict_json(&["--require", require]);
            assert_eq!(code, 3, "{name} {require}: {r}");
            assert_eq!(r["reason"], "mandatory_unmet", "{name} {require}");
            assert_eq!(r["unmet"], json!(["C2"]), "{name} {require}");
            let c = criterion(&r, "C2");
            assert_eq!(c["state"], state, "{name} {require}");
            assert_eq!(c["reason"].as_str(), reason, "{name} {require}");
        }
    }
}

/// T09a [K02]: a run row that disagrees with the receipt on any matched field is not observed.
/// (Consistency only: a perfectly matching forged row would pass; r3 §9 accepted risk.)
#[test]
fn t09a_verdict_observed_prefix_with_inconsistent_run_record_is_unverified() {
    let cases: [(&str, &Mutate); 11] = [
        ("exit", &|v| v["exit_code"] = json!(1)),
        ("cand", &|v| {
            v["candidate"]["base_commit"] = json!("f".repeat(40))
        }),
        ("digest", &|v| {
            v["candidate"]["dirty_digest"] = json!("other")
        }),
        ("cmd", &|v| v["command"] = json!("true")),
        ("start", &|v| v["t"] = json!(999)),
        ("finish", &|v| v["finished"] = json!(999)),
        ("task", &|v| v["task_id"] = json!("task-someone-else")),
        ("dispatch", &|v| v["dispatch_id"] = json!("other")),
        ("trust", &|v| v["completion_trust"] = json!("asserted")),
        ("kind", &|v| v["kind"] = json!("started")),
        ("profile", &|v| {
            v["execution_profile"]["approved"] = json!(false)
        }),
    ];
    let control = Fixture::git("t09a-control");
    control.observed_pass();
    assert_eq!(
        control.verdict_json(&[]).0,
        0,
        "control: the unmutated row is observed"
    );
    for (name, mutate) in cases {
        let f = Fixture::git(&format!("t09a-{name}"));
        f.task(
            &[("C2", true)],
            &[rc("C2", "observed:{task}:live", &f.head(), 0)],
            &[("live", 0, mutate)],
        );
        let (code, report) = f.verdict_json(&[]);
        assert_eq!(code, 3, "{name}: {report}");
        assert_eq!(
            criterion(&report, "C2")["state"],
            "unverified",
            "{name}: {report}"
        );
    }
}

/// T09b [K03]: a reference and row naming another task are inconsistent with this task.
#[test]
fn t09b_verdict_reference_from_another_task_is_unverified() {
    let f = Fixture::git("t09b");
    f.task(
        &[("C2", true)],
        &[rc("C2", "observed:task-old:live", &f.head(), 0)],
        &[("live", 0, &|v: &mut Value| v["task_id"] = json!("task-old"))],
    );
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{report}");
    assert_eq!(criterion(&report, "C2")["state"], "unverified");
}

/// T09c [K04]: an invalidated dispatch no longer backs a receipt.
#[test]
fn t09c_verdict_invalidated_dispatch_is_unverified() {
    let f = Fixture::git("t09c");
    let (_, dir, _) = f.observed_pass();
    assert_eq!(
        f.verdict_json(&[]).0,
        0,
        "control: observed before invalidation"
    );
    fs::write(
        dir.join("autonomy.json"),
        json!({"schema":1,"revisions":[],"invalidated_dispatches":["a"]}).to_string(),
    )
    .unwrap();
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{report}");
    assert_eq!(criterion(&report, "C2")["state"], "unverified");
}

/// T10: every deciding receipt that is not stale is at exactly the reported head.
#[test]
fn t10_verdict_current_receipt_sha_and_digest_equal_head() {
    let f = Fixture::git("t10");
    every_state(&f);
    let (_, r) = f.verdict_json(&[]);
    let mut checked = 0;
    for c in r["criteria"].as_array().unwrap() {
        if c["state"] != "stale" && !c["receipt"].is_null() {
            assert_eq!(c["receipt"]["head_sha"], r["head"]["sha"], "{c}");
            assert_eq!(
                c["receipt"]["dirty_digest"], r["head"]["dirty_digest"],
                "{c}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 6, "control: {r}");
    assert_ne!(criterion(&r, "C5")["receipt"]["head_sha"], r["head"]["sha"]);
}

/// T12: a clean worktree's digest is sha256 of nothing; an edit changes it.
#[test]
fn t12_verdict_clean_tree_digest_is_sha256_of_empty() {
    let f = Fixture::git("t12");
    assert_eq!(sha256::hex(b""), CLEAN);
    assert_eq!(f.verdict_json(&[]).1["head"]["dirty_digest"], CLEAN);
    fs::write(f.project.join("README"), "edited\n").unwrap();
    let digest = f.verdict_json(&[]).1["head"]["dirty_digest"].clone();
    assert!(is_hex(&digest, 64) && digest != CLEAN, "{digest}");
}

/// T13 [K12]: unborn HEAD is unknown: sha and digest null together, enum reason git_failed.
#[test]
fn t13_verdict_head_unknown_nulls_sha_and_digest_with_enum_reason() {
    let f = Fixture::new("t13");
    f.run_git(&["init", "-q"]);
    let any = CandidateId {
        base_commit: "0".repeat(40),
        dirty_digest: "x".into(),
    };
    f.task(
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &any, 0)],
        &[("a", 0, &same)],
    );
    let (code, r) = f.verdict_json(&["--require", "asserted"]);
    assert_eq!(code, 3, "{r}");
    assert_eq!(r["reason"], "head_unknown");
    assert_eq!(criterion(&r, "C2")["state"], "stale");
    assert_eq!(criterion(&r, "C2")["reason"], "head_unknown");
    assert_eq!(r["head"]["sha"], Value::Null);
    assert_eq!(
        r["head"]["dirty_digest"],
        Value::Null,
        "R3: null together with sha"
    );
    assert_eq!(
        r["head"]["reason"], "git_failed",
        "R3: closed enum, no free text"
    );
}

/// T14: each cause of an unknown HEAD is one enum value; the free-text cause is on stderr.
#[test]
fn t14_verdict_head_reason_carries_no_free_text_or_paths() {
    let check = |f: &Fixture, out: Output, want: &str| {
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let (code, r) = parse(&out);
        assert_eq!(code, 3, "{r}");
        assert_eq!(
            r["head"],
            json!({"sha":null,"dirty_digest":null,"reason":want})
        );
        let root = f.root.to_string_lossy();
        assert!(
            !stdout.contains(root.as_ref()),
            "no paths on stdout: {stdout}"
        );
        assert!(!out.stderr.is_empty(), "{want}: the cause goes to stderr");
    };
    let no_repo = Fixture::new("t14-no-repo");
    check(&no_repo, no_repo.verdict(&["--json"]), "git_failed");
    let sha256_repo = Fixture::new("t14-sha256");
    sha256_repo.run_git(&["init", "-q", "--object-format=sha256"]);
    fs::write(sha256_repo.project.join("README"), "one\n").unwrap();
    sha256_repo.commit("one");
    check(
        &sha256_repo,
        sha256_repo.verdict(&["--json"]),
        "head_unparseable",
    );
    let no_git = Fixture::git("t14-no-git");
    let empty = no_git.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let out = no_git
        .verdict_command(&["--json"])
        .env("PATH", &empty)
        .output()
        .unwrap();
    check(&no_git, out, "git_unavailable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let unreadable = Fixture::git("t14-unreadable");
        let secret = unreadable.project.join("secret");
        fs::write(&secret, "x").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        check(
            &unreadable,
            unreadable.verdict(&["--json"]),
            "worktree_unreadable",
        );
    }
}

/// T18a [K01]: MCP receipts stay `mcp` whatever reference shape the caller writes, and a
/// caller-supplied candidate that is not byte-equal to the live one is stale, never current.
#[test]
fn t18a_verdict_mcp_receipt_stays_mcp_and_caller_candidate_is_stale() {
    let f = Fixture::git("t18a");
    let mut mcp = Mcp::start(&f);
    let started = mcp.tool(
        "task_start",
        json!({"id":"probe","goal":"g",
               "criteria":[{"id":"C1","check":"x","command":"false","mandatory":true},
                           {"id":"C3","check":"x","command":"false","mandatory":true}],
               "forbidden_effects":[],"scope":["src/"],"max_repair_rounds":1}),
    );
    let task = started["task_id"].as_str().unwrap().to_owned();
    let project = f.project.to_string_lossy().into_owned();
    mcp.tool("candidate_observe", json!({"task_id":task}));
    for output_ref in [
        format!("observed:{task}:live"),
        format!("tc-reconstructed:{task}:live"),
    ] {
        mcp.tool(
            "receipt_record",
            json!({"task_id":task,"criterion":"C1","command":"false","cwd":project,
                   "exit_code":0,"output_ref":output_ref,"claim_supported":true}),
        );
    }
    let live = f.head();
    mcp.tool(
        "candidate_observe",
        json!({"task_id":task,"base_commit":live.base_commit.to_uppercase(),
               "dirty_digest":live.dirty_digest,"changed_paths":[]}),
    );
    mcp.tool(
        "receipt_record",
        json!({"task_id":task,"criterion":"C3","command":"false","cwd":project,
               "exit_code":0,"output_ref":"x","claim_supported":true}),
    );
    drop(mcp);
    let (code, report) = f.verdict_json(&["--require", "asserted"]);
    assert_eq!(code, 3, "{report}");
    let c1 = criterion(&report, "C1");
    assert_eq!(c1["state"], "asserted_not_observed");
    assert_eq!(c1["provenance"], "mcp");
    let c3 = criterion(&report, "C3");
    assert_eq!(c3["state"], "stale", "{report}");
    assert_eq!(c3["reason"], "receipt_at_other_head");
}

/// T18b [K13]: the join is byte-exact: short, upper-case, padded or CRLF shas are not current.
/// Kills M4.
#[test]
fn t18b_verdict_head_join_is_byte_exact() {
    type Form = dyn Fn(&str) -> String;
    let forms: [(&str, &Form); 6] = [
        ("short12", &|s| s[..12].to_owned()),
        ("short7", &|s| s[..7].to_owned()),
        ("upper", &|s| s.to_uppercase()),
        ("lf", &|s| format!("{s}\n")),
        ("crlf", &|s| format!("{s}\r\n")),
        ("space", &|s| format!(" {s}")),
    ];
    let control = Fixture::git("t18b-control");
    control.observed_pass();
    assert_eq!(
        control.verdict_json(&[]).0,
        0,
        "control: exact sha is current"
    );
    for (name, form) in forms {
        let f = Fixture::git(&format!("t18b-{name}"));
        let mut head = f.head();
        head.base_commit = form(&head.base_commit);
        f.task(
            &[("C2", true)],
            &[rc("C2", "observed:{task}:a", &head, 0)],
            &[("a", 0, &same)],
        );
        let (code, r) = f.verdict_json(&[]);
        assert_eq!(code, 3, "{name}: {r}");
        assert_eq!(criterion(&r, "C2")["state"], "stale", "{name}");
        assert_eq!(
            criterion(&r, "C2")["reason"],
            "receipt_at_other_head",
            "{name}"
        );
    }
}

/// T20 (+T19): no_active_task > head_unknown > no_mandatory_criteria > mandatory_unmet.
#[test]
fn t20_verdict_top_level_reason_precedence() {
    let a = Fixture::new("t20a");
    assert_eq!(a.verdict_json(&[]).1["reason"], "no_active_task");
    let b = Fixture::new("t20b");
    b.task(&[("C2", true)], &[], &[]);
    let (code, r) = b.verdict_json(&[]);
    assert_eq!((code, &r["reason"]), (3, &json!("head_unknown")), "{r}");
    let c = Fixture::new("t20c");
    c.task(&[("C9", false)], &[], &[]);
    assert_eq!(c.verdict_json(&[]).1["reason"], "head_unknown");
    let d = Fixture::git("t20d");
    d.task(&[("C9", false)], &[], &[]);
    let (code, r) = d.verdict_json(&[]);
    assert_eq!(code, 3, "T19: zero mandatory criteria fails closed: {r}");
    assert_eq!(r["verdict"], "fail");
    assert_eq!(r["reason"], "no_mandatory_criteria");
    let e = Fixture::git("t20e");
    e.task(&[("C2", true)], &[], &[]);
    assert_eq!(e.verdict_json(&[]).1["reason"], "mandatory_unmet");
}

/// T21: the `exit_code` field is the process exit, in both directions.
#[test]
fn t21_verdict_exit_code_field_mirrors_process_exit() {
    let pass = Fixture::git("t21-pass");
    pass.observed_pass();
    let fail = Fixture::git("t21-fail");
    fail.task(&[("C2", true)], &[], &[]);
    for (f, want) in [(&pass, 0), (&fail, 3)] {
        let (code, r) = f.verdict_json(&[]);
        assert_eq!(code, want, "{r}");
        assert_eq!(r["exit_code"], json!(want));
        assert_eq!(
            f.verdict(&[]).status.code(),
            Some(want),
            "human output exits the same"
        );
    }
}

/// T22a [K11]: tampered or unreadable gate.json is exit 1 with empty stdout, JSON or human.
#[test]
fn t22a_verdict_stdout_empty_on_tampered_or_unreadable_state() {
    let f = Fixture::git("t22a");
    let (_, dir) = f.task(
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &f.head(), 1)],
        &[("a", 0, &same)],
    );
    assert_eq!(
        f.verdict_json(&[]).0,
        3,
        "control: answers before tampering"
    );
    let gate = dir.join("gate.json");
    let text = fs::read_to_string(&gate).unwrap();
    assert!(text.contains("\"exit_code\":1"), "{text}");
    fs::write(&gate, text.replace("\"exit_code\":1", "\"exit_code\":0")).unwrap();
    assert_exit1_empty(&f.verdict(&["--json"]), "tampered --json");
    assert_exit1_empty(&f.verdict(&[]), "tampered human");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o000)).unwrap();
        let out = f.verdict(&["--json"]);
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o644)).unwrap();
        assert_exit1_empty(&out, "unreadable");
    }
}

/// T22b + T23 [K15]: usage is exit 2 with empty stdout; `--task` mismatch or no active task is
/// exit 1. The positive control comes first, so this cannot pass where `verdict` is absent.
#[test]
fn t22b_verdict_usage_and_task_mismatch_with_positive_control() {
    let f = Fixture::git("t22b");
    let control = f.verdict(&["--require", "observed", "--json"]);
    assert_eq!(
        control.status.code(),
        Some(3),
        "control: verdict answers (no task)"
    );
    assert!(!control.stdout.is_empty());
    for args in [
        &["--require", "vibes"][..],
        &["--require"],
        &["--strict"],
        &["--help"],
    ] {
        let out = f.verdict(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(
            out.stdout.is_empty(),
            "{args:?}: usage stdout must be empty"
        );
    }
    assert_exit1_empty(
        &f.verdict(&["--task", "task-x", "--json"]),
        "--task with no active task",
    );
    let (task, _) = f.task(&[("C2", true)], &[], &[]);
    assert_exit1_empty(
        &f.verdict(&["--task", "task-not-this-one", "--json"]),
        "--task mismatch",
    );
    assert_eq!(
        f.verdict(&["--task", &task]).status.code(),
        Some(3),
        "--task matches"
    );
}

/// T24: the stored gate state is never consulted, in either direction.
#[test]
fn t24_verdict_ignores_stored_gate_state() {
    let blocked = Fixture::git("t24-blocked");
    let head = blocked.head();
    blocked.task_stored(
        TaskState::Blocked,
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &head, 0)],
        &[("a", 0, &same)],
    );
    assert_eq!(
        blocked.verdict_json(&[]).0,
        0,
        "a blocked gate with evidence passes"
    );
    let completed = Fixture::git("t24-completed");
    completed.task_stored(TaskState::Completed, &[("C2", true)], &[], &[]);
    assert_eq!(
        completed.verdict_json(&[]).0,
        3,
        "a completed gate without evidence fails"
    );
}

/// T25: HOUSE's repro. The gate is `completed` on a fabricated MCP receipt; the strict
/// verdict fails it and names C1 as asserted.
#[test]
fn t25_verdict_fabricated_mcp_receipt_fails_strict() {
    let f = Fixture::git("t25");
    replay_fabricated_receipt(&f);
    let (code, r) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{r}");
    assert_eq!(r["unmet"], json!(["C1"]));
    assert_eq!(criterion(&r, "C1")["state"], "asserted_not_observed");
    assert_eq!(criterion(&r, "C1")["provenance"], "mcp");
    let out = f.verdict(&[]);
    assert_eq!(out.status.code(), Some(3));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].starts_with("C1: ") && lines[1].starts_with("verdict: fail ("),
        "{text}"
    );
}

/// T26: a malformed runs.jsonl line is exit 1 with empty stdout, never unverified/3.
#[test]
fn t26_verdict_corrupt_runs_journal_is_exit_one_with_empty_stdout() {
    let f = Fixture::git("t26");
    let (_, dir, _) = f.observed_pass();
    assert_eq!(
        f.verdict_json(&[]).0,
        0,
        "control: consistent journal is observed"
    );
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(dir.join("runs.jsonl"))
        .unwrap();
    writeln!(journal, "{{not json").unwrap();
    drop(journal);
    assert_exit1_empty(&f.verdict(&["--json"]), "corrupt runs.jsonl");
}

/// T27: invalid autonomy.json is exit 1 with empty stdout.
#[test]
fn t27_verdict_invalid_autonomy_metadata_is_exit_one() {
    for (name, body) in [
        ("schema", r#"{"schema":2,"revisions":[]}"#),
        ("syntax", "{nope"),
        (
            "ids",
            r#"{"schema":1,"revisions":[],"invalidated_dispatches":[7]}"#,
        ),
    ] {
        let f = Fixture::git(&format!("t27-{name}"));
        let (_, dir, _) = f.observed_pass();
        assert_eq!(f.verdict_json(&[]).0, 0, "{name}: control");
        fs::write(dir.join("autonomy.json"), body).unwrap();
        assert_exit1_empty(&f.verdict(&["--json"]), name);
    }
}

type Snapshot = BTreeMap<PathBuf, (Vec<u8>, SystemTime)>;
/// Every directory and file under `dir`, with file bytes and mtimes.
fn snapshot(dir: &Path) -> Snapshot {
    let mut all = Snapshot::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        all.insert(d.clone(), (Vec::new(), UNIX_EPOCH));
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                all.insert(path.clone(), (fs::read(&path).unwrap(), modified));
            }
        }
    }
    all
}

/// T28 [K14, no state.lock exclusion]: no state dir is created, and with a task nothing in the
/// state dir (lock included) or project is created or touched. Controls: each run answers.
#[test]
fn t28_verdict_writes_nothing_no_lock_no_state_dir() {
    let f = Fixture::git("t28");
    assert_eq!(
        f.verdict(&["--json"]).status.code(),
        Some(3),
        "control: no_active_task"
    );
    assert!(!f.state.exists(), "R11: verdict created the state dir");
    f.observed_pass();
    fs::remove_file(f.state_root().join("state.lock")).unwrap();
    let before = (snapshot(&f.state), snapshot(&f.project));
    for args in [&[][..], &["--json"], &["--require", "asserted"]] {
        assert_eq!(
            f.verdict(args).status.code(),
            Some(0),
            "control: {args:?} passes"
        );
    }
    assert!(
        before == (snapshot(&f.state), snapshot(&f.project)),
        "verdict wrote a file"
    );
}

/// T29: a pending transaction.json is exit 1 and is NOT rolled forward by verdict.
#[test]
fn t29_verdict_pending_transaction_is_exit_one_not_recovered() {
    let f = Fixture::git("t29");
    f.observed_pass();
    let root = f.state_root();
    let active = sha256_file(&root.join("active.json"));
    let journal = json!({"schema":1,"before_active":active,"after_active":active,
        "writes":[{"path":"marker.txt","text":"rolled forward"}]});
    fs::write(root.join("transaction.json"), format!("{journal}\n")).unwrap();
    assert_exit1_empty(&f.verdict(&["--json"]), "pending transaction");
    assert!(
        root.join("transaction.json").exists(),
        "R11: verdict consumed the transaction"
    );
    assert!(
        !root.join("marker.txt").exists(),
        "R11: verdict rolled the transaction forward"
    );
}

/// T30: .git/index is untouched (R12). README is made stat-dirty but content-clean, so a
/// refreshing `git status` would rewrite the index; the control proves it does.
#[test]
fn t30_verdict_leaves_git_index_untouched() {
    let f = Fixture::git("t30");
    f.observed_pass();
    std::thread::sleep(Duration::from_millis(1100));
    let readme = fs::read(f.project.join("README")).unwrap();
    fs::write(f.project.join("README"), &readme).unwrap();
    let index = f.project.join(".git").join("index");
    let read_index = || {
        (
            fs::read(&index).unwrap(),
            fs::metadata(&index).unwrap().modified().unwrap(),
        )
    };
    let before = read_index();
    let (code, r) = f.verdict_json(&[]);
    assert_eq!(
        code, 0,
        "control: same content is still the current candidate: {r}"
    );
    let after = read_index();
    assert!(before == after, "R12: verdict rewrote .git/index");
    f.run_git(&["status"]);
    assert!(
        read_index() != after,
        "control: setup failed to make the index refreshable"
    );
}

/// T31: criteria[] in contract.json order; unmet[] and flags[] ordered subsequences (Q5).
#[test]
fn t31_verdict_arrays_follow_contract_order() {
    let f = Fixture::git("t31");
    every_state(&f);
    let (_, r) = f.verdict_json(&[]);
    let ids: Vec<&str> = r["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["C3", "C1", "C2", "C6", "C5", "C4", "C7", "C0"], "{r}");
    assert_eq!(r["unmet"], json!(["C1", "C2", "C6", "C5", "C4"]));
    let flagged: Vec<&Value> = r["flags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["criterion"])
        .collect();
    assert_eq!(flagged, [&json!("C1"), &json!("C7")]);
}

/// T32: human stdout is criteria+1 lines: `<id>: ` prefixes in order, then
/// `verdict: pass|fail (`. Nothing after those prefixes is asserted (Q2).
#[test]
fn t32_verdict_human_output_only_prefixes_are_stable() {
    let lines_of = |f: &Fixture, args: &[&str]| -> Vec<String> {
        String::from_utf8(f.verdict(args).stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let f = Fixture::git("t32");
    let head = f.head();
    f.task(
        &[("C3", true), ("C1", false), ("C2", true)],
        &[
            rc("C3", "observed:{task}:a", &head, 0),
            rc("C2", "observed:{task}:b", &head, 0),
        ],
        &[("a", 0, &same), ("b", 1, &same)],
    );
    for args in [&[][..], &["--require", "asserted"]] {
        let lines = lines_of(&f, args);
        assert_eq!(lines.len(), 4, "{lines:?}");
        for (line, id) in lines.iter().zip(["C3: ", "C1: ", "C2: "]) {
            assert!(line.starts_with(id), "{line}");
        }
        assert!(lines[3].starts_with("verdict: pass ("), "{lines:?}");
    }
    let g = Fixture::git("t32-fail");
    every_state(&g);
    let lines = lines_of(&g, &[]);
    let ids = ["C3", "C1", "C2", "C6", "C5", "C4", "C7", "C0"];
    assert_eq!(lines.len(), ids.len() + 1, "{lines:?}");
    for (line, id) in lines.iter().zip(ids) {
        assert!(line.starts_with(&format!("{id}: ")), "{line}");
    }
    assert!(lines[ids.len()].starts_with("verdict: fail ("), "{lines:?}");
}

/// T33 [K08]: non-mandatory criteria are listed and never affect verdict/unmet/exit. Kills M6.
#[test]
fn t33_verdict_optional_criteria_never_affect_verdict() {
    let f = Fixture::git("t33");
    let head = f.head();
    f.task(
        &[("C2", true), ("C9", false), ("C8", false)],
        &[
            rc("C2", "observed:{task}:a", &head, 0),
            rc("C8", "observed:{task}:b", &head, 1),
        ],
        &[("a", 0, &same), ("b", 1, &same)],
    );
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["verdict"], "pass");
    assert_eq!(report["unmet"], json!([]));
    assert_eq!(criterion(&report, "C9")["mandatory"], false);
    assert_eq!(criterion(&report, "C9")["state"], "missing");
    assert_eq!(criterion(&report, "C8")["state"], "failed");
}

/// T34 [K07] + T06: strict is the default; lenient flips `satisfies`, not `state`. Kills M1.
#[test]
fn t34_verdict_default_require_is_observed() {
    let f = Fixture::git("t34");
    replay_fabricated_receipt(&f);
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{report}");
    assert_eq!(report["require"], "observed");
    assert_eq!(criterion(&report, "C1")["satisfies"], false);
    assert_eq!(criterion(&report, "C1")["state"], "asserted_not_observed");
    let (code, report) = f.verdict_json(&["--require", "asserted"]);
    assert_eq!(code, 0);
    assert_eq!(report["require"], "asserted");
    assert_eq!(criterion(&report, "C1")["satisfies"], true);
    assert_eq!(criterion(&report, "C1")["state"], "asserted_not_observed");
}

/// KX01a: HEAD moved before the snapshot: stale/receipt_at_other_head, exit 3, never 1.
#[test]
fn kx01a_verdict_head_moved_before_snapshot_is_stale_not_error() {
    let f = Fixture::git("kx01a");
    let (_, _, old) = f.observed_pass();
    assert_eq!(f.verdict_json(&[]).0, 0, "control");
    fs::write(f.project.join("README"), "two\n").unwrap();
    f.commit("two");
    let (code, r) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{r}");
    assert_eq!(r["head"]["sha"], f.rev_parse().as_str());
    assert_eq!(criterion(&r, "C2")["state"], "stale");
    assert_eq!(criterion(&r, "C2")["reason"], "receipt_at_other_head");
    assert_eq!(
        criterion(&r, "C2")["receipt"]["head_sha"],
        old.base_commit.as_str()
    );
}

/// KX01b: HEAD moves during the verdict (a PATH git shim commits on the first `status`). The
/// verdict is about the snapshot it reports: an answer for the old sha, never an error.
#[cfg(unix)]
#[test]
fn kx01b_verdict_head_moved_during_verdict_reports_the_snapshot() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::git("kx01b");
    let (_, _, old) = f.observed_pass();
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let bin = f.root.join("shim");
    fs::create_dir_all(&bin).unwrap();
    let marker = f.root.join("moved");
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in *\" status \"*) if [ ! -e '{m}' ]; then : > '{m}'; '{g}' -C '{p}' -c user.name=x -c user.email=x@x.invalid commit -q --allow-empty -m moved; fi;; esac\nexec '{g}' \"$@\"\n",
        m = marker.display(),
        g = real,
        p = f.project.display()
    );
    fs::write(bin.join("git"), script).unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = f
        .verdict_command(&["--json"])
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        marker.exists(),
        "control: the shim moved HEAD during the verdict"
    );
    assert_ne!(f.rev_parse(), old.base_commit, "control: HEAD really moved");
    let (code, r) = parse(&out);
    assert_eq!(code, 0, "{r}");
    assert_eq!(
        r["head"]["sha"],
        old.base_commit.as_str(),
        "head is the snapshot evaluated"
    );
    assert_eq!(
        criterion(&r, "C2")["receipt"]["head_sha"],
        r["head"]["sha"],
        "r3 §5 invariant"
    );
}

/// KX05 [K05]: tc-reconstructed with a consistent row is observed/tc; without one, unverified.
#[test]
fn kx05_verdict_tc_reconstructed_provenance() {
    let f = Fixture::git("kx05");
    f.task(
        &[("C2", true)],
        &[rc("C2", "tc-reconstructed:{task}:live", &f.head(), 0)],
        &[("live", 0, &same)],
    );
    let (code, report) = f.verdict_json(&[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(criterion(&report, "C2")["provenance"], "tc");
    assert_eq!(criterion(&report, "C2")["state"], "observed");
    let g = Fixture::git("kx05b");
    g.task(
        &[("C2", true)],
        &[rc("C2", "tc-reconstructed:{task}:live", &g.head(), 0)],
        &[],
    );
    assert_eq!(
        criterion(&g.verdict_json(&[]).1, "C2")["state"],
        "unverified"
    );
}

/// KX06 [r3.1 N1]: a state directory whose project.json names another worktree, or is not a
/// project identity at all, is exit 1 with empty stdout; the verdict answers before either.
#[test]
fn kx06_verdict_foreign_or_invalid_project_json_is_exit_one() {
    let f = Fixture::git("kx06");
    f.observed_pass();
    assert_eq!(f.verdict_json(&[]).0, 0, "control: answers before");
    let project = f.state_root().join("project.json");
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&project).unwrap()).unwrap();
    identity["canonical_worktree"] = "/another/worktree".into();
    fs::write(&project, identity.to_string()).unwrap();
    assert_exit1_empty(&f.verdict(&["--json"]), "foreign worktree");
    fs::write(&project, "not json").unwrap();
    assert_exit1_empty(&f.verdict(&["--json"]), "invalid project.json");
}

/// WP4 review: a budget_exhausted task never passes, whatever its receipts say. The verdict
/// names the state, and the limit when a task limit is reached; the JSON keys are unchanged.
#[test]
fn t40_verdict_budget_exhausted_task_fails_and_names_its_limit() {
    let f = Fixture::git("t40");
    let head = f.head();
    let (_, dir) = f.task_stored_with(
        TaskState::BudgetExhausted,
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &head, 0)],
        &[("a", 0, &same)],
        &json!({"limits":{"turns":1}}),
    );
    fs::write(
        dir.join("events.jsonl"),
        "{\"v\":1,\"event\":\"PreToolUse\"}\n",
    )
    .unwrap();
    let (code, r) = f.verdict_json(&[]);
    assert_eq!(code, 3, "{r}");
    assert_eq!(r["verdict"], "fail", "{r}");
    assert_eq!(r["reason"], "budget_exhausted", "{r}");
    assert_eq!(
        keys(&r["task"]),
        set(&["task_id", "contract_id", "contract_digest"])
    );
    let human = String::from_utf8_lossy(&f.verdict(&[]).stdout).into_owned();
    assert!(human.contains("task state: budget_exhausted"), "{human}");
    assert!(
        human.contains("task limit reached: 1 of 1 turns used"),
        "{human}"
    );
}

/// WP4 review round 2: a budget_exhausted task from spent repair rounds, with no limit set, fails
/// too, and the text output names no limit.
#[test]
fn t41_verdict_repair_rounds_budget_exhausted_fails_without_limit_text() {
    let f = Fixture::git("t41");
    let head = f.head();
    f.task_stored(
        TaskState::BudgetExhausted,
        &[("C2", true)],
        &[rc("C2", "observed:{task}:a", &head, 0)],
        &[("a", 0, &same)],
    );
    let (code, r) = f.verdict_json(&[]);
    assert_eq!((code, &r["reason"]), (3, &json!("budget_exhausted")), "{r}");
    let human = String::from_utf8_lossy(&f.verdict(&[]).stdout).into_owned();
    assert!(human.contains("task state: budget_exhausted"), "{human}");
    assert!(!human.contains("task limit"), "{human}");
}
