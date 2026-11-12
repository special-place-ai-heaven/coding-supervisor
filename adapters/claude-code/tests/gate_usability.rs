//! Gate usability (G15-G20): replay of hook payloads, not live harness enforcement.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
/// A candidate whose dirty digest the test can move, as a new commit would.
struct Switch(Arc<Mutex<&'static str>>);
impl CandidateProvider for Switch {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: (*self.0.lock().unwrap()).into(),
            },
            provenance: "fixture".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}

fn contract(id: &str) -> Value {
    json!({"id":id,"goal":"usability","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]})
}

fn autonomous() -> (Fixture, Env, Arc<Mutex<&'static str>>) {
    let (d, mut env, digest) = idle();
    let event = prompt(&mut env, "start", "Implement the requested behavior");
    propose(&env, &event, "start", None, Some(contract("first")));
    (d, env, digest)
}

/// A project with a Cargo.toml and a state root, and no task yet.
fn idle() -> (Fixture, Env, Arc<Mutex<&'static str>>) {
    let d = Fixture(std::env::temp_dir().join(format!(
        "supervisor-usability-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let state = d.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(
        d.0.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let digest = Arc::new(Mutex::new("a"));
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Switch(digest.clone())),
        clock: || 100,
        level: Default::default(),
    };
    (d, env, digest)
}

fn cwd(env: &Env) -> PathBuf {
    env.state_dir.parent().unwrap().to_path_buf()
}

fn prompt(env: &mut Env, id: &str, message: &str) -> Value {
    let result = handle(
        &json!({"hook_event_name":"UserPromptSubmit","cwd":cwd(env),"session_id":"session",
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
    try_propose(env, event, action, task, contract).unwrap()
}

fn try_propose(
    env: &Env,
    event: &Value,
    action: &str,
    task: Option<&str>,
    contract: Option<Value>,
) -> Result<Value, String> {
    supervisor_claude_code::autonomy::propose(
        &env.state_dir,
        &cwd(env),
        &json!({"event_id":event["event_id"],"nonce":event["nonce"],
            "prompt_hash":event["prompt_hash"],"action":action,"task_id":task,
            "contract":contract}),
    )
}

fn bash(env: &mut Env, kind: &str, id: &str, command: &str) -> Decision {
    let mut payload = json!({"hook_event_name":kind,"cwd":cwd(env),"session_id":"session",
        "agent_id":"main","tool_use_id":id,"tool_name":"Bash","tool_input":{"command":command}});
    if kind == "PostToolUse" {
        payload["tool_response"] = json!({"interrupted":false});
    }
    handle(&payload, env)
}

/// Run every mandatory criterion of the active task once, in the foreground, exit 0.
fn run_checks(env: &mut Env, prefix: &str) {
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
        let id = format!("{prefix}-{index}");
        let command = &criterion.command[0];
        assert_eq!(bash(env, "PreToolUse", &id, command), Decision::None);
        assert_eq!(bash(env, "PostToolUse", &id, command), Decision::None);
    }
}

fn stop(env: &mut Env, agent: &str) -> Decision {
    stop_with(env, agent, false)
}

fn stop_with(env: &mut Env, agent: &str, active: bool) -> Decision {
    handle(
        &json!({"hook_event_name":"Stop","cwd":cwd(env),"session_id":"session",
        "agent_id":agent,"last_assistant_message":"Done.","stop_hook_active":active}),
        env,
    )
}

fn gate(env: &Env) -> Value {
    let dir = supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .map_or_else(|| env.state_dir.clone(), |task| task.dir);
    serde_json::from_slice(&fs::read(dir.join("gate.json")).unwrap()).unwrap()
}

/// G15: a completed task keeps the candidate it completed on, so a later commit does not
/// make its evidence look stale and the next `start` is not refused.
#[test]
fn a_completed_task_rolls_over_after_head_moves() {
    let (_d, mut env, digest) = autonomous();
    run_checks(&mut env, "first");
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    *digest.lock().unwrap() = "b";
    // Any hook call restores the task; this is where the checkpoint candidate used to drift.
    assert_eq!(
        bash(&mut env, "PreToolUse", "status", "supervisor status --json"),
        Decision::None
    );
    // A later idle Stop keeps judging the completed task on the candidate it completed on.
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    let event = prompt(&mut env, "second", "Now do the next thing");
    let started = try_propose(&env, &event, "start", None, Some(contract("second")));
    assert!(started.is_ok(), "{started:?}");
}

fn tool(env: &mut Env, kind: &str, session: &str, name: &str, id: &str, input: Value) -> Decision {
    handle(
        &json!({"hook_event_name":kind,"cwd":cwd(env),"session_id":session,"agent_id":"main",
        "tool_use_id":id,"tool_name":name,"tool_input":input}),
        env,
    )
}

/// A prompt delivered with no intent_apply call after it.
fn follow_up(env: &mut Env, session: &str, id: &str, message: &str) -> Decision {
    handle(
        &json!({"hook_event_name":"UserPromptSubmit","cwd":cwd(env),"session_id":session,
        "prompt_id":id,"prompt":message}),
        env,
    )
}

fn launch_background(env: &mut Env, id: &str, agent: &str) {
    let input = json!({"run_in_background":true,"prompt":"work"});
    assert_eq!(
        tool(env, "PreToolUse", "session", "Agent", id, input.clone()),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":cwd(env),"session_id":"session",
        "agent_id":"main","tool_use_id":id,"tool_name":"Agent","tool_input":input,
        "tool_response":{"status":"async_launched","agentId":agent}}),
        env,
    );
}

fn subagent_stop(env: &mut Env, agent: &str) -> Decision {
    handle(
        &json!({"hook_event_name":"SubagentStop","cwd":cwd(env),"session_id":"session",
        "agent_id":agent,"last_assistant_message":"Done.","stop_hook_active":false}),
        env,
    )
}

fn system_text(decision: &Decision) -> &str {
    match decision {
        Decision::System(text) => text,
        other => panic!("expected an advisory, got {other:?}"),
    }
}

/// G16: while a background Agent has not reached SubagentStop, Stop judges nothing.
#[test]
fn stop_is_not_judged_while_a_background_agent_is_outstanding() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    launch_background(&mut env, "bg-call", "bg1");
    let decision = stop(&mut env, "main");
    let text = system_text(&decision);
    assert!(text.contains("background work outstanding") && text.contains("bg1"));
    assert_ne!(gate(&env)["state"], "completed");
    // The worker finishes (whatever the gate says to it): the next Stop is judged as before.
    let _ = subagent_stop(&mut env, "bg1");
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// G16: a current pending Terminal Commander job counts as outstanding work too.
#[test]
fn an_unfetched_terminal_commander_job_keeps_the_stop_refusal() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    let project = cwd(&env);
    let input = json!({"argv":["cargo","test","--workspace"],"cwd":project});
    let name = "mcp__terminal_commander__run_and_watch";
    assert_eq!(
        tool(
            &mut env,
            "PreToolUse",
            "session",
            name,
            "tc1",
            input.clone()
        ),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":project,"session_id":"session",
        "agent_id":"main","tool_use_id":"tc1","tool_name":name,"tool_input":input,
        "tool_response":{"job_id":"job-1","bucket_id":"bucket-1"}}),
        &mut env,
    );
    // Only background Agents hold Stop back; the unfetched job keeps the G7 refusal.
    let Decision::Block(why) = stop(&mut env, "main") else {
        panic!("an unfetched job must still be refused");
    };
    assert!(
        why.contains("command_status") && why.contains("job-1"),
        "{why}"
    );
}

