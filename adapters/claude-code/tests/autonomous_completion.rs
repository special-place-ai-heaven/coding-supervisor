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
        level: Default::default(),
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
    // G18: a follow-up on the session's own task is no longer pending, so read the event.
    let store: Value =
        serde_json::from_slice(&fs::read(env.state_dir.join("intents.json")).unwrap()).unwrap();
    store["events"][store["latest"]["session"].as_str().unwrap()].clone()
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
    run_autonomous_checks_with(env, prefix, 0);
}

fn run_autonomous_checks_with(env: &mut Env, prefix: &str, exit: i32) {
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
        if exit == 0 {
            payload["hook_event_name"] = "PostToolUse".into();
            payload["tool_response"] = json!({"interrupted":false});
        } else {
            payload["hook_event_name"] = "PostToolUseFailure".into();
            payload["error"] = format!("Exit code {exit}\nfailed").into();
        }
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
fn a_required_check_started_in_the_background_is_refused() {
    let (_d, mut env) = setup();
    let mut payload = json!({"hook_event_name":"PreToolUse", "cwd":env.state_dir.parent().unwrap(),
        "session_id":"session","agent_id":"main","tool_use_id":"bg",
        "tool_name":"Bash","tool_input":{"command":"cargo test","run_in_background":true}});
    let Decision::Deny(reason) = handle(&payload, &mut env) else {
        panic!("a background run of a required check must be refused");
    };
    assert!(reason.contains("foreground"), "{reason}");
    payload["tool_use_id"] = "bg-other".into();
    payload["tool_input"] = json!({"command":"npm run dev","run_in_background":true});
    assert_eq!(handle(&payload, &mut env), Decision::None);
}

#[test]
fn refusal_count_stops_at_three_per_turn() {
    let (_d, mut env) = setup();
    for n in 1..=3 {
        let Decision::Block(reason) = stop(&mut env, "Stop", "main") else {
            panic!("refusal {n} must block");
        };
        assert!(
            reason.contains(&format!("refusal {n} of 3 this turn")),
            "{reason}"
        );
        assert!(!reason.contains("harness cap"), "{reason}");
    }
    assert!(matches!(
        stop(&mut env, "Stop", "main"),
        Decision::System(_)
    ));
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

/// The prompt hook's context for a new prompt in `session`.
fn prompt_context(env: &mut Env, session: &str, id: &str) -> String {
    let Decision::Context { text, .. } = handle(
        &json!({"hook_event_name":"UserPromptSubmit",
        "cwd":env.state_dir.parent().unwrap(), "session_id":session,
        "prompt_id":id,"prompt":"Now add the other thing"}),
        env,
    ) else {
        panic!("a prompt gets its context");
    };
    text
}

/// G59: after the completing Stop (which still answers nothing), the next prompt says the task
/// was accepted and on how many hook-observed receipts; a task completed on caller-asserted
/// receipts gets no such clause.
#[test]
fn a_prompt_after_completion_reports_the_hook_observed_receipts() {
    let (_d, mut env, task) = autonomous();
    run_autonomous_checks(&mut env, "initial");
    let active = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let (contract, _) = supervisor_claude_code::state::read_contract(&active.dir).unwrap();
    let mandatory = contract
        .unwrap()
        .criteria
        .iter()
        .filter(|c| c.mandatory)
        .count();
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let text = prompt_context(&mut env, "session", "after");
    assert!(
        text.contains(&format!(
            "The active task {task} is completed and takes no more work (continue is refused); it was accepted on {mandatory} hook-observed receipts."
        )),
        "{text}"
    );

    // The same task shape completed on caller-asserted receipts only.
    use coding_supervisor::{CriterionId, TaskState, VerificationReceipt};
    use supervisor_claude_code::state::{Stored, load_gate, save_gate};
    let (_d, mut env, _task) = autonomous();
    let write = json!({"hook_event_name":"PreToolUse","cwd":env.state_dir.parent().unwrap(),
        "session_id":"session","agent_id":"main","tool_use_id":"w1","tool_name":"Write",
        "tool_input":{"file_path":"src/lib.rs","content":"x"}});
    assert_eq!(handle(&write, &mut env), Decision::None);
    let active = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let (contract, digest) = supervisor_claude_code::state::read_contract(&active.dir).unwrap();
    let Stored::Valid(mut g) = load_gate(&active.dir, &digest).unwrap() else {
        panic!("no stored gate");
    };
    for criterion in contract.unwrap().criteria.iter().filter(|c| c.mandatory) {
        g.receipts.push(VerificationReceipt {
            criterion: CriterionId(criterion.id.0.clone()),
            candidate: g.checkpoint.candidate.clone().unwrap(),
            command: criterion.command[0].clone(),
            cwd: ".".into(),
            exit_code: Some(0),
            output_ref: format!(
                "{}{}",
                supervisor_claude_code::adapter::MCP_ASSERTED,
                criterion.id.0
            ),
            claim_supported: true,
            started_at: 1,
            finished_at: 1,
        });
    }
    g.checkpoint.state = TaskState::Completed;
    save_gate(&active.dir, &g, &digest).unwrap();
    let text = prompt_context(&mut env, "session", "after");
    assert!(
        text.contains("is completed and takes no more work"),
        "{text}"
    );
    assert!(!text.contains("hook-observed"), "{text}");
}

#[test]
fn autonomous_refusals_exhaust_repair_budget_across_followup_prompts() {
    let (_d, mut env, task) = autonomous();
    // Each round is a real repair: new code, checks run again, still failing.
    for (index, version) in ["a", "b", "c"].into_iter().enumerate() {
        env.candidate = Box::new(Fixed(version));
        run_autonomous_checks_with(&mut env, &format!("failing-{index}"), 1);
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
fn repeated_refusals_on_unchanged_code_spend_one_round() {
    let (_d, mut env, task) = autonomous();
    run_autonomous_checks_with(&mut env, "failing", 1);
    for index in 0..3 {
        let Decision::Block(reason) = stop(&mut env, "Stop", "main") else {
            panic!("a failing check must refuse");
        };
        if index > 0 {
            assert!(reason.contains("already charged"), "{reason}");
        }
        let event = prompt(&mut env, &format!("wait-{index}"), "Continue the same work");
        propose(&env, &event, "continue", Some(&task), None);
    }
    assert_eq!(gate(&env)["consumed_repair_rounds"], 1);
    assert_ne!(gate(&env)["state"], "budget_exhausted");
    env.candidate = Box::new(Fixed("b"));
    run_autonomous_checks_with(&mut env, "failing-again", 1);
    assert!(matches!(stop(&mut env, "Stop", "main"), Decision::Block(_)));
    assert_eq!(gate(&env)["consumed_repair_rounds"], 2);
}

#[test]
fn refusal_without_any_observed_check_spends_no_repair_round() {
    let (_d, mut env, task) = autonomous();
    for index in 0..3 {
        let Decision::Block(reason) = stop(&mut env, "Stop", "main") else {
            panic!("stop without evidence must refuse");
        };
        assert!(reason.contains("no repair round spent"), "{reason}");
        let event = prompt(
            &mut env,
            &format!("followup-{index}"),
            "Continue the same work",
        );
        propose(&env, &event, "continue", Some(&task), None);
    }
    assert_eq!(gate(&env)["consumed_repair_rounds"], 0);
    assert_ne!(gate(&env)["state"], "budget_exhausted");
    run_autonomous_checks(&mut env, "finally");
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
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

/// A Terminal Commander job whose exit was seen only through `bucket_wait` has no recorded
/// completion. The refusal names the call that records it and the job, so the agent is not
/// left guessing.
#[test]
fn a_tc_job_without_a_recorded_exit_names_command_status_and_the_job() {
    let (_d, mut env, _task) = autonomous();
    run_autonomous_checks(&mut env, "passing");
    let cwd = env.state_dir.parent().unwrap().to_path_buf();
    let active = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let (contract, _) = supervisor_claude_code::state::read_contract(&active.dir).unwrap();
    let command = contract
        .unwrap()
        .criteria
        .iter()
        .find(|c| c.mandatory)
        .unwrap()
        .command[0]
        .clone();
    let argv: Vec<&str> = command.split_whitespace().collect();
    let mut payload = json!({"hook_event_name":"PreToolUse","cwd":cwd,"session_id":"session",
        "agent_id":"main","tool_use_id":"tc-rerun",
        "tool_name":"mcp__terminal_commander__command_start_combed",
        "tool_input":{"argv":argv,"cwd":cwd}});
    assert_eq!(handle(&payload, &mut env), Decision::None);
    payload["hook_event_name"] = "PostToolUse".into();
    payload["tool_response"] = json!({"job_id":"job-unfetched","bucket_id":"bucket-1"});
    handle(&payload, &mut env);
    let Decision::Block(reason) = stop(&mut env, "Stop", "main") else {
        panic!("an unrecorded rerun must refuse");
    };
    assert!(
        reason.contains("command_status") && reason.contains("job-unfetched"),
        "{reason}"
    );
}

/// G14: once an autonomous task is completed, read-only commands such as `supervisor status`
/// still run, and the refusal of anything else names the next step.
#[test]
fn a_completed_task_still_allows_read_only_commands_and_names_the_next_step() {
    let (_d, mut env, _task) = autonomous();
    run_autonomous_checks(&mut env, "initial");
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let cwd = env.state_dir.parent().unwrap().to_path_buf();
    let bash = |command: &str| {
        json!({"hook_event_name":"PreToolUse","cwd":cwd,"session_id":"session","agent_id":"main",
            "tool_use_id":command,"tool_name":"Bash","tool_input":{"command":command}})
    };
    for command in [
        "supervisor status --json",
        "supervisor level",
        "git status --short",
        // G40: the forge's read-only views after a merge.
        "gh pr checks 59",
    ] {
        assert_eq!(
            handle(&bash(command), &mut env),
            Decision::None,
            "{command}"
        );
    }
    // G40: asking the user changes nothing, so the question picker stays open.
    let ask = json!({"hook_event_name":"PreToolUse","cwd":cwd,"session_id":"session","agent_id":"main",
        "tool_use_id":"ask","tool_name":"AskUserQuestion","tool_input":{"questions":[]}});
    assert_eq!(handle(&ask, &mut env), Decision::None);
    let Decision::Deny(reason) = handle(&bash("touch x"), &mut env) else {
        panic!("a mutation after completion must still be refused");
    };
    assert!(
        reason.contains("task is completed") && reason.contains("new work prompt"),
        "{reason}"
    );
}

/// Review of G47: another session's completed task protects nothing (a start from this session
/// is accepted), so this session gets the same advice, not "protected; start is refused".
#[test]
fn a_prompt_after_another_sessions_completion_advises_a_new_task() {
    let (_d, mut env, _task) = autonomous();
    run_autonomous_checks(&mut env, "initial");
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let done = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap();
    let mut contract: Value =
        serde_json::from_slice(&fs::read(done.dir.join("contract.json")).unwrap()).unwrap();
    contract["id"] = "second".into();
    let Decision::Context { text, .. } = handle(
        &json!({"hook_event_name":"UserPromptSubmit",
        "cwd":env.state_dir.parent().unwrap(), "session_id":"other",
        "prompt_id":"other-1","prompt":"Now add the other thing"}),
        &mut env,
    ) else {
        panic!("a prompt gets its context");
    };
    assert!(
        text.contains(r#""action":"start""#)
            && text.contains("is completed")
            && !text.contains("protected"),
        "{text}"
    );
    let store: Value =
        serde_json::from_slice(&fs::read(env.state_dir.join("intents.json")).unwrap()).unwrap();
    let event = store["events"][store["latest"]["other"].as_str().unwrap()].clone();
    let started = propose(&env, &event, "start", None, Some(contract));
    assert_ne!(started["task_id"], done.task_id.as_str(), "{started}");
}

/// G47 (SymForge report 2026-11-10): after a task completed, the prompt hook still advised
/// `continue` with its id, and that call is refused ("completed task is immutable").
#[test]
fn a_prompt_after_completion_advises_a_new_task() {
    let (_d, mut env, _task) = autonomous();
    run_autonomous_checks(&mut env, "initial");
    assert_eq!(stop(&mut env, "Stop", "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let Decision::Context { text, .. } = handle(
        &json!({"hook_event_name":"UserPromptSubmit",
        "cwd":env.state_dir.parent().unwrap(), "session_id":"session",
        "prompt_id":"after","prompt":"Now add the other thing"}),
        &mut env,
    ) else {
        panic!("a prompt gets its context");
    };
    assert!(
        text.contains(r#""action":"start""#)
            && text.contains("is completed")
            && !text.contains(r#""action":"continue""#),
        "{text}"
    );
}
