//! T2 scenarios replayed through the adapter. Payloads are the captured T3 payloads with fields edited.
//!
//! R23 (repair cap) is not expressible through adapter v1: no hook event maps to
//! `request_repair`/`finish_repair`, so the adapter never consumes a repair round. It stays
//! covered by the core suite (tests/scenarios.rs) only; it is not faked here.

use std::fs;
use std::path::PathBuf;

use coding_supervisor::{Candidate, CandidateId, CriterionId, TaskState, VerificationReceipt};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, MCP_ASSERTED, handle};
use supervisor_claude_code::candidate::CandidateProvider;
use supervisor_claude_code::journal::decision_parts;
use supervisor_claude_code::state::{Stored, load_gate, read_contract, save_gate};

const CAPTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/claude-code-enforcement/captures/"
);
const FOREGROUND: &str = "41280c28-a6af-4be4-9963-92e677e85d4f.events.jsonl";
const BACKGROUND: &str = "bb8444b7-3db5-4d0f-9b53-d60ebe903e20.events.jsonl";

struct FakeCandidate {
    id: CandidateId,
    changed_paths: Vec<String>,
}
impl CandidateProvider for FakeCandidate {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: self.id.clone(),
            provenance: "fake".into(),
            changed_paths: self.changed_paths.clone(),
        })
    }
}
fn fake(digest: &str, paths: &[&str]) -> Box<FakeCandidate> {
    Box::new(FakeCandidate {
        id: CandidateId {
            base_commit: "base".into(),
            dirty_digest: digest.into(),
        },
        changed_paths: paths.iter().map(|p| p.to_string()).collect(),
    })
}

struct Unobservable;
impl CandidateProvider for Unobservable {
    fn observe(&self) -> Result<Candidate, String> {
        Err("git rev-parse HEAD failed".into())
    }
}

/// A temp directory removed when the test ends.
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!("supervisor-cc-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    Dir(path)
}

fn contract(criteria: &[(&str, bool)], scope: &[&str]) -> Value {
    json!({
        "id": "t4-test",
        "goal": "replay",
        // The commands the tests below cite: `cargo test c1` .. `cargo test c4` and `cargo test`.
        "criteria": criteria.iter().map(|(id, m)| json!({"id": id, "check": id, "mandatory": m,
            "command": [format!("cargo test {}", id.to_lowercase()), "cargo test".to_owned()]})).collect::<Vec<_>>(),
        "forbidden_effects": [],
        "scope": scope,
        "max_repair_rounds": 2
    })
}
fn four() -> Value {
    contract(
        &[("C1", true), ("C2", true), ("C3", true), ("C4", true)],
        &["src/", "tests/"],
    )
}

fn env_at(d: &Dir, candidate: Box<dyn CandidateProvider>) -> Env {
    Env {
        state_dir: d.0.join(".supervisor"),
        state_location: "test",
        candidate,
        clock: || 1_000,
    }
}
fn env(d: &Dir, contract: Option<Value>, candidate: Box<dyn CandidateProvider>) -> Env {
    let env = env_at(d, candidate);
    fs::create_dir_all(&env.state_dir).unwrap();
    if let Some(c) = contract {
        fs::write(env.state_dir.join("contract.json"), c.to_string()).unwrap();
    }
    env
}
fn gate(env: &Env) -> Value {
    serde_json::from_str(&fs::read_to_string(env.state_dir.join("gate.json")).unwrap()).unwrap()
}