/// G16: a delegation whose SubagentStop was lost stops counting after two hours.
#[test]
fn an_old_delegation_no_longer_holds_stop_back() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    launch_background(&mut env, "bg-call", "bg1");
    env.clock = || 100 + 2 * 3600 + 1;
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// G17: a turn that changed nothing and was never classified is recorded as discussion.
#[test]
fn stop_records_an_unclassified_no_change_turn_as_discussion() {
    let (_d, mut env, _) = idle();
    assert!(matches!(
        follow_up(&mut env, "session", "q1", "Explain how the gate works"),
        Decision::Context { .. }
    ));
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert!(supervisor_claude_code::autonomy::discussion_turn(&env.state_dir, "session").unwrap());
}

/// G17/G18: once something was edited under an unclassified prompt, Stop still judges it.
#[test]
fn stop_still_judges_a_turn_that_edited() {
    let (_d, mut env, _) = autonomous();
    follow_up(&mut env, "session", "p2", "Also tweak the parser");
    assert_eq!(
        tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({})),
        Decision::None
    );
    let Decision::Block(why) = stop(&mut env, "main") else {
        panic!("an edit without evidence must be refused");
    };
    assert!(!why.contains("classify"), "{why}");
}

/// G18: an unclassified prompt on the session's own unfinished task is a `continue`.
#[test]
fn an_unclassified_prompt_continues_the_sessions_own_task() {
    let (_d, mut env, _) = autonomous();
    let Decision::Context { text, .. } = follow_up(&mut env, "session", "p2", "Also tweak X")
    else {
        panic!("the prompt hook must answer with context");
    };
    assert!(text.contains("continues the active task"), "{text}");
    assert_eq!(
        tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({})),
        Decision::None
    );
}

