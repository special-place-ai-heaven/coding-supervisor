//! Hook replay proves receipt selection, not live harness enforcement.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::CandidateProvider;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Fixed(&'static str);
impl CandidateProvider for Fixed {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: self.0.into(),
            },
            provenance: "fixture".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}
fn setup() -> (Fixture, Env) {
    let d = Fixture(std::env::temp_dir().join(format!(
        "supervisor-headerless-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let state = d.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("contract.json"),
        json!({
            "id":"headerless", "goal":"observed completion", "scope":["src/"],
            "forbidden_effects":[], "max_repair_rounds":2,
            "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]
        })
        .to_string(),
    )
    .unwrap();
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Fixed("a")),
        clock: || 100,
    };
    (d, env)
}
fn event(env: &mut Env, kind: &str, agent: &str, id: &str, exit: Option<i32>) -> Decision {
    let mut p = json!({
        "hook_event_name":kind, "cwd":env.state_dir.parent().unwrap(),
        "session_id":"session", "agent_id":agent, "tool_use_id":id,
        "tool_name":"Bash", "tool_input":{"command":"cargo test"}
    });
    if let Some(exit) = exit {
        if exit == 0 {
            p["tool_response"] = json!({"interrupted":false});
        } else {
            p["hook_event_name"] = "PostToolUseFailure".into();
            p["error"] = format!("Exit code {exit}\nfailed").into();
        }
    }
    handle(&p, env)
}
fn run(env: &mut Env, agent: &str, id: &str, exit: i32) {
    event(env, "PreToolUse", agent, id, None);
    event(env, "PostToolUse", agent, id, Some(exit));
}
fn stop(env: &mut Env, kind: &str, agent: &str) -> Decision {
    handle(
        &json!({"hook_event_name":kind,"cwd":env.state_dir.parent().unwrap(),
        "session_id":"session","agent_id":agent,"last_assistant_message":"Done.",
        "stop_hook_active":false}),
        env,
    )
}
fn gate(env: &Env) -> Value {
    let dir = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .map_or_else(|| env.state_dir.clone(), |task| task.dir);
    serde_json::from_slice(&fs::read(dir.join("gate.json")).unwrap()).unwrap()
}

fn prompt(env: &mut Env, id: &str, message: &str) -> Value {
    let result = handle(
        &json!({"hook_event_name":"UserPromptSubmit",
        "cwd":env.state_dir.parent().unwrap(), "session_id":"session",
        "prompt_id":id,"prompt":message}),
        env,
    );
    assert!(matches!(result, Decision::Context { .. }));
    supervisor_claude_code::autonomy::pending(&env.state_dir, "session")
        .unwrap()
        .unwrap()
}

fn propose(
    env: &Env,
    event: &Value,
    action: &str,
    task: Option<&str>,
    contract: Option<Value>,
) -> Value {
    supervisor_claude_code::autonomy::propose(
        &env.state_dir,
        env.state_dir.parent().unwrap(),
        &json!({"event_id":event["event_id"],
            "nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,
            "task_id":task,"contract":contract}),
    )
    .unwrap()
}

fn autonomous() -> (Fixture, Env, String) {
    let (d, mut env) = setup();
    fs::write(
        d.0.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let contract: Value =
        serde_json::from_slice(&fs::read(env.state_dir.join("contract.json")).unwrap()).unwrap();
    fs::remove_file(env.state_dir.join("contract.json")).unwrap();
    let event = prompt(&mut env, "start", "Implement the requested behavior");
    let result = propose(&env, &event, "start", None, Some(contract));
    let task = result["task_id"].as_str().unwrap().to_owned();
    (d, env, task)
}

fn run_autonomous_checks(env: &mut Env, prefix: &str) {
    let active = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let (contract, _) = supervisor_claude_code::state::read_contract(&active.dir).unwrap();
    for (index, criterion) in contract
        .unwrap()
        .criteria
        .iter()
        .filter(|c| c.mandatory)
        .enumerate()
    {
        let mut payload = json!({"hook_event_name":"PreToolUse", "cwd":env.state_dir.parent().unwrap(),
            "session_id":"session","agent_id":"main","tool_use_id":format!("{prefix}-{index}"),
            "tool_name":"Bash","tool_input":{"command":criterion.command[0]}});
        assert_eq!(handle(&payload, env), Decision::None);
        payload["hook_event_name"] = "PostToolUse".into();
        payload["tool_response"] = json!({"interrupted":false});
        assert_eq!(handle(&payload, env), Decision::None);
    }
}

#[test]
fn stop_derives_receipts_without_check_headers() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "run", 0);
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    assert_eq!(gate(&env)["receipts"].as_array().unwrap().len(), 1);
}