fn captured(file: &str) -> Vec<Value> {
    fs::read_to_string(format!("{CAPTURES}{file}"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap()["payload"].clone())
        .collect()
}
fn template(file: &str, event: &str, tool: Option<&str>) -> Value {
    captured(file)
        .into_iter()
        .find(|p| p["hook_event_name"] == event && tool.is_none_or(|t| p["tool_name"] == t))
        .unwrap()
}

fn bash(agent: &str, command: &str, exit: i32) -> Value {
    let mut p = template(FOREGROUND, "PostToolUse", Some("Write"));
    p["tool_name"] = json!("Bash");
    p["tool_input"] = json!({ "command": command });
    p["tool_use_id"] = json!(format!("toolu_{agent}_{command}_{exit}"));
    p["agent_id"] = json!(agent);
    if exit == 0 {
        p["tool_response"] = json!({"stdout": "ok", "stderr": "", "interrupted": false});
    } else {
        p["hook_event_name"] = json!("PostToolUseFailure");
        p.as_object_mut().unwrap().remove("tool_response");
        p["error"] = json!(format!("Exit code {exit}\nfailed"));
    }
    p
}
fn handback(agent: &str, message: &str, active: bool) -> Value {
    let mut p = template(FOREGROUND, "SubagentStop", None);
    p["agent_id"] = json!(agent);
    p["last_assistant_message"] = json!(message);
    p["stop_hook_active"] = json!(active);
    p
}
fn stop(message: &str, active: bool) -> Value {
    let mut p = template(FOREGROUND, "Stop", None);
    p["last_assistant_message"] = json!(message);
    p["stop_hook_active"] = json!(active);
    p
}
fn pre(tool: &str) -> Value {
    let mut p = template(FOREGROUND, "PreToolUse", Some("Write"));
    p["tool_name"] = json!(tool);
    p
}

/// Each line runs as a real command by `agent`, then the worker hands back citing all of them.
fn worker(env: &mut Env, agent: &str, verdict: &str, lines: &[(&str, &str, i32)]) -> Decision {
    for (_, command, exit) in lines {
        assert_eq!(handle(&bash(agent, command, *exit), env), Decision::None);
    }
    let cited: Vec<String> = lines
        .iter()
        .map(|(id, command, exit)| format!("CHECK {id} | {command} | exit {exit}"))
        .collect();
    let message = format!(
        "VERDICT: {verdict}\nCHECKS RUN: {}\n{}",
        lines.len(),
        cited.join("\n")
    );
    handle(&handback(agent, &message, false), env)
}
const ALL4: [(&str, &str, i32); 4] = [
    ("C1", "cargo test c1", 0),
    ("C2", "cargo test c2", 0),
    ("C3", "cargo test c3", 0),
    ("C4", "cargo test c4", 0),
];

fn blocked(d: &Decision) -> &str {
    match d {
        Decision::Block(reason) => reason,
        other => panic!("expected a block, got {other:?}"),
    }
}
fn context(d: &Decision) -> &str {
    match d {
        Decision::Context { text, .. } => text,
        other => panic!("expected additionalContext, got {other:?}"),
    }
}

#[test]
fn replay_a18_self_reported_checks_cannot_mark_ready() {
    let d = dir("a18");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let r = handle(
        &handback("w1", "VERDICT: PASS\nCHECKS RUN: 0", false),
        &mut env,
    );
    assert!(blocked(&r).contains("refusal 1 of 3"));
    assert!(blocked(&r).contains("missing C1"));
    let r = handle(&stop("Done, the task is complete.", false), &mut env);
    assert!(blocked(&r).contains("missing"), "{r:?}");
    assert_eq!(gate(&env)["state"], "verifying");
}

#[test]
fn replay_a19_changed_candidate_makes_receipts_stale() {
    let d = dir("a19");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(worker(&mut env, "w1", "PASS", &ALL4), Decision::None);
    assert_eq!(gate(&env)["state"], "ready_for_review");
    env.candidate = fake("two", &["src/lib.rs"]);
    let r = handle(&stop("Done.", false), &mut env);
    assert!(blocked(&r).contains("stale C1"), "{r:?}");
    let g = gate(&env);
    assert_eq!(g["state"], "verifying");
    assert_eq!(g["candidate"]["dirty_digest"], "two");
    assert_eq!(g["receipts"].as_array().unwrap().len(), 4);
}

#[test]
fn replay_review_survives_older_candidate_receipts() {
    // L4 live run: fresh receipts on the new candidate reached review, then the next hook
    // call replayed the older candidate's receipts first and dropped review to verifying.
    let d = dir("review-restore");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(worker(&mut env, "w1", "PASS", &ALL4), Decision::None);
    env.candidate = fake("two", &["src/lib.rs"]);
    let r = handle(&stop("Done.", false), &mut env);
    assert!(blocked(&r).contains("stale C1"), "{r:?}");
    assert_eq!(worker(&mut env, "w2", "PASS", &ALL4), Decision::None);
    assert_eq!(gate(&env)["state"], "ready_for_review");
    let r = handle(
        &template(FOREGROUND, "PostToolUse", Some("Agent")),
        &mut env,
    );
    assert!(context(&r).contains("decision: accepted"), "{r:?}");
    assert_eq!(gate(&env)["state"], "ready_for_review");
    assert_eq!(handle(&stop("Done.", false), &mut env), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn replay_a20_resume_keeps_state_continuations_candidate_and_receipts() {
    let d = dir("a20");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let r = worker(&mut env, "w1", "PASS", &ALL4[..3]);
    assert!(blocked(&r).contains("missing C4"));
    let r = handle(&stop("Done.", false), &mut env);
    assert!(blocked(&r).contains("refusal 1 of 3"));
    drop(env);

    let mut env = env_at(&d, fake("one", &["src/lib.rs"]));
    let r = handle(
        &template(FOREGROUND, "PostToolUse", Some("Agent")),
        &mut env,
    );
    assert_eq!(
        context(&r),
        "GATE: state verifying; current receipts: [C1, C2, C3]; missing: [C4]; stale: []; decision: not accepted (missing C4)"
    );
    let g = gate(&env);
    assert_eq!(g["state"], "verifying");
    assert_eq!(g["continuations"], 1);
    assert_eq!(
        g["candidate"],
        json!({"base_commit": "base", "dirty_digest": "one"})
    );
    assert_eq!(g["receipts"].as_array().unwrap().len(), 3);
    let r = handle(&stop("Done.", true), &mut env);
    assert!(blocked(&r).contains("refusal 2 of 3"));
}

#[test]
fn replay_c1_c4_all_four_checks_needed_then_completion_allowed() {
    let d = dir("c1c4");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let r = worker(&mut env, "w1", "PASS", &ALL4[..3]);
    assert!(blocked(&r).contains("C4"), "{r:?}");
    assert_eq!(worker(&mut env, "w2", "PASS", &ALL4), Decision::None);
    let r = handle(
        &stop("All checks pass and the task is complete.", false),
        &mut env,
    );
    assert_eq!(r, Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let r = handle(&pre("Write"), &mut env);
    assert_eq!(
        r.to_json().unwrap(),
        json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "GATE: Write refused: task is completed"}})
    );
}

#[test]
fn replay_r26_failing_mandatory_check_refuses_completion() {
    let d = dir("r26");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true), ("C2", false)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    let r = worker(&mut env, "w1", "FAIL", &[("C1", "cargo test", 1)]);
    assert_eq!(r, Decision::None);
    let r = handle(&stop("Done.", false), &mut env);
    assert!(blocked(&r).contains("failing C1 exit 1"), "{r:?}");
}