/// G18: another session's task is never continued by default.
#[test]
fn an_unclassified_prompt_does_not_continue_a_foreign_task() {
    let (_d, mut env, _) = autonomous();
    follow_up(&mut env, "other", "p2", "Also tweak X");
    assert!(matches!(
        tool(&mut env, "PreToolUse", "other", "Write", "w1", json!({})),
        Decision::Deny(_)
    ));
}

/// G19: a required check run through a pipe is allowed, with a note that it will not count.
#[test]
fn a_piped_required_check_is_allowed_with_an_advisory() {
    let (_d, mut env, _) = autonomous();
    let command = "cargo test --workspace | tail -5";
    let decision = tool(
        &mut env,
        "PreToolUse",
        "session",
        "Bash",
        "p1",
        json!({"command":command}),
    );
    let Decision::Context { event, text } = decision else {
        panic!("expected an advisory, got {decision:?}");
    };
    // The advisory survives the locked-task round trip as a PreToolUse output.
    assert_eq!(event, "PreToolUse");
    assert!(
        text.contains("AUTO_TEST will not count: run `cargo test --workspace` without a pipe"),
        "{text}"
    );
}

fn task_dir(env: &Env) -> PathBuf {
    supervisor_claude_code::lifecycle::active_task(&env.state_dir)
        .unwrap()
        .unwrap()
        .dir
}

fn journal_lines(env: &Env) -> Vec<String> {
    fs::read_to_string(task_dir(env).join("delegations.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Review 1: a worker's SubagentStop that the prompt gate lets through early (a discussion
/// turn) still ends the worker, so its delegation must close.
#[test]
fn a_delegation_closes_when_the_subagent_stop_returns_early() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    launch_background(&mut env, "bg-call", "bg1");
    follow_up(&mut env, "session", "q", "Explain the design");
    let store: Value =
        serde_json::from_slice(&fs::read(env.state_dir.join("intents.json")).unwrap()).unwrap();
    let event = store["events"][store["latest"]["session"].as_str().unwrap()].clone();
    propose(&env, &event, "discussion", None, None);
    assert_eq!(subagent_stop(&mut env, "bg1"), Decision::None);
    follow_up(&mut env, "session", "r", "Carry on with the task");
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// Review 2: a refused handback keeps the worker running, so it stays outstanding.
#[test]
fn a_refused_handback_keeps_the_delegation_outstanding() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    launch_background(&mut env, "bg-call", "bg1");
    assert_eq!(
        as_agent(&mut env, "bg1", "Write", "bg1-w", json!({})),
        Decision::None
    );
    assert!(matches!(subagent_stop(&mut env, "bg1"), Decision::Block(_)));
    let decision = stop(&mut env, "main");
    assert!(system_text(&decision).contains("bg1"), "{decision:?}");
}

/// Review 3: a delegation of another session never holds this session's Stop back.
#[test]
fn a_delegation_of_another_session_is_ignored() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    fs::write(
        task_dir(&env).join("delegations.jsonl"),
        "{\"kind\":\"delegation\",\"session_id\":\"elsewhere\",\"agent_id\":\"zz\",\"t\":100}\n",
    )
    .unwrap();
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// Review 4: an agent that mutated under an earlier prompt keeps the verification rule when
/// its handback arrives under a pending prompt.
#[test]
fn a_mutating_agent_is_still_judged_under_a_pending_prompt() {
    let (_d, mut env, _) = autonomous();
    assert_eq!(
        as_agent(&mut env, "w1", "Write", "w1-call", json!({})),
        Decision::None
    );
    follow_up(&mut env, "session", "c", "cancel");
    let decision = subagent_stop(&mut env, "w1");
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
}

/// Review 5: at audit a denial becomes a message and the tool still runs, so it is a mutation.
#[test]
fn an_audit_level_denial_still_journals_the_mutation() {
    let (_d, mut env, _) = autonomous();
    env.level = supervisor_claude_code::level::Setting {
        level: supervisor_claude_code::level::Level::Audit,
        ..Default::default()
    };
    follow_up(&mut env, "session", "c", "cancel");
    let decision = as_agent(&mut env, "w1", "Write", "w1-call", json!({}));
    assert!(matches!(decision, Decision::System(_)), "{decision:?}");
    let lines = journal_lines(&env);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"mutation\"") && l.contains("\"w1\"")),
        "{lines:?}"
    );
}

