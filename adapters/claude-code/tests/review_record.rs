//! WP5: the reviewer record. A reviewer's final message starts with `REVIEW: APPROVED` or
//! `REVIEW: REJECTED` (with `FINDING:` lines below); the gate stores reviewer, model, decision,
//! findings, time, candidate and provenance. A review from an agent that changed something in the
//! task, or from the chair, is `caller-asserted` and does not count as independent. Replay of hook
//! payloads, not a live harness run.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::{CandidateProvider, GitCandidate};
use supervisor_claude_code::level::{Level, Setting};
use supervisor_claude_code::state::{self, Stored};
use supervisor_claude_code::{autonomy, lifecycle};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!(
        "supervisor-review-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("src")).unwrap();
    Dir(path)
}

/// A candidate whose dirty digest the test can move, as an edit would.
struct Switch(Arc<Mutex<&'static str>>);
impl CandidateProvider for Switch {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: (*self.0.lock().unwrap()).into(),
            },
            provenance: "fake".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}

/// A person's v2 contract with one mandatory check, `cargo test`, and `review`.
fn contract(review: Value) -> Value {
    json!({"schema_version":2,"id":"wp5","goal":"review","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,"base_sha":null,
        "approval":{"by":"person","at":1000,"via":"task-start"},
        "limits":{},"artifacts":[],"review":review,"isolation":"none",
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]})
}
fn required() -> Value {
    json!({"required":true,"independent":false})
}
fn independent() -> Value {
    json!({"required":true,"independent":true})
}

fn env_with(d: &Dir, contract: &Value, candidate: Box<dyn CandidateProvider>) -> Env {
    let state = d.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("contract.json"), format!("{contract}\n")).unwrap();
    Env {
        state_dir: state,
        state_location: "test",
        candidate,
        clock: || 1_000,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    }
}
fn setup(d: &Dir, contract: &Value) -> (Env, Arc<Mutex<&'static str>>) {
    let digest = Arc::new(Mutex::new("one"));
    let env = env_with(d, contract, Box::new(Switch(digest.clone())));
    (env, digest)
}
fn cwd(env: &Env) -> PathBuf {
    env.state_dir.parent().unwrap().to_path_buf()
}
fn tool(env: &Env, agent: &str, id: &str, name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","cwd":cwd(env),"session_id":"session","agent_id":agent,
        "agent_type":"general-purpose","tool_use_id":id,"tool_name":name,"tool_input":input})
}
/// `agent` edits a file in scope: from now on it is a builder of this task.
fn write(env: &mut Env, agent: &str) {
    let path = cwd(env).join("src").join("lib.rs");
    let id = format!("w-{agent}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let p = tool(
        env,
        agent,
        &id,
        "Write",
        json!({"file_path":path,"content":"x"}),
    );
    assert_eq!(handle(&p, env), Decision::None);
}
/// `agent` only reads.
fn read(env: &mut Env, agent: &str) {
    let path = cwd(env).join("src").join("lib.rs");
    let id = format!("r-{agent}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let p = tool(env, agent, &id, "Read", json!({"file_path":path}));
    assert_eq!(handle(&p, env), Decision::None);
}
/// `agent` runs `command` in the foreground: PreToolUse, then its result.
fn run(env: &mut Env, agent: &str, id: &str, command: &str, exit: i32) {
    let mut p = tool(env, agent, id, "Bash", json!({"command":command}));
    assert_eq!(handle(&p, env), Decision::None);
    if exit == 0 {
        p["hook_event_name"] = "PostToolUse".into();
        p["tool_response"] = json!({"stdout":"ok","stderr":"","interrupted":false});
    } else {
        p["hook_event_name"] = "PostToolUseFailure".into();
        p["error"] = format!("Exit code {exit}\nfailed").into();
    }
    handle(&p, env);
}
fn checks_pass(env: &mut Env, id: &str) {
    run(env, "main", id, "cargo test", 0);
}
fn subagent_stop(env: &mut Env, agent: &str, message: &str) -> Decision {
    handle(
        &json!({"hook_event_name":"SubagentStop","cwd":cwd(env),"session_id":"session",
            "agent_id":agent,"agent_type":"general-purpose","stop_hook_active":false,
            "last_assistant_message":message}),
        env,
    )
}
/// A reader dispatched as a reviewer: it reads, then hands back `message`.
fn reviewer(env: &mut Env, agent: &str, message: &str) -> Decision {
    read(env, agent);
    subagent_stop(env, agent, message)
}
fn stop(env: &mut Env, message: &str) -> Decision {
    handle(
        &json!({"hook_event_name":"Stop","cwd":cwd(env),"session_id":"session",
            "stop_hook_active":false,"last_assistant_message":message}),
        env,
    )
}
/// The Agent tool's PostToolUse for `agent`: `completed` (foreground) or `async_launched`.
fn agent_post(env: &mut Env, agent: &str, status: &str, model: &str) -> Decision {
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":cwd(env),"session_id":"session",
            "tool_name":"Agent","tool_use_id":format!("a-{agent}"),
            "tool_input":{"description":"review","prompt":"review the change"},
            "tool_response":{"status":status,"agentId":agent,"resolvedModel":model,
                "content":[{"type":"text","text":"REVIEW: APPROVED"}]}}),
        env,
    )
}
fn task_dir(env: &Env) -> PathBuf {
    lifecycle::active_task(&env.state_dir)
        .unwrap()
        .map_or_else(|| env.state_dir.clone(), |task| task.dir)
}
fn gate(env: &Env) -> Value {
    serde_json::from_slice(&fs::read(task_dir(env).join("gate.json")).unwrap()).unwrap()
}
fn reviews(env: &Env) -> Vec<Value> {
    gate(env)["reviews"].as_array().cloned().unwrap_or_default()
}
fn blocked(d: &Decision) -> &str {
    match d {
        Decision::Block(reason) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}
fn last_reasons(env: &Env) -> String {
    let journal = fs::read_to_string(task_dir(env).join("events.jsonl")).unwrap();
    let last: Value = serde_json::from_str(journal.lines().last().unwrap()).unwrap();
    last["reasons"].to_string()
}

/// No new text names something absent with the words the fail-closed rewording avoids.
fn plain(text: &str) {
    for word in ["missing", "not found", "no such file"] {
        assert!(!text.to_lowercase().contains(word), "{word:?} in {text}");
    }
}

#[test]
fn required_review_with_none_refuses_and_teaches_the_protocol() {
    let d = dir("required");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    let r = stop(&mut env, "Done.");
    let why = blocked(&r);
    assert!(why.contains("REVIEW: APPROVED"), "{why}");
    assert!(why.contains("REVIEW: REJECTED"), "{why}");
    assert!(why.contains("only reads"), "{why}");
    assert!(
        why.contains("supervisor task cancel --project <dir> --task wp5 --reason <text>"),
        "a person's way out: {why}"
    );
    assert!(!why.contains("run the required check commands"), "{why}");
    plain(why);
    assert_eq!(gate(&env)["state"], "verifying");
}

/// Spec matrix case 11: a rejection prevents approval; a second reviewer cannot override it on
/// the same content; a content change sheds it, and then a fresh review is needed.
#[test]
fn a_rejection_by_a_reader_refuses_until_the_content_changes() {
    let d = dir("rejected");
    let (mut env, digest) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    let r = reviewer(
        &mut env,
        "r1",
        "REVIEW: REJECTED\nFINDING: off by one in the loop\nFINDING: no test for empty input",
    );
    assert_eq!(
        r,
        Decision::None,
        "a reviewer's SubagentStop prints nothing"
    );
    let stored = reviews(&env);
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0]["reviewer"], "r1");
    assert_eq!(stored[0]["decision"], "rejected");
    assert_eq!(stored[0]["provenance"], "observed");
    assert_eq!(
        stored[0]["findings"],
        json!(["off by one in the loop", "no test for empty input"])
    );
    assert_eq!(stored[0]["at"], 1_000);
    assert_eq!(stored[0]["candidate"]["dirty_digest"], "one");
    let reasons = last_reasons(&env);
    assert!(
        !reasons.contains("off by one"),
        "no finding text in the journal: {reasons}"
    );
    let r = stop(&mut env, "Done.");
    let why = blocked(&r);
    assert!(why.contains("rejected"), "{why}");
    assert!(why.contains("finding"), "{why}");
    plain(why);
    assert_eq!(reviewer(&mut env, "r2", "REVIEW: APPROVED"), Decision::None);
    assert!(blocked(&stop(&mut env, "Done.")).contains("rejected"));
    assert_ne!(gate(&env)["state"], "completed");
    *digest.lock().unwrap() = "two";
    checks_pass(&mut env, "c2");
    let r = stop(&mut env, "Done.");
    let why = blocked(&r);
    assert!(
        !why.contains("rejected"),
        "the rejection was of other content: {why}"
    );
    assert!(why.contains("REVIEW: APPROVED"), "{why}");
    assert_eq!(reviewer(&mut env, "r3", "REVIEW: APPROVED"), Decision::None);
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// Spec matrix case 12: a timeout or inconclusive answer is not a decision; nothing is stored
/// and approval stays refused.
#[test]
fn an_inconclusive_or_empty_review_stores_nothing_and_still_refuses() {
    let d = dir("inconclusive");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    for (agent, message) in [
        ("r1", "REVIEW: INCONCLUSIVE\nFINDING: could not finish"),
        ("r2", "REVIEW:"),
        ("r3", "REVIEW: timed out after 600 s"),
        ("r4", "REVIEW: approved"),
    ] {
        assert_eq!(reviewer(&mut env, agent, message), Decision::None);
        assert!(
            last_reasons(&env).contains("not a decision"),
            "{agent}: {}",
            last_reasons(&env)
        );
    }
    assert!(gate(&env).get("reviews").is_none(), "{}", gate(&env));
    assert!(blocked(&stop(&mut env, "Done.")).contains("REVIEW: APPROVED"));
}

