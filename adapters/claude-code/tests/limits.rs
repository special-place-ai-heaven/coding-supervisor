//! WP4: task time and turn limits. A turn is one PreToolUse call judged for the task (it reached
//! the task gate and appended a task events.jsonl line); wall clock runs from `approval.at` on the
//! hook clock. At a limit new work is refused, read-only calls still run, a stop goes ahead NOT
//! certified with a report, and nothing in the work tree or the journal is rewritten.
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId, TaskState};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::CandidateProvider;
use supervisor_claude_code::level::{Level, Setting};
use supervisor_claude_code::state::{self, Stored};
use supervisor_claude_code::{autonomy, lifecycle};

static NEXT: AtomicU64 = AtomicU64::new(0);
const APPROVED_AT: u64 = 1_000;

thread_local! {
    /// The hook clock of the test running on this thread.
    static NOW: Cell<u64> = const { Cell::new(APPROVED_AT) };
}
fn clock() -> u64 {
    NOW.with(Cell::get)
}
fn set_clock(t: u64) {
    NOW.with(|n| n.set(t));
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!(
        "supervisor-limits-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("src")).unwrap();
    Dir(path)
}

struct Fake;
impl CandidateProvider for Fake {
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

/// A person's v2 contract approved at `APPROVED_AT` with `limits`.
fn contract(limits: Value) -> Value {
    json!({"schema_version":2,"id":"wp4","goal":"limits","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,"base_sha":null,
        "approval":{"by":"person","at":APPROVED_AT,"via":"task-start"},
        "limits":limits,"artifacts":[],"review":{"required":false,"independent":false},
        "isolation":"none",
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]})
}

/// The contract is written straight into the state root, so `approval.at` is on the fake clock.
fn env(d: &Dir, contract: &Value) -> Env {
    set_clock(APPROVED_AT);
    let state = d.0.join(".supervisor");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("contract.json"), format!("{contract}\n")).unwrap();
    Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Fake),
        clock,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    }
}
fn cwd(env: &Env) -> &Path {
    env.state_dir.parent().unwrap()
}
fn tool(env: &Env, id: &str, name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","cwd":cwd(env),"session_id":"session","agent_id":"main",
        "tool_use_id":id,"tool_name":name,"tool_input":input})
}
fn write(env: &Env, id: &str) -> Value {
    let path = cwd(env).join("src").join("lib.rs");
    tool(env, id, "Write", json!({"file_path":path,"content":"x"}))
}
fn read(env: &Env, id: &str) -> Value {
    let path = cwd(env).join("src").join("lib.rs");
    tool(env, id, "Read", json!({"file_path":path}))
}
fn bash(env: &Env, id: &str, command: &str) -> Value {
    tool(env, id, "Bash", json!({"command":command}))
}
fn stop(env: &Env, message: &str) -> Value {
    json!({"hook_event_name":"Stop","cwd":cwd(env),"session_id":"session",
        "stop_hook_active":false,"last_assistant_message":message})
}
fn journal(env: &Env) -> Vec<u8> {
    fs::read(env.state_dir.join("events.jsonl")).unwrap_or_default()
}
fn pre_tool_lines(env: &Env) -> usize {
    String::from_utf8(journal(env))
        .unwrap()
        .lines()
        .filter(|l| serde_json::from_str::<Value>(l).unwrap()["event"] == "PreToolUse")
        .count()
}
fn gate_state(env: &Env) -> TaskState {
    let (_, digest) = state::read_contract(&env.state_dir).unwrap();
    match state::load_gate(&env.state_dir, &digest).unwrap() {
        Stored::Valid(gate) => gate.checkpoint.state,
        _ => panic!("no valid gate"),
    }
}
fn set_gate_state(env: &Env, to: TaskState) {
    let (_, digest) = state::read_contract(&env.state_dir).unwrap();
    let Stored::Valid(mut gate) = state::load_gate(&env.state_dir, &digest).unwrap() else {
        panic!("no valid gate");
    };
    gate.checkpoint.state = to;
    state::save_gate(&env.state_dir, &gate, &digest).unwrap();
}
fn limit_denial(decision: &Decision) -> bool {
    matches!(decision, Decision::Deny(why) if why.contains("task limit reached"))
}
fn plain(text: &str) {
    let lower = text.to_lowercase();
    for word in ["missing", "not found", "no such file"] {
        assert!(!lower.contains(word), "{word:?} in {text}");
    }
}

