//! WP1: an adapter error fails closed for a governed project (`enforce`, `strict`): a
//! non-read-only PreToolUse is denied and a Stop is refused. Read-only calls, non-blocking
//! events, `audit` and a person's `failure open` keep the advisory message.
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::CandidateProvider;
use supervisor_claude_code::level::{Level, Setting};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

thread_local! {
    /// A path inside the transaction's task stage that the clock turns into a directory.
    static BREAK: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The fixture clock. The adapter reads it inside its state transaction; while `BREAK` names a
/// path in the task stage, that path becomes a directory there, so the adapter's own read or
/// write of it later in the same call fails, as a full disk or a held file would. The staged
/// directory is never committed, so the state stays loadable for the next call.
fn clock() -> u64 {
    BREAK.with_borrow(|path| {
        if let Some(path) = path
            && path.parent().is_some_and(Path::is_dir)
        {
            let _ = fs::create_dir_all(path);
        }
    });
    1_000
}

/// A project whose state root holds one contract (one mandatory criterion), at `level`.
fn project(level: Level) -> (Fixture, Env) {
    BREAK.set(None);
    let d = Fixture(std::env::temp_dir().join(format!(
        "supervisor-fail-closed-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let state = d.0.join(".supervisor");
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("contract.json"),
        json!({"id":"wp1","goal":"fail closed","scope":["src/"],"forbidden_effects":[],
            "max_repair_rounds":2,
            "criteria":[{"id":"C1","check":"test","mandatory":true,"command":"cargo test"}]})
        .to_string(),
    )
    .unwrap();
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Fake),
        clock,
        level: Setting {
            level,
            ..Default::default()
        },
    };
    (d, env)
}

/// The command journal holds a line that is not JSON: the judgement itself fails inside the
/// adapter, while events.jsonl stays writable.
fn break_runs(env: &Env) {
    fs::write(env.state_dir.join("runs.jsonl"), "{not json\n").unwrap();
}

/// events.jsonl cannot be appended to.
fn break_events(env: &Env) {
    BREAK.set(Some(env.state_dir.join("task-stage").join("events.jsonl")));
}

fn tool(env: &Env, event: &str, name: &str, input: Value) -> Value {
    json!({"hook_event_name":event,"cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "agent_id":"main","tool_use_id":format!("{event}-{name}"),"tool_name":name,
        "tool_input":input})
}

fn write(env: &Env) -> Value {
    tool(
        env,
        "PreToolUse",
        "Write",
        json!({"file_path":"src/lib.rs","content":"x"}),
    )
}

fn stop(env: &Env) -> Value {
    json!({"hook_event_name":"Stop","cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "stop_hook_active":false,"last_assistant_message":"done"})
}

fn fails_closed(decision: &Decision) {
    let (Decision::Deny(why) | Decision::Block(why)) = decision else {
        panic!("a governed adapter error must refuse, got {decision:?}");
    };
    assert!(why.contains("fails closed"), "{why}");
    for word in ["no such file", "not found", "missing"] {
        assert!(!why.to_lowercase().contains(word), "{why}");
    }
}

fn advisory(decision: &Decision) {
    let Decision::System(message) = decision else {
        panic!("the advisory adapter-error message is expected, got {decision:?}");
    };
    assert!(message.starts_with("GATE adapter error: "), "{message}");
}

fn journal(env: &Env) -> Vec<Value> {
    fs::read_to_string(env.state_dir.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn governed_mutation_with_an_adapter_error_is_denied() {
    for level in [Level::Enforce, Level::Strict] {
        let (_d, mut env) = project(level);
        break_runs(&env);
        let decision = handle(&write(&env), &mut env);
        assert!(
            matches!(decision, Decision::Deny(_)),
            "{level:?}: {decision:?}"
        );
        fails_closed(&decision);
    }
}

#[test]
fn an_unwritable_events_journal_denies_a_mutation() {
    let (_d, mut env) = project(Level::Enforce);
    break_events(&env);
    let decision = handle(&write(&env), &mut env);
    assert!(matches!(decision, Decision::Deny(_)), "{decision:?}");
    fails_closed(&decision);
}

#[test]
fn an_unwritable_events_journal_keeps_a_stop_refusal() {
    let (_d, mut env) = project(Level::Enforce);
    break_events(&env);
    // No receipt for C1, so the stop is refused; a lost journal line must not release it.
    let decision = handle(&stop(&env), &mut env);
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
}

#[test]
fn a_corrupt_cached_response_denies_a_governed_mutation() {
    let (_d, mut env) = project(Level::Enforce);
    assert_eq!(handle(&write(&env), &mut env), Decision::None);
    let path = env.state_dir.join("applied-events.json");
    let mut events: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    for event in events.as_object_mut().unwrap().values_mut() {
        event["response"] = json!({"unexpected":true});
    }
    fs::write(&path, events.to_string()).unwrap();
    // The same call again is answered from the cache, which no longer holds a decision.
    let decision = handle(&write(&env), &mut env);
    assert!(matches!(decision, Decision::Deny(_)), "{decision:?}");
    fails_closed(&decision);
}

#[test]
fn a_read_only_call_keeps_the_advisory_message() {
    let (_d, mut env) = project(Level::Enforce);
    break_runs(&env);
    let read = tool(
        &env,
        "PreToolUse",
        "Read",
        json!({"file_path":"src/lib.rs"}),
    );
    advisory(&handle(&read, &mut env));
    let status = tool(&env, "PreToolUse", "Bash", json!({"command":"git status"}));
    advisory(&handle(&status, &mut env));
}

#[test]
fn failure_open_keeps_the_advisory_message_and_journals_it() {
    let (_d, mut env) = project(Level::Enforce);
    fs::write(env.state_dir.join("failure"), "open\n").unwrap();
    break_runs(&env);
    advisory(&handle(&write(&env), &mut env));
    let lines = journal(&env);
    let reasons = lines.last().expect("one journal line")["reasons"].to_string();
    assert!(reasons.contains("failure open (advisory)"), "{reasons}");
}

#[test]
fn audit_keeps_the_advisory_message() {
    let (_d, mut env) = project(Level::Audit);
    break_runs(&env);
    advisory(&handle(&write(&env), &mut env));
}

#[test]
fn a_non_blocking_event_keeps_the_advisory_message() {
    let (_d, mut env) = project(Level::Enforce);
    break_runs(&env);
    let mut post = tool(
        &env,
        "PostToolUse",
        "Write",
        json!({"file_path":"src/lib.rs","content":"x"}),
    );
    post["tool_response"] = json!({"success":true});
    advisory(&handle(&post, &mut env));
}