#[test]
fn a_line_before_the_review_line_is_not_a_review() {
    let d = dir("first-line");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "Looks fine to me.\nREVIEW: APPROVED");
    assert!(reviews(&env).is_empty());
}

#[test]
fn an_approval_by_a_reader_completes_and_is_stored_observed() {
    let d = dir("approved");
    let (mut env, _) = setup(&d, &contract(independent()));
    checks_pass(&mut env, "c1");
    assert_eq!(
        reviewer(
            &mut env,
            "r1",
            "\n  REVIEW: APPROVED  \nFINDING: naming could be clearer"
        ),
        Decision::None
    );
    let stored = reviews(&env);
    assert_eq!(stored[0]["provenance"], "observed");
    assert_eq!(stored[0]["model"], Value::Null);
    assert_eq!(stored[0]["findings"], json!(["naming could be clearer"]));
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// A builder's own review, with its normal handback header below it, is caller-asserted: it
/// satisfies `required` and not `independent`.
#[test]
fn a_builders_review_is_caller_asserted() {
    for (policy, completes) in [(required(), true), (independent(), false)] {
        let d = dir("builder");
        let (mut env, _) = setup(&d, &contract(policy.clone()));
        write(&mut env, "w1");
        run(&mut env, "w1", "w1-c", "cargo test", 0);
        let r = subagent_stop(
            &mut env,
            "w1",
            "REVIEW: APPROVED\nVERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | cargo test | exit 0",
        );
        assert_eq!(
            r,
            Decision::None,
            "{policy}: a review error is journaled, not refused"
        );
        let stored = reviews(&env);
        assert_eq!(stored[0]["provenance"], "caller-asserted", "{policy}");
        assert_eq!(stored[0]["reviewer"], "w1");
        let r = stop(&mut env, "Done.");
        if completes {
            assert_eq!(r, Decision::None, "{policy}");
            assert_eq!(gate(&env)["state"], "completed");
        } else {
            let why = blocked(&r);
            assert!(why.contains("independent"), "{why}");
            plain(why);
        }
    }
}

#[test]
fn the_chairs_review_is_caller_asserted() {
    for (policy, completes) in [(required(), true), (independent(), false)] {
        let d = dir("chair");
        let (mut env, _) = setup(&d, &contract(policy.clone()));
        checks_pass(&mut env, "c1");
        let r = stop(&mut env, "REVIEW: APPROVED\nDone, the task is complete.");
        let stored = reviews(&env);
        assert_eq!(stored[0]["reviewer"], "main", "{policy}");
        assert_eq!(stored[0]["provenance"], "caller-asserted", "{policy}");
        if completes {
            assert_eq!(r, Decision::None, "{policy}");
        } else {
            assert!(blocked(&r).contains("independent"), "{r:?}");
        }
    }
}

#[test]
fn independent_alone_behaves_as_required() {
    let d = dir("independent-only");
    let (mut env, _) = setup(&d, &contract(json!({"required":false,"independent":true})));
    checks_pass(&mut env, "c1");
    assert!(blocked(&stop(&mut env, "Done.")).contains("REVIEW: APPROVED"));
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    assert_eq!(stop(&mut env, "Done."), Decision::None);
}

#[test]
fn with_the_policy_off_reviews_are_stored_and_never_block() {
    let d = dir("off");
    let (mut env, _) = setup(&d, &contract(json!({"required":false,"independent":false})));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: REJECTED\nFINDING: needs work");
    assert_eq!(reviews(&env)[0]["decision"], "rejected");
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn the_same_review_twice_is_stored_once() {
    let d = dir("dedupe");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    reviewer(&mut env, "r1", "REVIEW: APPROVED\nFINDING: again");
    assert_eq!(reviews(&env).len(), 1);
}

#[test]
fn findings_are_capped() {
    let d = dir("cap");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    let long = "x".repeat(400);
    let mut message = "REVIEW: REJECTED".to_owned();
    for _ in 0..25 {
        message.push_str(&format!("\nFINDING: {long}"));
    }
    reviewer(&mut env, "r1", &message);
    let findings = reviews(&env)[0]["findings"].as_array().unwrap().clone();
    assert_eq!(findings.len(), 20);
    assert!(
        findings
            .iter()
            .all(|f| f.as_str().unwrap().chars().count() == 300)
    );
}

/// Foreground: SubagentStop arrives before the Agent tool's PostToolUse, which carries the
/// model; background: the launch carries it and SubagentStop comes later (Claude Code 2.1.312).
#[test]
fn the_model_comes_from_the_agent_tool() {
    let d = dir("model-fg");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    assert_eq!(reviews(&env)[0]["model"], Value::Null);
    agent_post(&mut env, "r1", "completed", "claude-haiku-5-5");
    assert_eq!(reviews(&env)[0]["model"], "claude-haiku-5-5");

    let d = dir("model-bg");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    agent_post(&mut env, "r2", "async_launched", "claude-opus-5-5");
    reviewer(&mut env, "r2", "REVIEW: APPROVED");
    assert_eq!(reviews(&env)[0]["model"], "claude-opus-5-5");
}

/// A resumed task gets its reviews back before the receipts are replayed, so review state is
/// not lost on the next unrelated hook call (the replay passes through an older candidate).
#[test]
fn a_reviewed_task_keeps_its_state_on_a_later_unrelated_call() {
    let d = dir("survives");
    let (mut env, digest) = setup(&d, &contract(required()));
    write(&mut env, "w1");
    run(&mut env, "w1", "w1-a", "cargo test", 0);
    subagent_stop(&mut env, "w1", "VERDICT: PASS\nCHECKS RUN: 0");
    *digest.lock().unwrap() = "two";
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    write(&mut env, "w2");
    run(&mut env, "w2", "w2-a", "cargo test", 0);
    assert_eq!(
        subagent_stop(&mut env, "w2", "VERDICT: PASS\nCHECKS RUN: 0"),
        Decision::None
    );
    assert_eq!(gate(&env)["state"], "ready_for_review");
    read(&mut env, "main");
    assert_eq!(gate(&env)["state"], "ready_for_review");
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// A builder's handback does not reach review without a counting review; the review error is
/// journaled and the handback is not refused for it.
#[test]
fn a_builder_handback_without_review_stays_verifying() {
    let d = dir("handback");
    let (mut env, _) = setup(&d, &contract(required()));
    write(&mut env, "w1");
    run(&mut env, "w1", "w1-a", "cargo test", 0);
    assert_eq!(
        subagent_stop(&mut env, "w1", "VERDICT: PASS\nCHECKS RUN: 0"),
        Decision::None
    );
    assert_eq!(gate(&env)["state"], "verifying");
    assert!(
        last_reasons(&env).contains("review"),
        "{}",
        last_reasons(&env)
    );
}

/// A rejection recorded while the task waits in review moves it back to verifying.
#[test]
fn a_rejection_in_review_moves_the_task_back_to_verifying() {
    let d = dir("back");
    let (mut env, _) = setup(&d, &contract(required()));
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    write(&mut env, "w1");
    run(&mut env, "w1", "w1-a", "cargo test", 0);
    subagent_stop(&mut env, "w1", "VERDICT: PASS\nCHECKS RUN: 0");
    assert_eq!(gate(&env)["state"], "ready_for_review");
    reviewer(&mut env, "r2", "REVIEW: REJECTED\nFINDING: wrong");
    assert_eq!(gate(&env)["state"], "verifying");
    assert!(blocked(&stop(&mut env, "Done.")).contains("rejected"));
}

/// A gate.json written before WP5 (no `reviews` key) still verifies.
#[test]
fn a_gate_written_before_reviews_verifies() {
    const CONTRACT: &str = r#"{"approval":{"at":1000,"by":"person","via":"task-start"},"artifacts":[],"base_sha":null,"criteria":[{"check":"test","command":"cargo test","id":"C2","mandatory":true}],"forbidden_effects":[],"goal":"old gate","id":"wp5-old","isolation":"none","limits":{},"max_repair_rounds":2,"review":{"independent":false,"required":true},"schema_version":2,"scope":["src/"]}"#;
    const GATE: &str = r#"{"candidate":{"base_commit":"base","dirty_digest":"one"},"checkpoint_hash":"12749df194674ac7cd2ba0ec551905314cd8a86bec6f5ea6e274a338a303f37c","consumed_repair_rounds":0,"continuations":0,"contract_id":"wp5-old","handback_refusals":{},"next_action":"","open_findings":[],"prompt_id":null,"receipts":[{"candidate":{"base_commit":"base","dirty_digest":"one"},"claim_supported":true,"command":"cargo test","criterion":"C2","cwd":"c:/temp/cs-capture","exit_code":0,"finished_at":1000,"hash":"8378b6eeee5f3ea6f80f17ea405b7b3247590fe91d6eae8834d0cf48bf47b687","output_ref":"observed:wp5-old:run","prev":"genesis","started_at":1000}],"starting_candidate":{"base_commit":"base","dirty_digest":"one"},"state":"completed"}"#;
    let d = dir("old-gate");
    let state_dir = d.0.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join("contract.json"), format!("{CONTRACT}\n")).unwrap();
    fs::write(state_dir.join("gate.json"), format!("{GATE}\n")).unwrap();
    let (_, digest) = state::read_contract(&state_dir).unwrap();
    assert!(matches!(
        state::load_gate(&state_dir, &digest).unwrap(),
        Stored::Valid(_)
    ));
    let mut env = env_with(
        &d,
        &serde_json::from_str(CONTRACT).unwrap(),
        Box::new(Fixed),
    );
    fs::write(state_dir.join("gate.json"), format!("{GATE}\n")).unwrap();
    read(&mut env, "main");
    assert!(
        !last_reasons(&env).contains("tampered"),
        "{}",
        last_reasons(&env)
    );
    assert!(gate(&env).get("reviews").is_none());
}

struct Fixed;
impl CandidateProvider for Fixed {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: "one".into(),
            },
            provenance: "fake".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}

/// WP4 interplay: at a reached turn limit a new reviewer cannot be dispatched, but Stop is
/// judged first, so evidence and a counting review still certify.
#[test]
fn at_a_reached_limit_a_review_already_recorded_still_certifies() {
    let d = dir("limit");
    let mut c = contract(required());
    c["limits"] = json!({"turns":2});
    let (mut env, _) = setup(&d, &c);
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    let dispatch = tool(&env, "main", "agent-1", "Agent", json!({"prompt":"review"}));
    assert!(
        matches!(handle(&dispatch, &mut env), Decision::Deny(why) if why.contains("task limit reached"))
    );
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

// ---- identical-tree transfer: real git -------------------------------------------------

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

/// A repository with a committed file, then modified; the state lives outside it.
fn repository(name: &str) -> (Dir, Env) {
    let d = dir(name);
    let project = d.0.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/a.rs"), "a1\n").unwrap();
    docs_only_readme(&project);
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    fs::write(project.join("src/a.rs"), "a2\n").unwrap();
    // The state directory is beside the project; payloads carry the project as cwd.
    let mut c = contract(required());
    c["scope"] = json!(["src/", "README.md"]);
    let env = env_with(&d, &c, Box::new(GitCandidate { cwd: project }));
    (d, env)
}

/// A README and a `.github/docs-paths.txt` naming it: CI's documentation-only list, which the
/// receipts' tree leaves out (G28) and the reviews' tree must not.
fn docs_only_readme(project: &Path) {
    fs::create_dir_all(project.join(".github")).unwrap();
    fs::write(project.join(".github/docs-paths.txt"), "README.md\n").unwrap();
    fs::write(project.join("README.md"), "one\n").unwrap();
}
fn project_of(env: &Env) -> PathBuf {
    env.state_dir.parent().unwrap().join("project")
}
fn in_project(env: &Env, mut payload: Value) -> Value {
    payload["cwd"] = json!(project_of(env));
    payload
}

#[test]
fn a_commit_of_identical_content_keeps_an_approval_and_a_rejection() {
    for decision in ["APPROVED", "REJECTED"] {
        let (_d, mut env) = repository("tree");
        let project = project_of(&env);
        let run = |env: &mut Env, kind: &str| {
            let mut p = in_project(
                env,
                json!({"hook_event_name":kind,"session_id":"session","agent_id":"main",
                    "tool_use_id":"run","tool_name":"Bash","tool_input":{"command":"cargo test"}}),
            );
            if kind == "PostToolUse" {
                p["tool_response"] = json!({"interrupted":false});
            }
            assert_eq!(handle(&p, env), Decision::None);
        };
        run(&mut env, "PreToolUse");
        run(&mut env, "PostToolUse");
        let read = in_project(
            &env,
            json!({"hook_event_name":"PreToolUse","session_id":"session","agent_id":"r1",
                "tool_use_id":"read","tool_name":"Read",
                "tool_input":{"file_path":project.join("src/a.rs")}}),
        );
        assert_eq!(handle(&read, &mut env), Decision::None);
        let review = in_project(
            &env,
            json!({"hook_event_name":"SubagentStop","session_id":"session","agent_id":"r1",
                "stop_hook_active":false,
                "last_assistant_message":format!("REVIEW: {decision}\nFINDING: one")}),
        );
        assert_eq!(handle(&review, &mut env), Decision::None);
        git(&project, &["commit", "-q", "-a", "-m", "work"]);
        let stop = in_project(
            &env,
            json!({"hook_event_name":"Stop","session_id":"session","stop_hook_active":false,
                "last_assistant_message":"Done."}),
        );
        let r = handle(&stop, &mut env);
        if decision == "APPROVED" {
            assert_eq!(r, Decision::None, "the approval follows the commit");
            assert_eq!(gate(&env)["state"], "completed");
        } else {
            assert!(
                blocked(&r).contains("rejected"),
                "the rejection follows the commit: {r:?}"
            );
        }
        let stored = reviews(&env);
        assert_eq!(stored.len(), 2, "{stored:?}");
        assert!(stored.iter().all(|r| r["tree"].is_string()), "{stored:?}");
        assert_ne!(stored[0]["candidate"], stored[1]["candidate"]);
    }
}

// ---- contract revisions and repair rounds: the production layout ------------------------

struct Production {
    _d: Dir,
    env: Env,
    project: PathBuf,
    task: String,
    prompts: u64,
}

fn production(name: &str, review: Value) -> Production {
    production_with(name, review, false)
}

/// `git`: the candidate is read from the real repository, which holds a documentation-only
/// README, instead of a fixed fake.
fn production_with(name: &str, review: Value, git_candidate: bool) -> Production {
    let d = dir(name);
    let project = d.0.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    if git_candidate {
        docs_only_readme(&project);
    }
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    let root = d.0.join("state");
    fs::create_dir_all(&root).unwrap();
    let candidate: Box<dyn CandidateProvider> = if git_candidate {
        Box::new(GitCandidate {
            cwd: project.clone(),
        })
    } else {
        Box::new(Switch(Arc::new(Mutex::new("one"))))
    };
    let env = Env {
        state_dir: root,
        state_location: "test",
        candidate,
        clock: || 1_000,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    };
    let mut p = Production {
        _d: d,
        env,
        project,
        task: String::new(),
        prompts: 0,
    };
    let proposal = json!({"id":"wp5","goal":"review","scope":["src/", "README.md"],"forbidden_effects":[],
        "max_repair_rounds":2,"review":review,"artifacts":["src/lib.rs"],
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test -p fixture"}]});
    let started = p.apply("start", None, proposal).unwrap();
    p.task = started["task_id"].as_str().unwrap().to_owned();
    p
}
impl Production {
    fn apply(
        &mut self,
        action: &str,
        task: Option<&str>,
        contract: Value,
    ) -> Result<Value, String> {
        self.prompts += 1;
        let message = "Implement a small change";
        let event = autonomy::record_prompt(
            &self.env.state_dir,
            &self.project,
            &json!({"session_id":"session","prompt_id":format!("p{}", self.prompts),"prompt":message}),
        )?;
        let mut call = json!({"event_id":event["event_id"],"nonce":event["nonce"],
            "prompt_hash":event["prompt_hash"],"action":action,"task_id":task});
        if !contract.is_null() {
            call["contract"] = contract;
        }
        autonomy::propose(&self.env.state_dir, &self.project, &call)
    }
    /// A prompt recorded for this session and left unclassified.
    fn prompt_only(&mut self, message: &str) -> Value {
        self.prompts += 1;
        autonomy::record_prompt(
            &self.env.state_dir,
            &self.project,
            &json!({"session_id":"session","prompt_id":format!("p{}", self.prompts),"prompt":message}),
        )
        .unwrap()
    }
    fn review_in(&mut self, session: &str, agent: &str, message: &str) -> Decision {
        self.hook(
            json!({"hook_event_name":"SubagentStop","session_id":session,
            "agent_id":agent,"stop_hook_active":false,"last_assistant_message":message}),
        )
    }
    fn reviews(&self) -> Vec<Value> {
        self.gate()["reviews"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn written(&self) -> Value {
        let dir = self.env.state_dir.join("tasks").join(&self.task);
        serde_json::from_str(&fs::read_to_string(dir.join("contract.json")).unwrap()).unwrap()
    }
    fn hook(&mut self, mut payload: Value) -> Decision {
        payload["cwd"] = json!(self.project);
        handle(&payload, &mut self.env)
    }
    /// Every mandatory criterion of the written contract runs once, exit 0.
    fn checks(&mut self, id: &str) {
        self.checks_as("main", id);
    }
    /// `agent` edits a file in scope.
    fn write_as(&mut self, agent: &str) {
        let path = self.project.join("src").join("lib.rs");
        let d = self.hook(json!({"hook_event_name":"PreToolUse","session_id":"session",
            "agent_id":agent,"tool_use_id":format!("w-{agent}"),"tool_name":"Write",
            "tool_input":{"file_path":path,"content":"x"}}));
        assert_eq!(d, Decision::None, "{agent} write");
    }
    /// The gate fields a record-only judgement must leave alone.
    fn untouched(&self) -> Value {
        let g = self.gate();
        json!({"receipts":g["receipts"].as_array().map_or(0, Vec::len),
            "handback_refusals":g["handback_refusals"],"continuations":g["continuations"],
            "consumed_repair_rounds":g["consumed_repair_rounds"],"state":g["state"]})
    }
    fn checks_as(&mut self, agent: &str, id: &str) {
        let written = self.written();
        for (n, criterion) in written["criteria"].as_array().unwrap().iter().enumerate() {
            if criterion["mandatory"] != true {
                continue;
            }
            let command = match &criterion["command"] {
                Value::Array(list) => list[0].clone(),
                one => one.clone(),
            };
            for kind in ["PreToolUse", "PostToolUse"] {
                let d = self.hook(json!({"hook_event_name":kind,"session_id":"session",
                    "agent_id":agent,"tool_use_id":format!("{id}-{n}"),"tool_name":"Bash",
                    "tool_input":{"command":command},"tool_response":{"interrupted":false}}));
                assert_eq!(d, Decision::None, "{kind}");
            }
        }
    }
    fn review(&mut self, agent: &str, message: &str) -> Decision {
        self.hook(
            json!({"hook_event_name":"SubagentStop","session_id":"session",
            "agent_id":agent,"stop_hook_active":false,"last_assistant_message":message}),
        )
    }
    fn stop(&mut self) -> Decision {
        self.stop_with("Done.")
    }
    fn stop_with(&mut self, message: &str) -> Decision {
        self.hook(json!({"hook_event_name":"Stop","session_id":"session",
            "stop_hook_active":false,"last_assistant_message":message}))
    }
    fn gate(&self) -> Value {
        let dir = self.env.state_dir.join("tasks").join(&self.task);
        serde_json::from_slice(&fs::read(dir.join("gate.json")).unwrap()).unwrap()
    }
}

/// An agent's start may set `review` itself (a deliberate deviation from the WP2 follow-up); a
/// revision cannot switch it off, and cannot remove an artifact.
#[test]
fn a_revision_cannot_switch_review_off_or_remove_an_artifact() {
    let mut p = production("preserves", independent());
    assert_eq!(p.written()["review"], independent());
    let task = p.task.clone();
    for (change, what) in [
        (
            json!({"review":{"required":false,"independent":true}}),
            "required off",
        ),
        (
            json!({"review":{"required":true,"independent":false}}),
            "independent off",
        ),
        (json!({"artifacts":[]}), "artifact removed"),
    ] {
        let mut revised = p.written();
        for (key, value) in change.as_object().unwrap() {
            revised[key] = value.clone();
        }
        let why = p.apply("revise", Some(&task), revised).expect_err(what);
        assert!(
            why.contains("review") || why.contains("artifact"),
            "{what}: {why}"
        );
    }
    let mut kept = p.written();
    kept["artifacts"] = json!(["src/lib.rs", "src/extra.rs"]);
    p.apply("revise", Some(&task), kept).unwrap();
}

/// A rejection is the candidate judged wrong, so it spends a repair round (once per version of
/// the code); an absent review spends none.
#[test]
fn a_rejection_spends_a_repair_round_and_an_absent_review_does_not() {
    let mut p = production("rounds-absent", required());
    p.checks("c1");
    let why = blocked(&p.stop()).to_owned();
    assert!(why.contains("no repair round spent"), "{why}");
    assert_eq!(p.gate()["consumed_repair_rounds"], 0);

    // A review that is not independent is absent evidence, not a wrong candidate.
    let mut p = production("rounds-not-independent", independent());
    p.checks("c1");
    let why = blocked(&p.stop_with("REVIEW: APPROVED\nDone.")).to_owned();
    assert!(why.contains("independent"), "{why}");
    assert!(why.contains("no repair round spent"), "{why}");
    assert_eq!(p.gate()["consumed_repair_rounds"], 0);

    let mut p = production("rounds-rejected", required());
    p.checks("c1");
    assert_eq!(
        p.review("r1", "REVIEW: REJECTED\nFINDING: wrong result"),
        Decision::None
    );
    let why = blocked(&p.stop()).to_owned();
    assert!(why.contains("rejected"), "{why}");
    assert_eq!(p.gate()["consumed_repair_rounds"], 1, "{why}");
    let task = p.task.clone();
    p.apply("continue", Some(&task), Value::Null).unwrap();
    let why = blocked(&p.stop()).to_owned();
    assert!(why.contains("already charged"), "{why}");
    assert_eq!(p.gate()["consumed_repair_rounds"], 1);
}

/// A review after completion is journaled and stores nothing; it is not an adapter error.
#[test]
fn a_review_after_completion_stores_nothing() {
    let d = dir("after");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: APPROVED");
    assert_eq!(stop(&mut env, "Done."), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let r = subagent_stop(&mut env, "r2", "REVIEW: REJECTED\nFINDING: late");
    assert_eq!(r, Decision::None);
    let reasons = last_reasons(&env);
    assert!(reasons.contains("not recorded"), "{reasons}");
    assert!(!reasons.contains("adapter error"), "{reasons}");
    assert_eq!(reviews(&env).len(), 1);
    assert_eq!(gate(&env)["state"], "completed");
}

/// The hash chain covers the stored reviews: a hand edit of a finding or a decision, or the
/// removal of the whole `reviews` key, reads as tampered.
#[test]
fn a_hand_edit_of_a_stored_review_reads_as_tampered() {
    let d = dir("tamper");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "r1", "REVIEW: REJECTED\nFINDING: off by one");
    let sealed = fs::read_to_string(task_dir(&env).join("gate.json")).unwrap();
    let (_, digest) = state::read_contract(&task_dir(&env)).unwrap();
    type Edit = fn(&mut Value);
    let edits: [(&str, Edit); 3] = [
        ("finding", |g| {
            g["reviews"][0]["findings"][0] = json!("fine")
        }),
        ("decision", |g| {
            g["reviews"][0]["decision"] = json!("approved")
        }),
        ("removed", |g| {
            g.as_object_mut().unwrap().remove("reviews");
        }),
    ];
    for (what, edit) in edits {
        let mut gate: Value = serde_json::from_str(&sealed).unwrap();
        edit(&mut gate);
        fs::write(task_dir(&env).join("gate.json"), format!("{gate}\n")).unwrap();
        assert!(
            matches!(
                state::load_gate(&task_dir(&env), &digest).unwrap(),
                Stored::Tampered(_)
            ),
            "{what}"
        );
    }
    fs::write(task_dir(&env).join("gate.json"), &sealed).unwrap();
    assert!(matches!(
        state::load_gate(&task_dir(&env), &digest).unwrap(),
        Stored::Valid(_)
    ));
}

/// At a reached limit, a stop refused only for its review is released NOT certified, and the
/// report names the review instead of "remaining criteria: none".
#[test]
fn the_limit_report_names_an_unmet_review() {
    let d = dir("limit-report");
    let mut c = contract(required());
    c["limits"] = json!({"turns":1});
    let (mut env, _) = setup(&d, &c);
    checks_pass(&mut env, "c1");
    let r = stop(&mut env, "Done.");
    let Decision::System(report) = &r else {
        panic!("expected the limit report, got {r:?}");
    };
    assert!(report.contains("NOT certified"), "{report}");
    assert!(
        report.contains("remaining criteria: review required"),
        "{report}"
    );
    plain(report);
}

/// Reviews follow the full content, unlike receipts: a README-only fix sheds a rejection about
/// the README (so it spends no repair round and a fresh approval can clear it), while the
/// receipts still follow the documentation-excluded tree.
#[test]
fn a_rejection_is_not_carried_onto_a_docs_only_fix() {
    let mut p = production_with("docs-fix", required(), true);
    p.checks("c1");
    assert_eq!(
        p.review(
            "r1",
            "REVIEW: REJECTED\nFINDING: the README names the wrong flag"
        ),
        Decision::None
    );
    fs::write(p.project.join("README.md"), "fixed\n").unwrap();
    let why = blocked(&p.stop()).to_owned();
    assert!(why.contains("review required"), "{why}");
    assert!(!why.contains("rejected"), "{why}");
    assert_eq!(p.gate()["consumed_repair_rounds"], 0, "{why}");
    assert_eq!(p.review("r2", "REVIEW: APPROVED"), Decision::None);
    let task = p.task.clone();
    p.apply("continue", Some(&task), Value::Null).unwrap();
    assert_eq!(p.stop(), Decision::None);
    assert_eq!(p.gate()["state"], "completed");
}

#[test]
fn an_approval_is_not_carried_onto_a_docs_only_edit() {
    let (_d, mut env) = repository("docs-edit");
    let project = project_of(&env);
    for kind in ["PreToolUse", "PostToolUse"] {
        let p = in_project(
            &env,
            json!({"hook_event_name":kind,"session_id":"session","agent_id":"main",
                "tool_use_id":"run","tool_name":"Bash","tool_input":{"command":"cargo test"},
                "tool_response":{"interrupted":false}}),
        );
        assert_eq!(handle(&p, &mut env), Decision::None);
    }
    let review = in_project(
        &env,
        json!({"hook_event_name":"SubagentStop","session_id":"session","agent_id":"r1",
            "stop_hook_active":false,"last_assistant_message":"REVIEW: APPROVED"}),
    );
    assert_eq!(handle(&review, &mut env), Decision::None);
    fs::write(project.join("README.md"), "changed after the review\n").unwrap();
    let stop = in_project(
        &env,
        json!({"hook_event_name":"Stop","session_id":"session","stop_hook_active":false,
            "last_assistant_message":"Done."}),
    );
    let r = handle(&stop, &mut env);
    let why = blocked(&r);
    assert!(
        why.contains("review required"),
        "the receipts carried, the review did not: {why}"
    );
    assert!(!why.contains("C2"), "the check is met: {why}");
    let g = gate(&env);
    assert!(
        g["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["candidate"] == g["candidate"] && r["exit_code"] == 0),
        "a receipt on the edited candidate: {g}"
    );
    assert_ne!(g["state"], "completed");
}

/// A reviewer's rejection is recorded while this session's prompt is still unclassified, and
/// while the prompt is a discussion turn; approvals there stay dropped (they fail toward review
/// required). The write happens under the task lock after the ownership recheck.
#[test]
fn a_rejection_is_recorded_on_a_pending_or_discussion_turn() {
    for discussion in [false, true] {
        let mut p = production(
            if discussion {
                "discussion-reject"
            } else {
                "pending-reject"
            },
            required(),
        );
        p.checks("c1");
        // A builder with a run the gate has not synced yet: a full judgement would record its
        // receipt and judge its handback; record-only changes neither.
        p.write_as("w1");
        p.checks_as("w1", "w1-c");
        let event = p.prompt_only("hold on a moment");
        if discussion {
            supervisor_claude_code::autonomy::propose(
                &p.env.state_dir,
                &p.project,
                &json!({"event_id":event["event_id"],"nonce":event["nonce"],
                    "prompt_hash":event["prompt_hash"],"action":"discussion","task_id":p.task}),
            )
            .unwrap();
        } else {
            assert!(
                autonomy::pending(&p.env.state_dir, "session")
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(p.review("a1", "REVIEW: APPROVED"), Decision::None);
        assert!(p.reviews().is_empty(), "an approval here stays dropped");
        let before = p.untouched();
        assert_eq!(
            p.review("r1", "REVIEW: REJECTED\nFINDING: wrong result"),
            Decision::None
        );
        if discussion {
            let handback = "REVIEW: REJECTED\nFINDING: still wrong\nVERDICT: PASS\nCHECKS RUN: 0";
            assert_eq!(p.review("w1", handback), Decision::None);
        }
        assert_eq!(p.untouched(), before, "discussion={discussion}: record-only");
        let stored = p.reviews();
        let want = if discussion { 2 } else { 1 };
        assert_eq!(stored.len(), want, "discussion={discussion}: {stored:?}");
        assert_eq!(stored[0]["decision"], "rejected");
        assert_eq!(stored[0]["reviewer"], "r1");
        let task = p.task.clone();
        p.apply("continue", Some(&task), Value::Null).unwrap();
        // w1's run reached no receipt (its handback was not judged), so the chair reruns.
        p.checks("c2");
        let why = blocked(&p.stop()).to_owned();
        assert!(why.contains("rejected by r1"), "{why}");
    }
}

/// Guard of the ownership check: another session's task is never written to, a rejection
/// included.
#[test]
fn ownership_guard_another_sessions_rejection_records_nothing() {
    let mut p = production("other-session", required());
    p.checks("c1");
    assert_eq!(
        p.review_in("someone-else", "r1", "REVIEW: REJECTED\nFINDING: wrong"),
        Decision::None
    );
    assert!(p.reviews().is_empty());
}

struct Unobservable;
impl CandidateProvider for Unobservable {
    fn observe(&self) -> Result<Candidate, String> {
        Err("git rev-parse HEAD failed".into())
    }
}

/// A rejection that cannot be bound to a candidate is not dropped silently: it is an adapter
/// error, which fails closed at `enforce`. An approval there is only journaled.
#[test]
fn a_rejection_on_an_unobserved_candidate_fails_closed() {
    let d = dir("unobserved");
    let mut env = env_with(&d, &contract(required()), Box::new(Unobservable));
    assert_eq!(
        subagent_stop(&mut env, "a1", "REVIEW: APPROVED"),
        Decision::None
    );
    let r = subagent_stop(&mut env, "r1", "REVIEW: REJECTED\nFINDING: wrong");
    let why = blocked(&r);
    assert!(why.contains("fails closed"), "{why}");
    plain(why);
}

/// A contract revision clears the receipts; it drops the approvals too (the reviewed contract
/// changed) and keeps the rejections, so a revision cannot shed one.
#[test]
fn a_revision_drops_approvals_and_keeps_rejections() {
    let mut p = production("revision-reviews", required());
    p.checks("c1");
    assert_eq!(p.review("a1", "REVIEW: APPROVED"), Decision::None);
    assert_eq!(
        p.review("r1", "REVIEW: REJECTED\nFINDING: no"),
        Decision::None
    );
    assert_eq!(p.reviews().len(), 2);
    let mut revised = p.written();
    revised["artifacts"] = json!(["src/lib.rs", "src/extra.rs"]);
    let task = p.task.clone();
    p.apply("revise", Some(&task), revised).unwrap();
    let kept = p.reviews();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0]["decision"], "rejected");
}

/// Item 4: a builder's rejection on a pending-prompt turn gets the unclassified refusal, and the
/// rejection is still recorded (caller-asserted), record-only.
#[test]
fn a_builders_rejection_on_a_pending_turn_is_recorded() {
    let mut p = production("pending-builder", required());
    p.checks("c1");
    p.write_as("w1");
    p.checks_as("w1", "w1-c");
    p.prompt_only("hold on a moment");
    let before = p.untouched();
    let r = p.review("w1", "REVIEW: REJECTED\nFINDING: the tests fail");
    assert!(matches!(r, Decision::Block(_)), "the route's refusal stays: {r:?}");
    assert_eq!(p.untouched(), before);
    let stored = p.reviews();
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0]["decision"], "rejected");
    assert_eq!(stored[0]["provenance"], "caller-asserted");
}

/// Item 3 through the hook: a loosely written rejection is recorded, a loosely written approval
/// is not.
#[test]
fn a_loose_rejection_counts_and_a_loose_approval_does_not() {
    let d = dir("loose");
    let (mut env, _) = setup(&d, &contract(required()));
    checks_pass(&mut env, "c1");
    reviewer(&mut env, "a1", "**REVIEW: APPROVED**");
    reviewer(&mut env, "a2", "REVIEW: APPROVED - with caveats");
    assert!(reviews(&env).is_empty());
    reviewer(&mut env, "r1", "**Review: rejected** - see below\nFINDING: wrong");
    assert_eq!(reviews(&env)[0]["decision"], "rejected");
}

// ---- item 2: an approval binds to the content the reviewer was launched on ---------------

fn launch_in_background(env: &mut Env, agent: &str) {
    let p = in_project(
        env,
        json!({"hook_event_name":"PostToolUse","session_id":"session","tool_name":"Agent",
            "tool_use_id":format!("a-{agent}"),"tool_input":{"prompt":"review"},
            "tool_response":{"status":"async_launched","agentId":agent,
                "resolvedModel":"claude-haiku-5-5"}}),
    );
    handle(&p, env);
}
fn git_checks(env: &mut Env, id: &str) {
    for kind in ["PreToolUse", "PostToolUse"] {
        let p = in_project(
            env,
            json!({"hook_event_name":kind,"session_id":"session","agent_id":"main",
                "tool_use_id":id,"tool_name":"Bash","tool_input":{"command":"cargo test"},
                "tool_response":{"interrupted":false}}),
        );
        assert_eq!(handle(&p, env), Decision::None);
    }
}
fn git_review(env: &mut Env, agent: &str, message: &str) -> Decision {
    let p = in_project(
        env,
        json!({"hook_event_name":"SubagentStop","session_id":"session","agent_id":agent,
            "stop_hook_active":false,"last_assistant_message":message}),
    );
    handle(&p, env)
}
fn git_stop(env: &mut Env) -> Decision {
    let p = in_project(
        env,
        json!({"hook_event_name":"Stop","session_id":"session","stop_hook_active":false,
            "last_assistant_message":"Done."}),
    );
    handle(&p, env)
}

#[test]
fn an_approval_after_the_chair_edited_during_the_review_is_not_counted() {
    let (_d, mut env) = repository("launch-edited");
    git_checks(&mut env, "c1");
    launch_in_background(&mut env, "r1");
    // The chair keeps working while the background reviewer reads.
    fs::write(project_of(&env).join("src/a.rs"), "a3\n").unwrap();
    git_checks(&mut env, "c2");
    assert_eq!(git_review(&mut env, "r1", "REVIEW: APPROVED"), Decision::None);
    assert!(reviews(&env).is_empty(), "{:?}", reviews(&env));
    assert!(last_reasons(&env).contains("launched"), "{}", last_reasons(&env));
    let why = blocked(&git_stop(&mut env)).to_owned();
    assert!(why.contains("review required"), "{why}");
}

#[test]
fn an_approval_with_no_edit_since_the_launch_is_counted() {
    let (_d, mut env) = repository("launch-same");
    git_checks(&mut env, "c1");
    launch_in_background(&mut env, "r1");
    assert_eq!(git_review(&mut env, "r1", "REVIEW: APPROVED"), Decision::None);
    assert_eq!(reviews(&env)[0]["decision"], "approved");
    assert_eq!(git_stop(&mut env), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn a_rejection_after_an_edit_since_the_launch_is_recorded() {
    let (_d, mut env) = repository("launch-rejected");
    git_checks(&mut env, "c1");
    launch_in_background(&mut env, "r1");
    fs::write(project_of(&env).join("src/a.rs"), "a3\n").unwrap();
    git_review(&mut env, "r1", "REVIEW: REJECTED\nFINDING: wrong");
    assert_eq!(reviews(&env)[0]["decision"], "rejected");
}