#[test]
fn replay_r28_out_of_scope_path_refuses() {
    let d = dir("r28");
    let mut env = env(
        &d,
        Some(four()),
        fake("one", &["src/lib.rs", "docs/out.md"]),
    );
    let r = worker(&mut env, "w1", "PASS", &ALL4);
    assert!(blocked(&r).contains("out of scope docs/out.md"), "{r:?}");
    let r = handle(&stop("Done.", false), &mut env);
    assert!(blocked(&r).contains("out of scope"), "{r:?}");
}

#[test]
fn replay_r35_would_run_is_not_run() {
    let d = dir("r35");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let message = "VERDICT: PASS\nCHECKS RUN: 0\nI would run cargo test.";
    let r = handle(&handback("w1", message, false), &mut env);
    assert!(blocked(&r).contains("missing C1"), "{r:?}");
    let r = handle(&stop(message, false), &mut env);
    assert!(blocked(&r).contains("missing C1"), "{r:?}");
}

#[test]
fn replay_x1_check_line_without_exit_is_a_parse_failure() {
    let d = dir("x1");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    handle(&bash("w1", "cargo test", 0), &mut env);
    let r = handle(
        &handback(
            "w1",
            "VERDICT: PASS\nCHECKS RUN: 1\nCHECK C1 | cargo test",
            false,
        ),
        &mut env,
    );
    assert!(
        blocked(&r).contains("CHECKS RUN 1 but has 0 well-formed CHECK lines"),
        "{r:?}"
    );
}