/// Review 6: one mutation row per agent, and a malformed line never blocks anything.
#[test]
fn the_mutation_row_is_written_once_per_agent() {
    let (_d, mut env, _) = autonomous();
    for call in ["a", "b", "c"] {
        assert_eq!(
            as_agent(&mut env, "w1", "Write", call, json!({})),
            Decision::None
        );
    }
    let rows = journal_lines(&env)
        .into_iter()
        .filter(|l| l.contains("\"mutation\""))
        .count();
    assert_eq!(rows, 1);
}

#[test]
fn a_malformed_journal_line_fails_closed_per_meaning() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    fs::write(task_dir(&env).join("delegations.jsonl"), "{not json\n").unwrap();
    // Not outstanding for the chair's Stop ...
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
    // ... and "this agent mutated" for a handback.
    let decision = subagent_stop(&mut env, "rev9");
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
}

/// Review 8: a short prompt that mentions a control word keeps the prompt pending.
#[test]
fn a_short_prompt_with_a_control_word_is_not_continued_by_default() {
    for message in ["please cancel this", "pause, I want to review", "stop"] {
        let (_d, mut env, _) = autonomous();
        follow_up(&mut env, "session", "p2", message);
        let decision = tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({}));
        assert!(
            matches!(decision, Decision::Deny(_)),
            "{message}: {decision:?}"
        );
    }
}

/// Review 9: Stop on a completed task whose tree changed warns, and does not block.
#[test]
fn a_completed_task_warns_when_the_tree_changed() {
    let (_d, mut env, digest) = autonomous();
    run_checks(&mut env, "first");
    assert_eq!(stop(&mut env, "main"), Decision::None);
    let runs = task_dir(&env).join("runs.jsonl");
    let rows: Vec<String> = fs::read_to_string(&runs)
        .unwrap()
        .lines()
        .map(|l| {
            let mut row: Value = serde_json::from_str(l).unwrap();
            row["tree"] = "T0".into();
            row.to_string()
        })
        .collect();
    fs::write(&runs, format!("{}\n", rows.join("\n"))).unwrap();
    *digest.lock().unwrap() = "b";
    let decision = stop(&mut env, "main");
    assert!(
        system_text(&decision).contains("tree changed since completion"),
        "{decision:?}"
    );
    assert_eq!(gate(&env)["state"], "completed");
}

fn as_agent(env: &mut Env, agent: &str, name: &str, id: &str, input: Value) -> Decision {
    handle(
        &json!({"hook_event_name":"PreToolUse","cwd":cwd(env),"session_id":"session",
        "agent_id":agent,"tool_use_id":id,"tool_name":name,"tool_input":input}),
        env,
    )
}

/// G21: StructuredOutput only returns a value, so it is read-only even before classification.
#[test]
fn structured_output_is_read_only_under_an_unclassified_prompt() {
    let (_d, mut env, _) = idle();
    follow_up(&mut env, "session", "q1", "Review the change");
    let decision = tool(
        &mut env,
        "PreToolUse",
        "session",
        "StructuredOutput",
        "s1",
        json!({}),
    );
    assert!(!matches!(decision, Decision::Deny(_)), "{decision:?}");
}

