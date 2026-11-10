//! WP1: an adapter error fails closed for a governed project (`enforce`, `strict`): a
//! non-read-only PreToolUse is denied and a Stop is refused. Read-only calls, non-blocking
//! events, `audit` and a person's `failure open` keep the advisory message.
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle, run_hook};
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

/// No word Claude Code reads as "nothing to block" (review: such stop texts do not block).
fn clean(text: &str) {
    for word in ["no such file", "not found", "missing"] {
        assert!(!text.to_lowercase().contains(word), "{text}");
    }
}

fn answer(raw: &str) -> Value {
    serde_json::from_str(&run_hook(raw).expect("an answer")).unwrap()
}

#[test]
fn a_truncated_pre_tool_use_payload_is_denied() {
    let raw = r#"{"hook_event_name":"PreToolUse","cwd":"C:/nowhere","tool_name":"Write","tool_input":{"file_path":"src/lib.rs","content":"x"#;
    let v = answer(raw);
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny", "{v}");
    let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(reason.contains("fails closed"), "{reason}");
    clean(reason);
}

#[test]
fn a_truncated_stop_payload_is_refused_unless_a_stop_hook_is_already_active() {
    let v =
        answer(r#"{"hook_event_name" : "Stop", "cwd":"C:/nowhere","last_assistant_message":"do"#);
    assert_eq!(v["decision"], "block", "{v}");
    clean(v["reason"].as_str().unwrap());
    let v = answer(
        r#"{"hook_event_name":"SubagentStop","stop_hook_active":true,"cwd":"C:/nowhere","last_assistant_message":"do"#,
    );
    assert_ne!(v["decision"], "block", "{v}");
    let message = v["systemMessage"].as_str().unwrap_or_default();
    assert!(message.contains("went ahead NOT certified"), "{v}");
}

#[test]
fn a_contract_error_naming_a_missing_field_is_denied_in_blocking_words() {
    let (_d, mut env) = project(Level::Enforce);
    fs::write(
        env.state_dir.join("contract.json"),
        json!({"id":"wp1","goal":"fail closed","scope":["src/"],"forbidden_effects":[],
            "max_repair_rounds":2,"criteria":[{"id":"C1","check":"test","command":"cargo test"}]})
        .to_string(),
    )
    .unwrap();
    let decision = handle(&write(&env), &mut env);
    let Decision::Deny(why) = &decision else {
        panic!("a governed contract error must deny, got {decision:?}");
    };
    assert!(why.contains("fails closed"), "{why}");
    assert!(why.contains("field mandatory"), "{why}");
    clean(why);
}

#[test]
fn a_governed_stop_with_an_adapter_error_fails_closed() {
    let (_d, mut env) = project(Level::Enforce);
    break_runs(&env);
    let decision = handle(&stop(&env), &mut env);
    assert!(matches!(decision, Decision::Block(_)), "{decision:?}");
    fails_closed(&decision);
}

#[test]
fn a_stop_already_continued_by_a_stop_hook_goes_ahead_uncertified() {
    let (_d, mut env) = project(Level::Enforce);
    break_runs(&env);
    let mut payload = stop(&env);
    payload["stop_hook_active"] = json!(true);
    let decision = handle(&payload, &mut env);
    let Decision::System(message) = &decision else {
        panic!("an active stop hook must not loop on a broken gate, got {decision:?}");
    };
    assert!(
        message.starts_with("GATE: WARNING, this Stop went ahead NOT certified: "),
        "{message}"
    );
}

#[test]
fn intent_apply_is_not_exempt_from_failing_closed() {
    let (_d, mut env) = project(Level::Enforce);
    break_runs(&env);
    let call = tool(
        &env,
        "PreToolUse",
        "mcp__supervisor__intent_apply",
        json!({"event_id":"e","nonce":"n","prompt_hash":"h","action":"discussion"}),
    );
    let decision = handle(&call, &mut env);
    assert!(matches!(decision, Decision::Deny(_)), "{decision:?}");
    fails_closed(&decision);
}

#[test]
fn an_unwritable_events_journal_commits_nothing_for_the_denied_call() {
    let (_d, mut env) = project(Level::Enforce);
    break_events(&env);
    let run = tool(&env, "PreToolUse", "Bash", json!({"command":"cargo test"}));
    let decision = handle(&run, &mut env);
    assert!(matches!(decision, Decision::Deny(_)), "{decision:?}");
    let runs = fs::read_to_string(env.state_dir.join("runs.jsonl")).unwrap_or_default();
    assert!(
        runs.trim().is_empty(),
        "a start record was committed: {runs}"
    );
    let cached = fs::read_to_string(env.state_dir.join("applied-events.json")).unwrap_or_default();
    assert!(
        !cached.contains("PreToolUse-Bash"),
        "a decision was cached: {cached}"
    );
    assert!(
        !env.state_dir.join("gate.json").exists(),
        "gate.json was committed"
    );
}

#[test]
fn failure_open_keeps_a_computed_stop_refusal_when_the_journal_fails() {
    let (_d, mut env) = project(Level::Enforce);
    fs::write(env.state_dir.join("failure"), "open\n").unwrap();
    break_events(&env);
    // No receipt for C1: the gate refuses this stop for evidence reasons.
    let decision = handle(&stop(&env), &mut env);
    let Decision::Block(why) = &decision else {
        panic!("the evidence refusal must survive a lost journal line, got {decision:?}");
    };
    assert!(!why.contains("fails closed"), "{why}");
    assert!(!why.contains("adapter error"), "{why}");
}

#[test]
fn an_evidence_refusal_still_blocks_a_stop_already_continued_by_a_stop_hook() {
    let (_d, mut env) = project(Level::Enforce);
    // No adapter error: C1 simply has no receipt, so the NOT certified release does not apply.
    let mut payload = stop(&env);
    payload["stop_hook_active"] = json!(true);
    let decision = handle(&payload, &mut env);
    let Decision::Block(why) = &decision else {
        panic!("an ordinary evidence refusal must still block, got {decision:?}");
    };
    assert!(!why.contains("NOT certified"), "{why}");
    assert!(!why.contains("fails closed"), "{why}");
}