#[test]
fn replay_x2_run_by_another_agent_does_not_link() {
    let d = dir("x2");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    handle(&bash("A", "cargo test", 0), &mut env);
    let r = handle(
        &handback(
            "B",
            "VERDICT: PASS\nCHECKS RUN: 1\nCHECK C1 | cargo test | exit 0",
            false,
        ),
        &mut env,
    );
    assert!(blocked(&r).contains("unlinked CHECK C1"), "{r:?}");
    assert!(gate(&env)["receipts"].as_array().unwrap().is_empty());
}

#[test]
fn replay_gap1_check_citing_another_command_is_refused() {
    let d = dir("gap1");
    let mut c2 = contract(&[("C2", true)], &["src/"]);
    c2["criteria"][0]["command"] = json!("echo check-ok");
    let mut env = env(&d, Some(c2), fake("one", &["src/lib.rs"]));
    let r = worker(&mut env, "w1", "PASS", &[("C2", "echo ok", 0)]);
    assert_eq!(
        blocked(&r),
        "GATE: handback refused (refusal 1 of 3): C2 cites `echo ok` but the contract requires `echo check-ok`; missing C2. Run that command in this session and cite it as CHECK C2 | echo check-ok | exit 0."
    );
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(
        line["receipts"],
        json!({"recorded": [], "rejected": [], "unlinked": ["C2"]})
    );
    assert!(
        line["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().starts_with(
                "unlinked CHECK C2 | echo ok | exit 0 (claim_supported false: C2 cites"
            )),
        "{line}"
    );
    assert!(gate(&env)["receipts"].as_array().unwrap().is_empty());
    assert_eq!(gate(&env)["state"], "verifying");
}

#[test]
fn contract_without_a_command_fails_open_naming_the_criterion() {
    let d = dir("no-command");
    let mut c = contract(&[("C2", true)], &["src/"]);
    c["criteria"][0].as_object_mut().unwrap().remove("command");
    let mut env = env(&d, Some(c), fake("one", &["src/lib.rs"]));
    let r = handle(&pre("Write"), &mut env);
    assert!(
        matches!(r, Decision::System(ref m) if m.starts_with("GATE adapter error: ")
            && m.ends_with("criterion C2 has no command; name the command a receipt for it must cite")),
        "{r:?}"
    );
    assert!(!env.state_dir.join("gate.json").exists());
}

#[test]
fn handback_refusals_release_after_three() {
    let d = dir("cap");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let message = "VERDICT: PASS\nCHECKS RUN: 0";
    for k in 1..=3 {
        let r = handle(&handback("w1", message, k > 1), &mut env);
        assert!(blocked(&r).contains(&format!("refusal {k} of 3")));
    }
    // Silent release: any output on SubagentStop keeps the worker running (L1 live run).
    let r = handle(&handback("w1", message, true), &mut env);
    assert_eq!(r, Decision::None);
    let journal = std::fs::read_to_string(env.state_dir.join("events.jsonl")).unwrap();
    assert!(journal.contains("UNVERIFIED after 3 refusals"), "{journal}");
}

#[test]
fn unobserved_candidate_denies_writes() {
    let d = dir("unobserved");
    let mut env = env(&d, Some(four()), Box::new(Unobservable));
    let r = handle(&pre("Edit"), &mut env);
    assert_eq!(
        r,
        Decision::Deny(
            "GATE: Edit refused: candidate identity could not be observed (git rev-parse HEAD failed)"
                .into()
        )
    );
}

#[test]
fn no_contract_is_advisory_only() {
    let d = dir("nocontract");
    let mut env = env(&d, None, fake("one", &[]));
    assert_eq!(handle(&stop("Done.", false), &mut env), Decision::None);
    let r = handle(
        &template(BACKGROUND, "PostToolUse", Some("Agent")),
        &mut env,
    );
    assert!(context(&r).starts_with("GATE (no contract):"));
    assert!(!env.state_dir.join("gate.json").exists());
}