/// G21: a handback from an agent that mutated nothing is a read-only report.
#[test]
fn a_read_only_agents_handback_is_allowed_without_verification() {
    let (_d, mut env, _) = autonomous();
    let read = as_agent(&mut env, "rev1", "Read", "r1", json!({}));
    assert_eq!(read, Decision::None);
    assert_eq!(subagent_stop(&mut env, "rev1"), Decision::None);
    assert_ne!(gate(&env)["state"], "completed");
}

/// G21: an agent that edited keeps the verification rule.
#[test]
fn an_agent_that_edited_is_still_refused_without_verification() {
    let (_d, mut env, _) = autonomous();
    assert_eq!(
        as_agent(&mut env, "w1", "Write", "w1-call", json!({})),
        Decision::None
    );
    let decision = subagent_stop(&mut env, "w1");
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(command.output().unwrap().status.success(), "git {args:?}");
}

/// An active task in a committed git project (state directory ignored).
fn autonomous_in_git() -> (Fixture, Env) {
    let (d, env, _) = autonomous();
    fs::write(d.0.join(".gitignore"), "state/\n").unwrap();
    git(&d.0, &["init", "--quiet"]);
    git(&d.0, &["add", "-A"]);
    git(
        &d.0,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    (d, env)
}

/// G44 (field brief 2026-11-09, change 2): a research agent used tools outside the read-only
/// list (a web fetch, a shell search) and changed nothing. Its report is its last message: the
/// handback is not judged, so the agent is never pushed into another turn.
#[test]
fn an_agent_that_left_the_tree_unchanged_hands_back_a_report() {
    let (_d, mut env) = autonomous_in_git();
    for (name, id, input) in [
        (
            "WebFetch",
            "f1",
            json!({"url":"https://example.invalid/docs"}),
        ),
        (
            "Bash",
            "b1",
            json!({"command":"grep -rn session src | head -5"}),
        ),
    ] {
        let allowed = as_agent(&mut env, "res1", name, id, input);
        assert!(
            !matches!(allowed, Decision::Deny(_) | Decision::Block(_)),
            "{name}: {allowed:?}"
        );
    }
    assert_eq!(subagent_stop(&mut env, "res1"), Decision::None);
    assert_ne!(gate(&env)["state"], "completed");
}

/// The same agent stopping after the user sent a new, still unclassified prompt.
#[test]
fn an_unchanged_tree_handback_passes_under_an_unclassified_prompt() {
    let (_d, mut env) = autonomous_in_git();
    let allowed = as_agent(
        &mut env,
        "res2",
        "WebFetch",
        "f2",
        json!({"url":"https://example.invalid/docs"}),
    );
    assert!(!matches!(allowed, Decision::Deny(_)), "{allowed:?}");
    follow_up(
        &mut env,
        "session",
        "q2",
        "And what about the other module?",
    );
    assert_eq!(subagent_stop(&mut env, "res2"), Decision::None);
}

/// The protection that stays: an agent after whose tools the tree differs is held to the
/// criteria, whatever tool made the change.
#[test]
fn an_agent_that_changed_the_tree_is_still_judged() {
    let (d, mut env) = autonomous_in_git();
    let allowed = as_agent(
        &mut env,
        "w2",
        "Bash",
        "b2",
        json!({"command":"python fix.py"}),
    );
    assert!(!matches!(allowed, Decision::Deny(_)), "{allowed:?}");
    fs::write(d.0.join("changed.rs"), "// the agent's edit\n").unwrap();
    let decision = subagent_stop(&mut env, "w2");
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
}

/// G20: the discovered test floor accepts nextest as well, `cargo test` stays the advised one.
#[test]
fn the_test_floor_accepts_nextest_too() {
    let (d, _env, _) = idle();
    let floor = supervisor_claude_code::autonomy::repository_floor(&d.0).unwrap();
    let test = floor.iter().find(|c| c["id"] == "AUTO_TEST").unwrap();
    assert_eq!(
        test["command"],
        json!(["cargo test --workspace", "cargo nextest run --workspace"])
    );
}

/// G20: a proposal that keeps only one of the floor's test commands still starts.
#[test]
fn a_proposal_may_keep_one_of_the_floor_commands() {
    for kept in ["cargo test --workspace", "cargo nextest run --workspace"] {
        let (_d, mut env, _) = idle();
        let event = prompt(&mut env, "start", "Implement the requested behavior");
        let mut contract = contract("first");
        contract["criteria"].as_array_mut().unwrap().push(json!(
            {"id":"AUTO_TEST","check":"repository_validation","mandatory":true,"command":kept}
        ));
        let started = try_propose(&env, &event, "start", None, Some(contract));
        assert!(started.is_ok(), "{kept}: {started:?}");
    }
}

/// G31: saving memory (the harness's memory notes, EverOS) is not project work, so neither an
/// unclassified nor a discussion prompt refuses it; a project write is still refused.
#[test]
fn memory_saves_are_allowed_under_discussion_and_unclassified_prompts() {
    let (_d, mut env, _) = idle();
    let home = PathBuf::from(
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap(),
    );
    let note = home.join(".claude/projects/p/memory/note.md");
    let save = |env: &mut Env, id: &str| {
        tool(
            env,
            "PreToolUse",
            "session",
            "Write",
            id,
            json!({"file_path":note,"content":"x"}),
        )
    };
    follow_up(
        &mut env,
        "session",
        "q1",
        "Remember that reviews use orch-review",
    );
    assert!(!matches!(save(&mut env, "m1"), Decision::Deny(_)));
    for name in ["mcp__everos__memory_add", "mcp__everos__memory_flush"] {
        let decision = tool(&mut env, "PreToolUse", "session", name, name, json!({}));
        assert!(
            !matches!(decision, Decision::Deny(_)),
            "{name}: {decision:?}"
        );
    }
    let event = supervisor_claude_code::autonomy::pending(&env.state_dir, "session")
        .unwrap()
        .unwrap();
    propose(&env, &event, "discussion", None, None);
    assert!(!matches!(save(&mut env, "m2"), Decision::Deny(_)));
    let project = cwd(&env).join("src/lib.rs");
    let edit = tool(
        &mut env,
        "PreToolUse",
        "session",
        "Write",
        "p1",
        json!({"file_path":project,"content":"x"}),
    );
    assert!(matches!(edit, Decision::Deny(_)), "{edit:?}");
    let escape = home.join(".claude/projects/p/memory/../../../settings.json");
    let escape = tool(
        &mut env,
        "PreToolUse",
        "session",
        "Write",
        "e1",
        json!({"file_path":escape,"content":"x"}),
    );
    assert!(matches!(escape, Decision::Deny(_)), "{escape:?}");
    // Review: only the harness memory root under the home directory counts, not a look-alike
    // path inside the project, a relative one or a share.
    let look_alikes = [
        cwd(&env).join(".claude/projects/p/memory/a.md"),
        PathBuf::from(".claude/projects/p/memory/a.md"),
        PathBuf::from("//server/share/.claude/projects/p/memory/a.md"),
    ];
    for path in look_alikes {
        let decision = tool(
            &mut env,
            "PreToolUse",
            "session",
            "Write",
            "l1",
            json!({"file_path":path,"content":"x"}),
        );
        assert!(
            matches!(decision, Decision::Deny(_)),
            "{path:?}: {decision:?}"
        );
    }
}

/// G27: a still-running Terminal Commander job is refused once; the next stop in the same
/// turn is released with a note naming the job, and nothing is certified.
#[test]
fn a_pending_terminal_commander_job_refuses_stop_once_then_waits() {
    let (_d, mut env, _) = autonomous();
    let project = cwd(&env);
    let input = json!({"argv":["cargo","test"],"cwd":project});
    let name = "mcp__terminal_commander__command_start_combed";
    assert_eq!(
        tool(
            &mut env,
            "PreToolUse",
            "session",
            name,
            "tc1",
            input.clone()
        ),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":project,"session_id":"session",
        "agent_id":"main","tool_use_id":"tc1","tool_name":name,"tool_input":input,
        "tool_response":{"job_id":"job-1","bucket_id":"bucket-1"}}),
        &mut env,
    );
    let Decision::Block(first) = stop_with(&mut env, "main", false) else {
        panic!("the first stop must name the pending job");
    };
    assert!(first.contains("job-1"), "{first}");
    assert!(
        !first.contains("report the task as not complete"),
        "{first}"
    );
    let second = stop_with(&mut env, "main", true);
    let Decision::System(note) = &second else {
        panic!("the second stop must be released, got {second:?}");
    };
    assert!(
        note.contains("job-1") && note.contains("command_status"),
        "{note}"
    );
    assert_ne!(gate(&env)["state"], "completed");
}