#[test]
fn the_turn_limit_admits_exactly_its_number_of_judged_calls() {
    let d = dir("boundary");
    let mut env = env(&d, &contract(json!({"turns":3})));
    for id in ["t1", "t2", "t3"] {
        let decision = handle(&write(&env, id), &mut env);
        assert_eq!(decision, Decision::None, "{id}");
    }
    assert_eq!(pre_tool_lines(&env), 3);
    let fourth = handle(&write(&env, "t4"), &mut env);
    let Decision::Deny(why) = &fourth else {
        panic!("fourth call admitted: {fourth:?}");
    };
    assert!(
        why.contains("task limit reached: 3 of 3 turns used"),
        "{why}"
    );
    assert!(why.contains("read-only tools still work"), "{why}");
    plain(why);
    assert_eq!(gate_state(&env), TaskState::BudgetExhausted);
}

#[test]
fn read_only_calls_still_run_at_the_limit() {
    let d = dir("read-only");
    let mut env = env(&d, &contract(json!({"turns":1})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    assert!(limit_denial(&handle(&write(&env, "t2"), &mut env)));
    for (id, payload) in [
        ("r1", read(&env, "r1")),
        ("r2", bash(&env, "r2", "git status")),
    ] {
        let decision = handle(&payload, &mut env);
        assert!(
            !matches!(decision, Decision::Deny(_)),
            "{id} denied: {decision:?}"
        );
    }
}

#[test]
fn a_shell_mutation_is_denied_at_the_limit_for_a_non_autonomous_task() {
    let d = dir("bash");
    let mut env = env(&d, &contract(json!({"turns":1})));
    assert_eq!(handle(&read(&env, "r1"), &mut env), Decision::None);
    let decision = handle(&bash(&env, "b1", "echo x > src/out.txt"), &mut env);
    assert!(limit_denial(&decision), "{decision:?}");
    let decision = handle(
        &tool(
            &env,
            "p1",
            "PowerShell",
            json!({"command":"Set-Content src/out.txt x"}),
        ),
        &mut env,
    );
    assert!(limit_denial(&decision), "{decision:?}");
}

#[test]
fn the_wall_clock_is_checked_at_stop_with_no_tool_call_in_between() {
    let d = dir("wall-stop");
    let mut env = env(&d, &contract(json!({"wall_clock_s":600})));
    set_clock(APPROVED_AT + 700);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    let Decision::System(report) = &decision else {
        panic!("stop not released with a report: {decision:?}");
    };
    for part in [
        "task limit reached",
        "elapsed 700 s of 600 s",
        "NOT certified",
        "turns used 0",
        "remaining criteria: C2",
        "work tree and logs are kept",
    ] {
        assert!(report.contains(part), "{part:?} in {report}");
    }
    plain(report);
    assert_eq!(gate_state(&env), TaskState::BudgetExhausted);
    // A later tool call is refused on the same limit.
    let decision = handle(&write(&env, "t1"), &mut env);
    assert!(
        matches!(&decision, Decision::Deny(why) if why.contains("elapsed 700 s of 600 s")),
        "{decision:?}"
    );
}

#[test]
fn a_stop_at_the_turn_limit_goes_ahead_with_the_report_every_time() {
    let d = dir("turn-stop");
    let mut env = env(&d, &contract(json!({"turns":1,"wall_clock_s":3600})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    set_clock(APPROVED_AT + 42);
    for message in ["Done. All checks pass.", "Stopping here."] {
        let decision = handle(&stop(&env, message), &mut env);
        let Decision::System(report) = &decision else {
            panic!("stop not released with a report: {decision:?}");
        };
        assert!(report.contains("1 of 1 turns used"), "{report}");
        assert!(report.contains("elapsed 42 s"), "{report}");
        assert!(report.contains("turns used 1"), "{report}");
        assert!(report.contains("remaining criteria: C2"), "{report}");
        plain(report);
    }
}

#[test]
fn a_worker_handback_at_the_limit_is_released_silently() {
    let d = dir("subagent");
    let mut env = env(&d, &contract(json!({"turns":1})));
    let mut first = write(&env, "w1");
    first["agent_id"] = "worker".into();
    assert_eq!(handle(&first, &mut env), Decision::None);
    let handback = json!({"hook_event_name":"SubagentStop","cwd":cwd(&env),"session_id":"session",
        "agent_id":"worker","stop_hook_active":false,"last_assistant_message":"done"});
    // Any output on SubagentStop keeps the worker running, so the release says nothing.
    assert_eq!(handle(&handback, &mut env), Decision::None);
    assert_eq!(gate_state(&env), TaskState::BudgetExhausted);
}

#[test]
fn repair_round_exhaustion_keeps_its_refusal_when_no_limit_is_reached() {
    for limits in [json!({}), json!({"turns":100,"wall_clock_s":3600})] {
        let d = dir("rounds");
        let mut env = env(&d, &contract(limits.clone()));
        assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
        set_gate_state(&env, TaskState::BudgetExhausted);
        let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
        assert!(
            matches!(&decision, Decision::Block(why) if why.contains("budget_exhausted") && !why.contains("limit")),
            "{limits}: {decision:?}"
        );
    }
}

#[test]
fn a_completed_task_never_exhausts() {
    let d = dir("completed");
    let mut env = env(&d, &contract(json!({"turns":1,"wall_clock_s":10})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    set_gate_state(&env, TaskState::Completed);
    set_clock(APPROVED_AT + 100);
    let decision = handle(&write(&env, "t2"), &mut env);
    assert!(!limit_denial(&decision), "{decision:?}");
    let decision = handle(&stop(&env, "Stopping here."), &mut env);
    assert!(
        !matches!(&decision, Decision::System(text) if text.contains("limit")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

/// Spec section 9 case 17: the journal before the limit is a byte prefix of the journal after the
/// denied call and the stop, and a file written before the limit is byte-identical.
#[test]
fn the_journal_is_only_appended_and_the_work_tree_is_kept() {
    let d = dir("kept");
    let mut env = env(&d, &contract(json!({"turns":1})));
    let file = d.0.join("src").join("lib.rs");
    fs::write(&file, b"before the limit\n").unwrap();
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    let before = journal(&env);
    assert!(limit_denial(&handle(&write(&env, "t2"), &mut env)));
    assert!(limit_denial(&handle(
        &bash(&env, "b1", "rm src/lib.rs"),
        &mut env
    )));
    assert!(matches!(
        handle(&stop(&env, "Done."), &mut env),
        Decision::System(_)
    ));
    let after = journal(&env);
    assert!(
        after.len() > before.len(),
        "nothing journaled after the limit"
    );
    assert_eq!(&after[..before.len()], &before[..], "journal rewritten");
    assert_eq!(fs::read(&file).unwrap(), b"before the limit\n");
}

#[test]
fn without_an_approval_time_there_is_no_wall_clock_limit() {
    let d = dir("no-approval");
    let mut c = contract(json!({"wall_clock_s":1}));
    c.as_object_mut().unwrap().remove("approval");
    let mut env = env(&d, &c);
    set_clock(APPROVED_AT + 1_000_000);
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    let decision = handle(&stop(&env, "Stopping here."), &mut env);
    assert!(
        !matches!(&decision, Decision::System(text) if text.contains("limit")),
        "{decision:?}"
    );
}

#[test]
fn a_cached_replay_is_not_a_turn() {
    let d = dir("replay");
    let mut env = env(&d, &contract(json!({"turns":2})));
    let first = write(&env, "t1");
    assert_eq!(handle(&first, &mut env), Decision::None);
    assert_eq!(handle(&first, &mut env), Decision::None);
    assert_eq!(pre_tool_lines(&env), 1, "the replay appended a line");
    assert_eq!(handle(&write(&env, "t2"), &mut env), Decision::None);
    assert!(limit_denial(&handle(&write(&env, "t3"), &mut env)));
}

#[test]
fn an_unreadable_journal_fails_closed_when_a_turn_limit_is_set() {
    let d = dir("unreadable");
    let mut env = env(&d, &contract(json!({"turns":5})));
    fs::write(env.state_dir.join("events.jsonl"), "not a journal line\n").unwrap();
    let decision = handle(&write(&env, "t1"), &mut env);
    assert!(
        matches!(&decision, Decision::Deny(why) if why.contains("fails closed")),
        "{decision:?}"
    );
    // Without a turn limit the journal is not read for limits.
    let d = dir("unreadable-unlimited");
    let mut unlimited = self::env(&d, &contract(json!({})));
    fs::write(
        unlimited.state_dir.join("events.jsonl"),
        "not a journal line\n",
    )
    .unwrap();
    assert_eq!(
        handle(&write(&unlimited, "t1"), &mut unlimited),
        Decision::None
    );
}

#[test]
fn reset_rounds_is_refused_while_a_limit_is_reached() {
    let d = dir("reset");
    let root = d.0.join("state");
    let mut request = contract(json!({"turns":1}));
    request.as_object_mut().unwrap().remove("approval");
    let started = lifecycle::start_as(
        &root,
        &state::Writer::task_start(None),
        &request,
        false,
        None,
        None,
    )
    .unwrap();
    let id = started["task_id"].as_str().unwrap().to_owned();
    lifecycle::with_task(&root, Some(&id), None, None, |stage| {
        let (_, digest) = state::read_contract(stage)?;
        let gate = state::Gate::new(
            "wp4".to_owned(),
            coding_supervisor::Checkpoint {
                state: TaskState::BudgetExhausted,
                candidate: None,
                open_findings: vec![],
                consumed_repair_rounds: 2,
                next_action: String::new(),
            },
        );
        state::save_gate(stage, &gate, &digest)?;
        state::append_line(
            &stage.join("events.jsonl"),
            &json!({"v":1,"event":"PreToolUse"}),
        )?;
        Ok(Value::Null)
    })
    .unwrap();
    let why = lifecycle::reset_rounds(&root, &id, "flaky check").unwrap_err();
    assert!(why.contains("1 of 1 turns used"), "{why}");
    assert!(why.contains("supervisor task next"), "{why}");
    plain(&why);
}

fn git(project: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(project)
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// An agent may limit itself at start; a revision may lower or add a limit, never raise or
/// remove one.
#[test]
fn an_agent_revision_cannot_raise_or_remove_a_limit() {
    let d = dir("preserves");
    let (project, root) = (d.0.join("project"), d.0.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "Cargo.toml"]);
    git(&project, &["commit", "-q", "-m", "one"]);
    let prompts = AtomicU64::new(0);
    let apply = |action: &str, task: Option<&str>, contract: Value| {
        let n = prompts.fetch_add(1, Ordering::Relaxed);
        let event = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":format!("p{n}"),"prompt":"Implement a small change"}),
        )
        .unwrap();
        autonomy::propose(
            &root,
            &project,
            &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,"task_id":task,"contract":contract}),
        )
    };
    let proposal = json!({"id":"wp4","goal":"limits","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,"limits":{"turns":10,"wall_clock_s":600},
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test -p fixture"}]});
    let started = apply("start", None, proposal).unwrap();
    let id = started["task_id"].as_str().unwrap().to_owned();
    let task = root.join("tasks").join(&id);
    let written = || -> Value {
        serde_json::from_str(&fs::read_to_string(task.join("contract.json")).unwrap()).unwrap()
    };
    assert_eq!(
        written()["limits"],
        json!({"turns":10,"wall_clock_s":600}),
        "a self-imposed limit is kept at start"
    );
    for (limits, what) in [
        (json!({"turns":11,"wall_clock_s":600}), "raised turns"),
        (json!({"turns":10,"wall_clock_s":601}), "raised wall clock"),
        (json!({"wall_clock_s":600}), "removed turns"),
        (json!({"turns":10}), "removed wall clock"),
        (Value::Null, "removed both"),
    ] {
        let mut revised = written();
        match limits {
            Value::Null => {
                revised.as_object_mut().unwrap().remove("limits");
            }
            limits => revised["limits"] = limits,
        }
        let result = apply("revise", Some(&id), revised);
        let why = result.expect_err(what);
        assert!(why.contains("limit"), "{what}: {why}");
    }
    let mut lowered = written();
    lowered["limits"] = json!({"turns":5,"wall_clock_s":300});
    apply("revise", Some(&id), lowered).unwrap();
    assert_eq!(written()["limits"], json!({"turns":5,"wall_clock_s":300}));
}