#[test]
fn journal_has_one_line_per_handled_event() {
    let d = dir("journal");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    let mut decisions = Vec::new();
    for file in [FOREGROUND, BACKGROUND] {
        for payload in captured(file) {
            decisions.push(handle(&payload, &mut env).to_json());
        }
    }
    let journal = fs::read_to_string(env.state_dir.join("events.jsonl")).unwrap();
    let lines: Vec<Value> = journal
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), decisions.len());
    for (line, decision) in lines.iter().zip(&decisions) {
        let (kind, text) = decision_parts(decision.as_ref().unwrap_or(&Value::Null));
        assert_eq!(line["decision"], json!({"kind": kind, "text": text}));
        assert!(line["event"].is_string() && line["t"] == 1_000 && line["v"] == 1);
    }
    let async_line = &lines[decisions.len() - 6];
    assert_eq!(async_line["event"], "PostToolUse");
    assert!(
        async_line["decision"]["text"]
            .as_str()
            .unwrap()
            .contains("background dispatch")
    );
    // No prompt text, worker message or tool input is copied.
    for leaked in [
        "dispatch the fake-worker",
        "Verify the fix in candidate.txt",
        "candidate content v2",
        "The gate is right to refuse",
    ] {
        assert!(!journal.contains(leaked), "{leaked}");
    }
}

#[test]
fn journal_v1_fields() {
    let d = dir("journal-v1");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(worker(&mut env, "w1", "PASS", &ALL4), Decision::None);
    handle(
        &template(FOREGROUND, "PostToolUse", Some("Agent")),
        &mut env,
    );
    assert_eq!(handle(&stop("Done.", false), &mut env), Decision::None);
    handle(&stop("Done.", false), &mut env);
    let r = handle(&pre("Write"), &mut env);
    assert!(matches!(r, Decision::Deny(_)));
    let journal = fs::read_to_string(env.state_dir.join("events.jsonl")).unwrap();
    let lines: Vec<Value> = journal
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let keys = [
        "v",
        "t",
        "session_id",
        "prompt_id",
        "task_id",
        "state_location",
        "candidate",
        "event",
        "tool_name",
        "agent_id",
        "agent_type",
        "decision",
        "reason",
        "reasons",
        "receipts",
        "header",
        "gate_state",
        "accepted",
        "tampered",
        "usage",
    ];
    for line in &lines {
        let mut got: Vec<&str> = line
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        got.sort();
        let mut want = keys.to_vec();
        want.sort();
        assert_eq!(got, want);
        assert_eq!(line["task_id"], "t4-test");
        assert_eq!(
            line["candidate"],
            json!({"base_commit": "base", "dirty_digest": "one"})
        );
    }
    let [handback, agent, first_stop, second_stop, deny] = &lines[lines.len() - 5..] else {
        unreachable!()
    };
    assert_eq!(handback["event"], "SubagentStop");
    assert_eq!(
        handback["receipts"]["recorded"],
        json!(["C1", "C2", "C3", "C4"])
    );
    assert_eq!(
        handback["header"],
        json!({"verdict": "PASS", "checks_run": 4, "lines": 4, "consistent": true})
    );
    assert_eq!(handback["gate_state"], "ready_for_review");
    assert_eq!(agent["usage"]["total_tokens"], 7086);
    assert_eq!(agent["usage"]["model"], "claude-haiku-5-5");
    assert_eq!(
        (&first_stop["accepted"], &first_stop["gate_state"]),
        (&json!(true), &json!("completed"))
    );
    // Only the call that completed the task counts as acceptance.
    assert_eq!(second_stop["accepted"], false);
    assert_eq!(deny["decision"]["kind"], "deny");
    assert_eq!(deny["reason"], "task is completed");
    assert_eq!(deny["tampered"], false);
}