/// G58: a stop held only for a job with no recorded exit is not a numbered refusal, and a tool
/// call between it and the next stop (which resets the refusal count) does not make the gate
/// refuse again: the harness's continuation flag releases that stop with the note.
#[test]
fn a_pending_job_refusal_is_not_counted_and_tool_calls_do_not_rearm_it() {
    let (_d, mut env, _) = autonomous();
    let project = cwd(&env);
    let input = json!({"argv":["cargo","test"],"cwd":project});
    let name = "mcp__terminal_commander__command_start_combed";
    assert_eq!(
        tool(
            &mut env,
            "PreToolUse",
            "session",
            name,
            "tc1",
            input.clone()
        ),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":project,"session_id":"session",
        "agent_id":"main","tool_use_id":"tc1","tool_name":name,"tool_input":input,
        "tool_response":{"job_id":"job-1","bucket_id":"bucket-1"}}),
        &mut env,
    );
    let Decision::Block(first) = stop_with(&mut env, "main", false) else {
        panic!("the first stop must name the pending job");
    };
    assert!(
        first.contains("job-1") && first.contains("no recorded exit yet"),
        "{first}"
    );
    assert!(!first.contains("refusal 1 of 3"), "{first}");
    // A judged tool call resets the refusal count, as before.
    assert_eq!(
        tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({})),
        Decision::None
    );
    let second = stop_with(&mut env, "main", true);
    let Decision::System(note) = &second else {
        panic!("the continued stop must be released, got {second:?}");
    };
    assert!(
        note.contains("job-1") && note.contains("command_status"),
        "{note}"
    );
    assert_ne!(gate(&env)["state"], "completed");
}

