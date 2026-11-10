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
    assert!(why.contains("still allowed: read-only tools"), "{why}");
    plain(why);
    // A refusal changes no task state: both limits are recomputed at every call.
    assert_eq!(gate_state(&env), TaskState::Verifying);
}

/// At a limit only read-only tools, intent proposals and the supervisor's own read-only status
/// commands run. Routes that can run a program the agent prepared (git's diff drivers and
/// textconv, a `--version` found on PATH) and memory saves are refused.
#[test]
fn only_read_only_tools_proposals_and_status_run_at_the_limit() {
    let d = dir("read-only");
    let mut env = env(&d, &contract(json!({"turns":1})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    assert!(limit_denial(&handle(&write(&env, "t2"), &mut env)));
    for (id, payload) in [
        ("r1", read(&env, "r1")),
        (
            "r2",
            tool(&env, "r2", "Grep", json!({"pattern":"x","path":"src"})),
        ),
        ("r3", bash(&env, "r3", "supervisor status --json")),
        ("r4", bash(&env, "r4", "supervisor level")),
        (
            "r5",
            bash(&env, "r5", "supervisor intent apply --proposal '{}'"),
        ),
    ] {
        let decision = handle(&payload, &mut env);
        assert!(
            !matches!(decision, Decision::Deny(_)),
            "{id} denied: {decision:?}"
        );
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap();
    let note = Path::new(&home)
        .join(".claude")
        .join("projects")
        .join("limits-fixture")
        .join("memory")
        .join("note.md");
    for (id, payload) in [
        ("x1", bash(&env, "x1", "git status")),
        ("x2", bash(&env, "x2", "git diff")),
        ("x3", bash(&env, "x3", "git log -1")),
        ("x4", bash(&env, "x4", "git show HEAD")),
        ("x5", bash(&env, "x5", "node --version")),
        (
            "x6",
            tool(&env, "x6", "Write", json!({"file_path":note,"content":"x"})),
        ),
        (
            "x7",
            tool(
                &env,
                "x7",
                "mcp__everos__memory_add",
                json!({"content":"x"}),
            ),
        ),
    ] {
        let decision = handle(&payload, &mut env);
        assert!(limit_denial(&decision), "{id} ran: {decision:?}");
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
        "turns not counted (no turn limit)",
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
    // A stop the gate would refuse is released at the limit, every time.
    for _ in 0..2 {
        let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
        let Decision::System(report) = &decision else {
            panic!("stop not released with a report: {decision:?}");
        };
        assert!(report.contains("1 of 1 turns used"), "{report}");
        assert!(report.contains("elapsed 42 s"), "{report}");
        assert!(report.contains("turns used 1"), "{report}");
        assert!(report.contains("remaining criteria: C2"), "{report}");
        assert!(
            report.contains("supervisor task next --task ")
                && report.contains("--contract <file> --disposition budget_exhausted"),
            "{report}"
        );
        plain(report);
    }
    // A stop the gate would let through anyway (no completion claim) is not a limit release.
    assert_eq!(
        handle(&stop(&env, "Stopping here."), &mut env),
        Decision::None
    );
}

fn run_check(env: &mut Env, id: &str) {
    let mut pre = bash(env, id, "cargo test");
    assert_eq!(handle(&pre, env), Decision::None, "{id} denied");
    pre["hook_event_name"] = "PostToolUse".into();
    pre["tool_response"] = json!({"stdout":"ok","stderr":"","interrupted":false});
    let after = handle(&pre, env);
    assert!(
        !matches!(after, Decision::Deny(_) | Decision::Block(_)),
        "{after:?}"
    );
}

/// The N-th admitted call may be the passing check: the task still completes, certified.
#[test]
fn a_task_that_uses_every_turn_on_passing_work_still_completes() {
    let d = dir("last-turn");
    let mut env = env(&d, &contract(json!({"turns":2})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    assert_eq!(pre_tool_lines(&env), 2);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        !matches!(&decision, Decision::Block(_))
            && !matches!(&decision, Decision::System(text) if text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

/// The wall clock may pass while the evidence is already complete: the stop is certified.
#[test]
fn a_task_with_full_evidence_completes_after_the_wall_clock_expired() {
    let d = dir("late-stop");
    let mut env = env(&d, &contract(json!({"wall_clock_s":600})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    set_clock(APPROVED_AT + 900);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        !matches!(&decision, Decision::Block(_))
            && !matches!(&decision, Decision::System(text) if text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
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
    // Any output on SubagentStop keeps the worker running, so the release says nothing; it
    // changes no task state, so the chair's own Stop still decides.
    assert_eq!(handle(&handback, &mut env), Decision::None);
    assert_eq!(gate_state(&env), TaskState::Verifying);
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

/// A completed task keeps its state, but the limit still refuses new work: completing a task
/// (for example through MCP) does not lift it.
#[test]
fn a_completed_task_never_exhausts_and_still_refuses_work_past_the_limit() {
    let d = dir("completed");
    let mut env = env(&d, &contract(json!({"turns":1,"wall_clock_s":10})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    set_gate_state(&env, TaskState::Completed);
    set_clock(APPROVED_AT + 100);
    for (id, payload) in [
        ("t2", write(&env, "t2")),
        ("b1", bash(&env, "b1", "echo x > src/out.txt")),
    ] {
        let decision = handle(&payload, &mut env);
        assert!(limit_denial(&decision), "{id}: {decision:?}");
    }
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

/// Item 4 evidence: `task_digest` covers gate.json but not events.jsonl, so a replay of an
/// earlier call is still served after a later judged call. A turn mark stored in gate.json
/// would change it on every call and refuse this replay; the count stays a pure recount.
#[test]
fn a_replay_after_a_later_call_is_still_served() {
    let d = dir("replay-later");
    let mut env = env(&d, &contract(json!({"turns":5})));
    let first = write(&env, "t1");
    assert_eq!(handle(&first, &mut env), Decision::None);
    assert_eq!(handle(&write(&env, "t2"), &mut env), Decision::None);
    assert_eq!(handle(&first, &mut env), Decision::None);
    assert_eq!(pre_tool_lines(&env), 2);
}

/// A journal line that does not parse is one turn: it never loosens a limit and never locks
/// the task. A journal that cannot be read at all fails closed.
#[test]
fn an_unparsable_journal_line_is_a_turn_and_an_unreadable_journal_fails_closed() {
    let d = dir("unparsable");
    let mut env = env(&d, &contract(json!({"turns":2})));
    // A torn last line, as a crash mid-append would leave. Bytes that are not UTF-8 are refused
    // earlier, by the task transaction, which copies the task's files as text.
    fs::write(
        env.state_dir.join("events.jsonl"),
        "not a journal line\n{\"v\":1,\"event\":\"Pre\n",
    )
    .unwrap();
    let decision = handle(&write(&env, "t1"), &mut env);
    assert!(limit_denial(&decision), "{decision:?}");
    assert!(
        matches!(&decision, Decision::Deny(why) if why.contains("2 of 2 turns used")),
        "{decision:?}"
    );
    let d = dir("unreadable");
    let mut env = self::env(&d, &contract(json!({"turns":5})));
    fs::create_dir(env.state_dir.join("events.jsonl")).unwrap();
    let decision = handle(&write(&env, "t1"), &mut env);
    assert!(
        matches!(&decision, Decision::Deny(why) if why.contains("fails closed")),
        "{decision:?}"
    );
    // Without a turn limit the journal is not read for limits. Every judged call also appends
    // to the journal, so a directory there fails any call; `limits::tests` shows the read is
    // skipped for a wall-clock-only contract.
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
    assert!(
        why.contains(&format!(
            "supervisor task next --task {id} --contract <file> --disposition budget_exhausted"
        )),
        "{why}"
    );
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

/// The production layout: prompts registered (`intents.json`), the task under `tasks/<id>`.
fn production(d: &Dir, limits: Value) -> (Env, String) {
    production_rounds(d, limits, 2)
}

fn production_rounds(d: &Dir, limits: Value, rounds: u32) -> (Env, String) {
    set_clock(APPROVED_AT);
    fs::write(
        d.0.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let mut env = Env {
        state_dir: d.0.join("state"),
        state_location: "test",
        candidate: Box::new(Fake),
        clock,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    };
    fs::create_dir_all(&env.state_dir).unwrap();
    let prompt = json!({"hook_event_name":"UserPromptSubmit","cwd":d.0,"session_id":"session",
        "prompt_id":"p1","prompt":"Implement the requested behavior"});
    assert!(matches!(
        handle(&prompt, &mut env),
        Decision::Context { .. }
    ));
    let event = autonomy::pending(&env.state_dir, "session")
        .unwrap()
        .unwrap();
    let contract = json!({"id":"wp4","goal":"limits","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":rounds,"limits":limits,
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]});
    let started = autonomy::propose(
        &env.state_dir,
        &d.0,
        &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"start","task_id":null,"contract":contract}),
    )
    .unwrap();
    (env, started["task_id"].as_str().unwrap().to_owned())
}

fn task_turns(env: &Env, id: &str) -> usize {
    let path = env.state_dir.join("tasks").join(id).join("events.jsonl");
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| serde_json::from_str::<Value>(l).unwrap()["event"] == "PreToolUse")
        .count()
}

/// With prompts registered, read-only tool calls are answered before the task gate and are not
/// turns, while a judged shell read is. At the limit an autonomous task is refused, its stop is
/// released with the report, and every way out names the cancel command.
#[test]
fn an_autonomous_task_in_the_production_layout_stops_at_its_turn_limit() {
    let d = dir("production");
    let (mut env, id) = production(&d, json!({"turns":1}));
    assert_eq!(handle(&read(&env, "r1"), &mut env), Decision::None);
    assert_eq!(task_turns(&env, &id), 0, "a Read was counted");
    assert_eq!(
        handle(&bash(&env, "g1", "git status"), &mut env),
        Decision::None
    );
    assert_eq!(
        task_turns(&env, &id),
        1,
        "a judged shell read was not counted"
    );
    let decision = handle(&write(&env, "w1"), &mut env);
    assert!(limit_denial(&decision), "{decision:?}");
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    let Decision::System(report) = &decision else {
        panic!("stop not released with a report: {decision:?}");
    };
    let cancel = format!("supervisor task cancel --project <dir> --task {id} --reason <text>");
    assert!(report.contains("NOT certified"), "{report}");
    assert!(report.contains(&cancel), "{report}");
    plain(report);
    let why = lifecycle::reset_rounds(&env.state_dir, &id, "flaky check").unwrap_err();
    assert!(why.contains(&cancel), "{why}");
    let why = lifecycle::start_as(
        &env.state_dir,
        &state::Writer::task_start(None),
        &contract(json!({})),
        false,
        None,
        None,
    )
    .unwrap_err();
    assert!(
        why.contains("supervisor task cancel --project <dir> --task"),
        "{why}"
    );
}

/// In strict mode a baseline criterion is met by the reported failure reproducing (exit not 0)
/// on the starting candidate, so the report does not list it as remaining.
#[test]
fn a_reproduced_baseline_is_not_a_remaining_criterion() {
    let d = dir("baseline");
    let mut c = contract(json!({"turns":1}));
    c["criteria"] = json!([
        {"id":"C1","check":"baseline_reproduces_reported_failure","mandatory":true,"command":"python repro.py"},
        {"id":"C2","check":"test","mandatory":true,"command":"cargo test"}
    ]);
    let mut env = env(&d, &c);
    env.level.level = Level::Strict;
    let mut run = bash(&env, "b1", "python repro.py");
    assert_eq!(handle(&run, &mut env), Decision::None);
    run["hook_event_name"] = "PostToolUseFailure".into();
    run["error"] = "Exit code 1\nfailed".into();
    handle(&run, &mut env);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    let Decision::System(report) = &decision else {
        panic!("stop not released with a report: {decision:?}");
    };
    assert!(report.contains("remaining criteria: C2;"), "{report}");
}

/// With prompts registered a memory save is answered before the task gate; at a reached limit
/// it goes on to the gate and is refused there.
#[test]
fn a_memory_save_is_refused_at_the_limit_in_the_production_layout() {
    let d = dir("production-memory");
    let (mut env, _) = production(&d, json!({"turns":1}));
    let save = tool(
        &env,
        "m1",
        "mcp__everos__memory_add",
        json!({"content":"x"}),
    );
    assert_eq!(
        handle(&save, &mut env),
        Decision::None,
        "a save before the limit"
    );
    assert_eq!(
        handle(&bash(&env, "g1", "git status"), &mut env),
        Decision::None
    );
    let save = tool(
        &env,
        "m2",
        "mcp__everos__memory_add",
        json!({"content":"x"}),
    );
    let decision = handle(&save, &mut env);
    assert!(limit_denial(&decision), "{decision:?}");
}

/// A refused call after the last turn changes nothing: a task whose passing check was its last
/// turn still completes, certified, at the next Stop.
#[test]
fn a_refused_call_after_the_last_turn_does_not_stop_certification() {
    let d = dir("refused-then-stop");
    let mut env = env(&d, &contract(json!({"turns":2})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    assert!(limit_denial(&handle(
        &bash(&env, "g1", "git status"),
        &mut env
    )));
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        !matches!(&decision, Decision::Block(_))
            && !matches!(&decision, Decision::System(text) if text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

/// The same with the wall clock: evidence complete, the clock expires, one call is refused,
/// and the Stop still completes.
#[test]
fn a_refused_call_after_the_wall_clock_does_not_stop_certification() {
    let d = dir("late-refused-then-stop");
    let mut env = env(&d, &contract(json!({"wall_clock_s":600})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    set_clock(APPROVED_AT + 900);
    assert!(limit_denial(&handle(&write(&env, "t2"), &mut env)));
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        !matches!(&decision, Decision::Block(_))
            && !matches!(&decision, Decision::System(text) if text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

fn task_gate(env: &Env, id: &str) -> state::Gate {
    let dir = env.state_dir.join("tasks").join(id);
    let (_, digest) = state::read_contract(&dir).unwrap();
    match state::load_gate(&dir, &digest).unwrap() {
        Stored::Valid(gate) => *gate,
        _ => panic!("no valid gate"),
    }
}

/// A stop released for a limit spends no repair round on a refusal nobody saw, also when the
/// repair budget is already spent, and its report names the limit.
#[test]
fn a_stop_released_for_a_limit_spends_no_repair_round() {
    for rounds in [2, 0] {
        let d = dir("release-rounds");
        let (mut env, id) = production_rounds(&d, json!({"turns":1}), rounds);
        let mut run = bash(&env, "c1", "cargo test");
        assert_eq!(handle(&run, &mut env), Decision::None);
        run["hook_event_name"] = "PostToolUseFailure".into();
        run["error"] = "Exit code 101\nfailed".into();
        handle(&run, &mut env);
        let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
        let Decision::System(report) = &decision else {
            panic!("stop not released with a report: {decision:?}");
        };
        assert!(
            report.contains("task limit reached (1 of 1 turns used)"),
            "{rounds}: {report}"
        );
        let gate = task_gate(&env, &id);
        assert_eq!(
            gate.checkpoint.consumed_repair_rounds, 0,
            "{rounds}: {report}"
        );
        assert_eq!(gate.repair_charged, None, "{rounds}: {report}");
    }
}

/// A stop the gate would let through without certifying because of the refusal cap is released
/// at a reached limit with the report too. (The pending-job case takes the same release path;
/// no test drives a pending Terminal Commander job here.)
#[test]
fn a_stop_past_the_refusal_cap_at_a_limit_gets_the_report() {
    let d = dir("cap");
    let mut env = env(&d, &contract(json!({"wall_clock_s":600})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    for _ in 0..3 {
        let refused = handle(&stop(&env, "Done. All checks pass."), &mut env);
        assert!(matches!(refused, Decision::Block(_)), "{refused:?}");
    }
    set_clock(APPROVED_AT + 700);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        matches!(&decision, Decision::System(text) if text.contains("task limit reached (elapsed 700 s of 600 s)") && text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::BudgetExhausted);
}

/// A completed task's handback released at a limit is not called uncertified.
#[test]
fn a_completed_task_handback_at_the_limit_is_not_called_uncertified() {
    let d = dir("completed-handback");
    let mut env = env(&d, &contract(json!({"turns":1})));
    let mut first = write(&env, "w1");
    first["agent_id"] = "worker".into();
    assert_eq!(handle(&first, &mut env), Decision::None);
    set_gate_state(&env, TaskState::Completed);
    let handback = json!({"hook_event_name":"SubagentStop","cwd":cwd(&env),"session_id":"session",
        "agent_id":"worker","stop_hook_active":false,"last_assistant_message":"done"});
    assert_eq!(handle(&handback, &mut env), Decision::None);
    let journal = String::from_utf8(journal(&env)).unwrap();
    let last: Value = serde_json::from_str(journal.lines().last().unwrap()).unwrap();
    assert!(
        last["reasons"].to_string().contains("task limit reached"),
        "{last}"
    );
    assert!(
        !last["reasons"].to_string().contains("NOT certified"),
        "{last}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

/// With prompts registered, a memory save whose task state cannot be read is not answered early:
/// it goes on to the task gate and fails closed there (before WP4 it was answered early).
#[test]
fn a_memory_save_with_unreadable_task_state_fails_closed() {
    let d = dir("memory-unreadable");
    let (mut env, id) = production(&d, json!({"turns":5}));
    fs::write(
        env.state_dir.join("tasks").join(&id).join("contract.json"),
        "{ not json",
    )
    .unwrap();
    let save = tool(
        &env,
        "m1",
        "mcp__everos__memory_add",
        json!({"content":"x"}),
    );
    let decision = handle(&save, &mut env);
    assert!(
        matches!(&decision, Decision::Deny(why) if why.contains("fails closed")),
        "{decision:?}"
    );
}

/// At a limit the exempt proposal path cannot cancel, pause or replace the task: control actions
/// need the person's own control prompt, and a new task needs the active one completed.
#[test]
fn the_proposal_path_cannot_lift_a_limit() {
    let d = dir("proposal-at-limit");
    let (mut env, id) = production(&d, json!({"turns":1}));
    assert_eq!(
        handle(&bash(&env, "g1", "git status"), &mut env),
        Decision::None
    );
    assert!(limit_denial(&handle(&write(&env, "w1"), &mut env)));
    let prompt = json!({"hook_event_name":"UserPromptSubmit","cwd":d.0,"session_id":"session",
        "prompt_id":"p2","prompt":"Keep going with the change"});
    assert!(matches!(
        handle(&prompt, &mut env),
        Decision::Context { .. }
    ));
    let store: Value =
        serde_json::from_str(&fs::read_to_string(env.state_dir.join("intents.json")).unwrap())
            .unwrap();
    let latest = store["latest"]["session"].as_str().unwrap().to_owned();
    let event = &store["events"][&latest];
    for action in ["cancel", "pause"] {
        let why = autonomy::propose(
            &env.state_dir,
            &d.0,
            &json!({"event_id":latest,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
                "action":action,"task_id":id}),
        )
        .unwrap_err();
        assert!(why.contains("control prompt"), "{action}: {why}");
    }
    let why = autonomy::propose(
        &env.state_dir,
        &d.0,
        &json!({"event_id":latest,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"start","task_id":null,"contract":contract(json!({}))}),
    )
    .unwrap_err();
    assert!(why.contains("unfinished task retained"), "{why}");
}

/// A worker handback refused and released at the limit does not end the task: the chair's
/// evidence is complete, and the chair's Stop still completes it, certified.
#[test]
fn a_worker_handback_released_at_the_limit_leaves_the_chair_to_complete() {
    let d = dir("worker-then-chair");
    let mut env = env(&d, &contract(json!({"turns":3})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    let mut worker = write(&env, "w1");
    worker["agent_id"] = "worker".into();
    assert_eq!(handle(&worker, &mut env), Decision::None);
    run_check(&mut env, "c1");
    let handback = json!({"hook_event_name":"SubagentStop","cwd":cwd(&env),"session_id":"session",
        "agent_id":"worker","stop_hook_active":false,"last_assistant_message":"done"});
    assert_eq!(handle(&handback, &mut env), Decision::None);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(
        !matches!(&decision, Decision::Block(_))
            && !matches!(&decision, Decision::System(text) if text.contains("NOT certified")),
        "{decision:?}"
    );
    assert_eq!(gate_state(&env), TaskState::Completed);
}

/// The report's remaining criteria follow the evidence rule: a check that passed and then failed
/// on the same candidate is still remaining.
#[test]
fn a_check_that_passed_then_failed_is_a_remaining_criterion() {
    let d = dir("pass-then-fail");
    let mut env = env(&d, &contract(json!({"turns":3})));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    let mut run = bash(&env, "c2", "cargo test");
    assert_eq!(handle(&run, &mut env), Decision::None);
    run["hook_event_name"] = "PostToolUseFailure".into();
    run["error"] = "Exit code 101\nfailed".into();
    handle(&run, &mut env);
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    let Decision::System(report) = &decision else {
        panic!("stop not released with a report: {decision:?}");
    };
    assert!(report.contains("remaining criteria: C2;"), "{report}");
}

/// A person's task at `root`, with `limits`, in a project the floor discovery knows.
fn person(d: &Dir, limits: Value) -> (Env, String) {
    set_clock(APPROVED_AT);
    fs::write(
        d.0.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let env = Env {
        state_dir: d.0.join("state"),
        state_location: "test",
        candidate: Box::new(Fake),
        clock,
        level: Setting {
            level: Level::Enforce,
            ..Default::default()
        },
    };
    let mut request = contract(limits);
    request.as_object_mut().unwrap().remove("approval");
    let started = lifecycle::start_as(
        &env.state_dir,
        &state::Writer::task_start(None),
        &request,
        false,
        None,
        None,
    )
    .unwrap();
    (env, started["task_id"].as_str().unwrap().to_owned())
}

fn propose_start(env: &mut Env, d: &Dir) -> Result<Value, String> {
    let prompt = json!({"hook_event_name":"UserPromptSubmit","cwd":d.0,"session_id":"session",
        "prompt_id":"next","prompt":"Implement the next change"});
    assert!(matches!(handle(&prompt, env), Decision::Context { .. }));
    let event = autonomy::pending(&env.state_dir, "session")
        .unwrap()
        .unwrap();
    let contract = json!({"id":"wp4-next","goal":"next","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]});
    autonomy::propose(
        &env.state_dir,
        &d.0,
        &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"start","task_id":event["task_id"],"contract":contract}),
    )
}

/// A person's limit is not shed by completing the task on caller-asserted receipts (as MCP
/// does) and then starting a new task through an agent proposal.
#[test]
fn an_agent_cannot_replace_a_persons_task_completed_without_observed_evidence() {
    let d = dir("person-asserted");
    let (mut env, id) = person(&d, json!({"turns":10}));
    let candidate = coding_supervisor::CandidateId {
        base_commit: "base".into(),
        dirty_digest: "one".into(),
    };
    lifecycle::with_task(&env.state_dir, Some(&id), None, None, |stage| {
        let (_, digest) = state::read_contract(stage)?;
        let mut gate = state::Gate::new(
            "wp4".to_owned(),
            coding_supervisor::Checkpoint {
                state: TaskState::Completed,
                candidate: Some(candidate.clone()),
                open_findings: vec![],
                consumed_repair_rounds: 0,
                next_action: String::new(),
            },
        );
        gate.receipts.push(coding_supervisor::VerificationReceipt {
            criterion: coding_supervisor::CriterionId("C2".into()),
            candidate: candidate.clone(),
            command: "cargo test".into(),
            cwd: ".".into(),
            exit_code: Some(0),
            output_ref: "mcp-asserted:out".into(),
            claim_supported: true,
            started_at: 1,
            finished_at: 2,
        });
        state::save_gate(stage, &gate, &digest)?;
        Ok(Value::Null)
    })
    .unwrap();
    let why = propose_start(&mut env, &d).unwrap_err();
    assert!(why.contains("started by a person"), "{why}");
    assert!(
        why.contains(&format!("supervisor task next --task {id}")),
        "{why}"
    );
}

/// Positive control: the same task completed on hook-observed evidence may be followed by an
/// agent's start.
#[test]
fn an_agent_may_start_after_a_persons_task_completed_on_observed_evidence() {
    let d = dir("person-observed");
    let (mut env, _) = person(&d, json!({"turns":10}));
    assert_eq!(handle(&write(&env, "t1"), &mut env), Decision::None);
    run_check(&mut env, "c1");
    let decision = handle(&stop(&env, "Done. All checks pass."), &mut env);
    assert!(!matches!(decision, Decision::Block(_)), "{decision:?}");
    let started = propose_start(&mut env, &d).unwrap();
    assert!(started["task_id"].as_str().is_some(), "{started}");
}