fn edit_gate(env: &Env, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut g = gate(env);
    change(&mut g);
    fs::write(env.state_dir.join("gate.json"), g.to_string()).unwrap();
    fs::read(env.state_dir.join("gate.json")).unwrap()
}
fn last_reasons(env: &Env) -> String {
    let journal = fs::read_to_string(env.state_dir.join("events.jsonl")).unwrap();
    journal.lines().last().unwrap().to_owned()
}

#[test]
fn tampered_receipt_refuses_and_leaves_gate_untouched() {
    let d = dir("tamper-receipt");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(worker(&mut env, "w1", "PASS", &ALL4), Decision::None);
    assert_eq!(gate(&env)["state"], "ready_for_review");
    let edited = edit_gate(&env, |g| g["receipts"][0]["exit_code"] = json!(1));
    let r = handle(&stop("Done.", false), &mut env);
    let reason = blocked(&r);
    assert!(
        reason.contains("tampered") && reason.contains("receipt 0"),
        "{reason}"
    );
    assert_eq!(fs::read(env.state_dir.join("gate.json")).unwrap(), edited);
    assert!(last_reasons(&env).contains("state tampered: receipt 0"));
    // Every later call still sees it.
    let r = handle(&pre("Agent"), &mut env);
    assert!(matches!(r, Decision::Deny(ref m) if m.contains("tampered (receipt 0)")));
    assert!(last_reasons(&env).contains("state tampered: receipt 0"));
    // One block per prompt: when the harness is already continuing, release visibly.
    let r = handle(&stop("Done.", true), &mut env);
    assert!(
        matches!(r, Decision::System(ref m) if m.contains("tampered (receipt 0)")),
        "{r:?}"
    );
    assert_eq!(fs::read(env.state_dir.join("gate.json")).unwrap(), edited);
}

#[test]
fn tampered_checkpoint_denies_writes() {
    let d = dir("tamper-checkpoint");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    handle(&pre("Write"), &mut env);
    edit_gate(&env, |g| g["state"] = json!("completed"));
    let r = handle(&pre("Write"), &mut env);
    assert!(
        matches!(r, Decision::Deny(ref m) if m.contains("tampered (checkpoint)")),
        "{r:?}"
    );
}

#[test]
fn contract_edit_after_first_call_trips_the_chain() {
    let d = dir("tamper-contract");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    handle(&pre("Write"), &mut env);
    let wider = contract(
        &[("C1", true), ("C2", true), ("C3", true), ("C4", true)],
        &["src/", "tests/", "docs/"],
    );
    fs::write(env.state_dir.join("contract.json"), wider.to_string()).unwrap();
    let r = handle(&pre("Write"), &mut env);
    assert!(
        matches!(r, Decision::Deny(ref m) if m.contains("tampered (checkpoint)")),
        "{r:?}"
    );
}

#[test]
fn every_journal_line_names_the_state_location() {
    let d = dir("location-journal");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    handle(&pre("Write"), &mut env);
    assert!(last_reasons(&env).contains("\"state_location\":\"test\""));
}

#[test]
fn mcp_asserted_receipt_does_not_let_the_chair_stop() {
    let d = dir("mcp-asserted");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    with_asserted_receipt(&mut env, false);

    // No acceptance decision at PreToolUse, so nothing is journaled as unlinked there.
    handle(&pre("Write"), &mut env);
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(line["event"], "PreToolUse");
    assert_eq!(line["receipts"]["unlinked"], json!([]));

    let r = handle(&stop("Done, the task is complete.", false), &mut env);
    assert!(blocked(&r).contains("missing C1"), "{r:?}");
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(line["event"], "Stop");
    assert_eq!(line["receipts"]["unlinked"], json!(["C1"]));
    assert_eq!(line["tampered"], false);
    let g = gate(&env);
    assert_eq!(g["state"], "verifying");
    assert_eq!(g["receipts"].as_array().unwrap().len(), 1);
}