/// Start `cargo test` as a Terminal Commander job whose exit is never fetched.
fn start_job(env: &mut Env, id: &str, job: &str) {
    let input = json!({"argv":["cargo","test"],"cwd":cwd(env)});
    let name = "mcp__terminal_commander__command_start_combed";
    assert_eq!(
        tool(env, "PreToolUse", "session", name, id, input.clone()),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":cwd(env),"session_id":"session",
        "agent_id":"main","tool_use_id":id,"tool_name":name,"tool_input":input,
        "tool_response":{"job_id":job,"bucket_id":"bucket-1"}}),
        env,
    );
}

/// G58 review: the continuation flag releases a pending-only stop only when the gate's own
/// pending hold came before it. After a numbered refusal, an edit and a restarted check, the
/// continued stop is held and names the job and the `command_status` call.
#[test]
fn a_continued_stop_after_a_numbered_refusal_is_held_for_a_pending_job() {
    let (_d, mut env, _) = autonomous();
    let Decision::Block(first) = stop(&mut env, "main") else {
        panic!("a stop with no check run must be refused");
    };
    assert!(first.contains("refusal 1 of 3"), "{first}");
    assert_eq!(
        tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({})),
        Decision::None
    );
    start_job(&mut env, "tc1", "job-1");
    let second = stop_with(&mut env, "main", true);
    let Decision::Block(held) = &second else {
        panic!("the continued stop must be held for the pending job, got {second:?}");
    };
    assert!(
        held.contains("no recorded exit yet")
            && held.contains("job-1")
            && held.contains("command_status"),
        "{held}"
    );
}

/// G58 review: a released hold is used up, so a continued stop in a later turn, after a block
/// the gate did not give, is held again rather than released.
#[test]
fn a_released_hold_does_not_release_a_later_continued_stop() {
    let (_d, mut env, _) = autonomous();
    start_job(&mut env, "tc1", "job-1");
    let Decision::Block(first) = stop(&mut env, "main") else {
        panic!("the first stop must name the pending job");
    };
    assert!(first.contains("no recorded exit yet"), "{first}");
    assert_eq!(
        tool(&mut env, "PreToolUse", "session", "Write", "w1", json!({})),
        Decision::None
    );
    assert!(matches!(
        stop_with(&mut env, "main", true),
        Decision::System(_)
    ));
    assert!(matches!(
        follow_up(&mut env, "session", "later", "Keep going on the same work"),
        Decision::Context { .. }
    ));
    let later = stop_with(&mut env, "main", true);
    let Decision::Block(held) = &later else {
        panic!("a continued stop with no hold before it must be held, got {later:?}");
    };
    assert!(
        held.contains("no recorded exit yet")
            && held.contains("job-1")
            && held.contains("command_status"),
        "{held}"
    );
}