#[test]
fn worker_derives_receipts_without_headers_but_does_not_complete_task() {
    let (_d, mut env) = setup();
    run(&mut env, "worker", "run", 0);
    assert_eq!(stop(&mut env, "SubagentStop", "worker"), Decision::None);
    assert_eq!(gate(&env)["state"], "ready_for_review");
}

#[test]
fn later_failure_cannot_be_masked_by_earlier_success() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "success", 0);
    run(&mut env, "main", "failure", 9);
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn later_unknown_attempt_cannot_be_masked_by_earlier_success() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "success", 0);
    event(&mut env, "PreToolUse", "main", "pending", None);
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn successful_retry_supersedes_the_failed_attempt_after_restart() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "failure", 9);
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    run(&mut env, "main", "retry", 0);
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    assert_eq!(gate(&env)["receipts"].as_array().unwrap().len(), 2);
}

#[test]
fn citing_the_old_success_cannot_hide_the_latest_failure() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "success", 0);
    run(&mut env, "main", "failure", 9);
    let response = handle(
        &json!({"hook_event_name":"Stop", "session_id":"session",
        "cwd":env.state_dir.parent().unwrap(),
        "last_assistant_message":"VERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | cargo test | exit 0"}),
        &mut env,
    );
    assert!(matches!(response, Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn malformed_observation_is_a_stop_refusal() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "success", 0);
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(env.state_dir.join("runs.jsonl"))
        .unwrap()
        .write_all(b"not-json\n")
        .unwrap();
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn candidate_change_invalidates_headerless_evidence() {
    let (_d, mut env) = setup();
    run(&mut env, "main", "success", 0);
    env.candidate = Box::new(Fixed("b"));
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn a_claim_without_observed_execution_cannot_complete() {
    let (_d, mut env) = setup();
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_ne!(gate(&env)["state"], "completed");
}

#[test]
fn prompt_gates_mutation_but_keeps_inspection_and_proposal_usable() {
    let (_d, mut env) = setup();
    fs::remove_file(env.state_dir.join("contract.json")).unwrap();
    prompt(&mut env, "pending", "Implement the change");
    for tool in [
        "Write",
        "Agent",
        "mcp__terminal_commander__file_write",
        "mcp__terminal_commander__run_and_watch",
    ] {
        let response = handle(
            &json!({"hook_event_name":"PreToolUse", "session_id":"session",
            "cwd":env.state_dir.parent().unwrap(), "tool_name":tool,"tool_input":{}}),
            &mut env,
        );
        assert!(
            matches!(response, Decision::Deny(_)),
            "{tool}: {response:?}"
        );
    }
    for tool in [
        "Read",
        "mcp__symforge__get_file_context",
        "mcp__supervisor__intent_apply",
        "mcp__terminal_commander__command_status",
    ] {
        let response = handle(
            &json!({"hook_event_name":"PreToolUse", "session_id":"session",
            "cwd":env.state_dir.parent().unwrap(), "tool_name":tool,"tool_input":{}}),
            &mut env,
        );
        assert!(
            !matches!(response, Decision::Deny(_)),
            "{tool}: {response:?}"
        );
    }
}

#[test]
fn discussion_cannot_be_used_to_edit_without_a_scoped_task() {
    let (_d, mut env) = setup();
    fs::remove_file(env.state_dir.join("contract.json")).unwrap();
    let event = prompt(&mut env, "discussion", "Explain the design");
    propose(&env, &event, "discussion", None, None);
    let response = handle(
        &json!({"hook_event_name":"PreToolUse", "session_id":"session",
        "cwd":env.state_dir.parent().unwrap(),"tool_name":"Write","tool_input":{}}),
        &mut env,
    );
    assert!(matches!(response, Decision::Deny(_)));
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
}

#[test]
fn autonomous_stop_evaluates_evidence_without_completion_wording() {
    let (_d, mut env, _task) = autonomous();
    run_autonomous_checks(&mut env, "initial");
    let result = handle(
        &json!({"hook_event_name":"Stop", "session_id":"session",
        "cwd":env.state_dir.parent().unwrap(),"last_assistant_message":"The implementation uses the existing pattern."}),
        &mut env,
    );
    assert_eq!(result, Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn autonomous_refusals_exhaust_repair_budget_across_followup_prompts() {
    let (_d, mut env, task) = autonomous();
    for index in 0..3 {
        assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
        if index < 2 {
            let event = prompt(
                &mut env,
                &format!("followup-{index}"),
                "Continue the same work",
            );
            propose(&env, &event, "continue", Some(&task), None);
        }
    }
    assert_eq!(gate(&env)["state"], "budget_exhausted");
    assert_eq!(gate(&env)["consumed_repair_rounds"], 2);
}

#[test]
fn active_discussion_preserves_task_but_does_not_authorize_mutation() {
    let (_d, mut env, task) = autonomous();
    let event = prompt(&mut env, "discussion", "Explain status without editing");
    propose(&env, &event, "discussion", None, None);
    for tool in ["Write", "Agent", "mcp__terminal_commander__file_write"] {
        let response = handle(
            &json!({"hook_event_name":"PreToolUse", "session_id":"session",
            "cwd":env.state_dir.parent().unwrap(),"tool_name":tool,"tool_input":{}}),
            &mut env,
        );
        assert!(
            matches!(response, Decision::Deny(_)),
            "{tool}: {response:?}"
        );
    }
    assert_eq!(
        supervisor_claude_code::lifecycle::active_task(&env.state_dir)
            .unwrap()
            .unwrap()
            .task_id,
        task
    );
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    let event = prompt(&mut env, "followup", "Continue the same work");
    propose(&env, &event, "continue", Some(&task), None);
    let response = handle(
        &json!({"hook_event_name":"PreToolUse", "session_id":"session",
        "cwd":env.state_dir.parent().unwrap(),"tool_name":"Write","tool_input":{}}),
        &mut env,
    );
    assert_eq!(response, Decision::None);
}

#[test]
fn scope_revision_preserves_journal_but_requires_fresh_checks() {
    let (_d, mut env, task) = autonomous();
    run_autonomous_checks(&mut env, "before-revision");
    assert_eq!(stop(&mut env, "SubagentStop", "main"), Decision::None);
    let active = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let old_runs = fs::read(active.dir.join("runs.jsonl")).unwrap();
    let mut contract: Value =
        serde_json::from_slice(&fs::read(active.dir.join("contract.json")).unwrap()).unwrap();
    contract["scope"] = json!(["src/", "tests/"]);
    let event = prompt(&mut env, "revision", "Also update the tests");
    propose(&env, &event, "revise", Some(&task), Some(contract));
    assert_eq!(fs::read(active.dir.join("runs.jsonl")).unwrap(), old_runs);
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert!(gate(&env)["receipts"].as_array().unwrap().is_empty());
    run_autonomous_checks(&mut env, "after-revision");
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

#[test]
fn prompt_record_failure_blocks_later_mutation() {
    let (_d, mut env) = setup();
    fs::remove_file(env.state_dir.join("contract.json")).unwrap();
    fs::write(
        env.state_dir.parent().unwrap().join("package.json"),
        "{invalid",
    )
    .unwrap();
    let result = handle(
        &json!({"hook_event_name":"UserPromptSubmit",
        "cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "prompt_id":"failed","prompt":"Implement this change"}),
        &mut env,
    );
    assert!(matches!(result, Decision::Block(_)));
    let result = handle(
        &json!({"hook_event_name":"PreToolUse",
        "cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "tool_name":"Write","tool_input":{}}),
        &mut env,
    );
    assert!(matches!(result, Decision::Deny(_)));
}

#[test]
fn paused_task_can_report_pause_and_only_resume_releases_mutation() {
    let (_d, mut env, task) = autonomous();
    let event = prompt(&mut env, "pause", "pause task");
    propose(&env, &event, "pause", Some(&task), None);
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["consumed_repair_rounds"], 0);
    let result = handle(
        &json!({"hook_event_name":"PreToolUse",
        "cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "tool_name":"mcp__terminal_commander__file_write","tool_input":{}}),
        &mut env,
    );
    assert!(matches!(result, Decision::Deny(_)));
    let event = prompt(&mut env, "resume", "resume task");
    propose(&env, &event, "resume", Some(&task), None);
    let result = handle(
        &json!({"hook_event_name":"PreToolUse",
        "cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "tool_name":"Write","tool_input":{}}),
        &mut env,
    );
    assert_eq!(result, Decision::None);
}