/// gate.json with one MCP-asserted receipt for C1 on the current candidate, written after a
/// first hook call created the gate. `completed` also sets the saved state to completed.
fn with_asserted_receipt(env: &mut Env, completed: bool) {
    handle(&pre("Write"), env);
    let (_, digest) = read_contract(&env.state_dir).unwrap();
    let Stored::Valid(mut g) = load_gate(&env.state_dir, &digest).unwrap() else {
        panic!("gate.json not valid after the first call");
    };
    g.receipts.push(VerificationReceipt {
        criterion: CriterionId("C1".into()),
        candidate: g.checkpoint.candidate.clone().unwrap(),
        command: "cargo test".into(),
        cwd: ".".into(),
        exit_code: Some(0),
        output_ref: format!("{MCP_ASSERTED}run-1"),
        claim_supported: true,
        started_at: 1,
        finished_at: 1,
    });
    if completed {
        g.checkpoint.state = TaskState::Completed;
    }
    save_gate(&env.state_dir, &g, &digest).unwrap();
}

#[test]
fn task_completed_through_mcp_does_not_let_the_chair_stop() {
    let d = dir("mcp-completed");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    with_asserted_receipt(&mut env, true);
    let before = gate(&env);
    let r = handle(&stop("Done, the task is complete.", false), &mut env);
    assert!(blocked(&r).contains("state completed; missing C1"), "{r:?}");
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(line["receipts"]["unlinked"], json!(["C1"]));
    assert_eq!(line["accepted"], false);
    let after = gate(&env);
    for field in [
        "state",
        "candidate",
        "receipts",
        "open_findings",
        "consumed_repair_rounds",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    assert_eq!(after["state"], "completed");
    // The refusal is counted towards the continuation cap, as for every Stop refusal.
    assert_eq!(after["continuations"], 1);
}

#[test]
fn task_completed_through_mcp_does_not_pass_a_handback() {
    let d = dir("mcp-completed-handback");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    with_asserted_receipt(&mut env, true);
    let r = handle(
        &handback(
            "w1",
            "VERDICT: PASS
CHECKS RUN: 0",
            false,
        ),
        &mut env,
    );
    assert!(blocked(&r).contains("missing C1"), "{r:?}");
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(line["receipts"]["unlinked"], json!(["C1"]));
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn task_completed_with_observed_receipts_still_stops() {
    let d = dir("observed-completed");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(worker(&mut env, "w1", "PASS", &ALL4), Decision::None);
    assert_eq!(handle(&stop("Done.", false), &mut env), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    assert_eq!(handle(&stop("Done.", false), &mut env), Decision::None);
}

/// The chair runs each line itself (agent `main`) unless `skip_runs`, then stops citing all of them.
fn chair_stop(env: &mut Env, lines: &[(&str, &str, i32)], skip_runs: usize) -> Decision {
    for (_, command, exit) in &lines[skip_runs..] {
        assert_eq!(handle(&bash("main", command, *exit), env), Decision::None);
    }
    let cited: Vec<String> = lines
        .iter()
        .map(|(id, command, exit)| format!("CHECK {id} | {command} | exit {exit}"))
        .collect();
    let message = format!(
        "VERDICT: PASS\nCHECKS RUN: {}\n{}\n\nThe task is complete.",
        lines.len(),
        cited.join("\n")
    );
    handle(&stop(&message, false), env)
}

#[test]
fn chair_stop_links_check_lines_to_its_own_runs_and_completes() {
    let d = dir("chair-stop");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    assert_eq!(chair_stop(&mut env, &ALL4, 0), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let line: Value = serde_json::from_str(&last_reasons(&env)).unwrap();
    assert_eq!(line["event"], "Stop");
    assert_eq!(
        line["receipts"]["recorded"],
        json!(["C1", "C2", "C3", "C4"])
    );
    assert_eq!(line["receipts"]["unlinked"], json!([]));
}

#[test]
fn chair_stop_citing_a_run_that_was_not_journaled_is_refused() {
    let d = dir("chair-stop-unjournaled");
    let mut env = env(&d, Some(four()), fake("one", &["src/lib.rs"]));
    // C1 is cited but never ran.
    let r = chair_stop(&mut env, &ALL4, 1);
    let text = blocked(&r);
    assert!(
        text.starts_with("GATE refuses completion (refusal 1 of 3, harness cap 8): state verifying; unlinked CHECK C1 | cargo test c1 | exit 0 (no run by agent main with that command and exit code); record the check with CHECK lines"),
        "{text}"
    );
    assert_eq!(gate(&env)["state"], "verifying");
    // A worker's run does not link to the chair's CHECK line either.
    assert_eq!(
        handle(&bash("w1", "cargo test c1", 0), &mut env),
        Decision::None
    );
    assert!(blocked(&chair_stop(&mut env, &ALL4[..1], 1)).contains("unlinked CHECK C1"));
}

#[test]
fn chair_stop_citing_another_command_names_the_contract_command() {
    let d = dir("chair-stop-gap1");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    let r = chair_stop(&mut env, &[("C1", "cargo test c1; echo done", 0)], 0);
    let text = blocked(&r);
    assert!(
        text.contains("C1 cites `cargo test c1; echo done` but the contract requires `cargo test c1` or one of: `cargo test`"),
        "{text}"
    );
    assert!(
        text.ends_with(
            "Run that command in this session and cite it as CHECK C1 | cargo test c1 | exit 0."
        ),
        "{text}"
    );
    assert_eq!(gate(&env)["state"], "verifying");
}

/// The live sequence of the first real-project use: the chair completed the task through MCP
/// on asserted receipts, then stopped citing runs the hooks had journaled.
#[test]
fn chair_stop_after_mcp_completion_counts_its_own_runs() {
    let d = dir("chair-stop-after-mcp");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    with_asserted_receipt(&mut env, true);
    assert_eq!(
        chair_stop(&mut env, &[("C1", "cargo test c1", 0)], 0),
        Decision::None
    );
    let g = gate(&env);
    assert_eq!(g["state"], "completed");
    assert_eq!(g["receipts"].as_array().unwrap().len(), 2);
}

const C1_HANDBACK: &str = "VERDICT: PASS\nCHECKS RUN: 1\nCHECK C1 | cargo test | exit 0";

#[test]
fn gap2_run_before_the_last_change_is_refused_and_a_new_run_is_accepted() {
    let d = dir("gap2-rerun");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    assert_eq!(
        handle(&bash("w1", "cargo test", 0), &mut env),
        Decision::None
    );
    let line = fs::read_to_string(env.state_dir.join("runs.jsonl")).unwrap();
    let run: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        run["candidate"],
        json!({"base_commit": "base", "dirty_digest": "one"})
    );
    // The worker edits a file after the run: the candidate changes.
    env.candidate = fake("two", &["src/lib.rs"]);
    let r = handle(&handback("w1", C1_HANDBACK, false), &mut env);
    let text = blocked(&r);
    assert!(
        text.contains("C1 `cargo test` ran before your last change; run it again"),
        "{text}"
    );
    assert!(!text.contains("re-cite"), "{text}");
    assert!(gate(&env)["receipts"].as_array().unwrap().is_empty());
    assert_eq!(gate(&env)["state"], "verifying");
    // Run again on the current candidate, then hand back: accepted.
    let mut again = bash("w1", "cargo test", 0);
    again["tool_use_id"] = json!("toolu_w1_rerun");
    assert_eq!(handle(&again, &mut env), Decision::None);
    assert_eq!(
        handle(&handback("w1", C1_HANDBACK, false), &mut env),
        Decision::None
    );
    assert_eq!(gate(&env)["state"], "ready_for_review");
}

#[test]
fn gap2_run_record_without_a_candidate_never_links() {
    let d = dir("gap2-legacy");
    let mut env = env(
        &d,
        Some(contract(&[("C1", true)], &["src/"])),
        fake("one", &["src/lib.rs"]),
    );
    // A record written before runs carried the candidate.
    let legacy = json!({"agent_id": "w1", "tool": "Bash", "command": "cargo test",
        "cwd": "x", "exit_code": 0, "tool_use_id": "toolu_legacy", "t": 1});
    fs::write(env.state_dir.join("runs.jsonl"), format!("{legacy}\n")).unwrap();
    let r = handle(&handback("w1", C1_HANDBACK, false), &mut env);
    assert!(blocked(&r).contains("run it again"), "{r:?}");
    assert!(gate(&env)["receipts"].as_array().unwrap().is_empty());
}