/// G25: the chair records the exit of a Terminal Commander job its worker started in the
/// same session, and that run then counts for the chair's Stop.
#[test]
fn the_chair_records_the_exit_of_a_workers_terminal_commander_job() {
    let (_d, mut env, _) = autonomous();
    run_checks(&mut env, "first");
    let project = cwd(&env);
    let input = json!({"argv":["cargo","test"],"cwd":project});
    let start = "mcp__terminal_commander__command_start_combed";
    assert_eq!(
        as_agent(&mut env, "w1", start, "tc1", input.clone()),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":project,"session_id":"session",
        "agent_id":"w1","tool_use_id":"tc1","tool_name":start,"tool_input":input,
        "tool_response":{"job_id":"job-w","bucket_id":"bucket-w"}}),
        &mut env,
    );
    let status = "mcp__terminal_commander__command_status";
    let status_input = json!({"job_id":"job-w"});
    assert_eq!(
        tool(
            &mut env,
            "PreToolUse",
            "session",
            status,
            "st1",
            status_input.clone()
        ),
        Decision::None
    );
    handle(
        &json!({"hook_event_name":"PostToolUse","cwd":project,"session_id":"session",
        "agent_id":"main","tool_use_id":"st1","tool_name":status,"tool_input":status_input,
        "tool_response":{"job_id":"job-w","bucket_id":"bucket-w","state":"exited",
            "complete":true,"exit_code":0,"outcome_trust":"observed","degraded":false,
            "pipeline_exit_masked":false}}),
        &mut env,
    );
    assert_eq!(stop(&mut env, "main"), Decision::None);
    assert_eq!(gate(&env)["state"], "completed");
}

/// G24(b): when the gate cannot journal a mutating tool, the agent would later read as
/// read-only; the tool is refused instead (fail closed), with advice to retry.
#[test]
fn a_failed_mutation_journal_write_denies_the_tool() {
    let (_d, mut env, _) = autonomous();
    fs::create_dir_all(task_dir(&env).join("delegations.jsonl")).unwrap();
    let decision = as_agent(&mut env, "w1", "Write", "w1-call", json!({}));
    let Decision::Deny(why) = &decision else {
        panic!("an unjournaled mutation must be refused, got {decision:?}");
    };
    assert!(why.contains("retry"), "{why}");
}

/// G36 (AAP report): a background-task notice delivered as a prompt registers nothing; the
/// previous prompt's classification stands.
#[test]
fn a_task_notification_prompt_is_not_classified() {
    let (_d, mut env, _) = idle();
    let first = prompt(&mut env, "q1", "Review the change");
    let notice = follow_up(
        &mut env,
        "session",
        "n1",
        "<task-notification>\n<task-id>x</task-id>\n</task-notification>",
    );
    assert_eq!(notice, Decision::None);
    let pending = supervisor_claude_code::autonomy::pending(&env.state_dir, "session")
        .unwrap()
        .unwrap();
    assert_eq!(pending["event_id"], first["event_id"]);
}

/// G35 (AAP report): a repository with no discoverable checks gets one short line naming the
/// person's command and what blocks at the level, not the full proposal block.
#[test]
fn a_checkless_repository_gets_one_short_line() {
    let (_d, mut env, _) = idle();
    fs::remove_file(cwd(&env).join("Cargo.toml")).unwrap();
    let Decision::Context { text, .. } = follow_up(&mut env, "session", "q1", "Fix the bug") else {
        panic!("a prompt context is expected");
    };
    assert!(text.contains("supervisor task add-check"), "{text}");
    assert!(text.contains("supervisor intent apply"), "{text}");
    assert!(!text.contains("requires an intent proposal"), "{text}");
}
