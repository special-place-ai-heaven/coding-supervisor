//! Caller-asserted intent is separate from the trusted Claude hook lifecycle.

mod common;

use common::{call, dir, ok, refused, server};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use supervisor_claude_code::lifecycle;

fn project_with_floor(path: &Path) {
    fs::write(
        path.join("Cargo.toml"),
        "[package]\nname='intent-fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::create_dir(path.join(".git")).unwrap();
}

fn event(s: &supervisor_mcp::Server, session: &str, prompt_id: &str, prompt: &str) -> Value {
    ok(
        s,
        "intent_record",
        json!({"session_id": session, "prompt_id": prompt_id, "prompt": prompt}),
    )
}

fn apply(s: &supervisor_mcp::Server, event: &Value, action: &str, extra: Value) -> (bool, String) {
    let mut args = json!({"event_id": event["event_id"], "nonce": event["nonce"], "prompt_hash": event["prompt_hash"], "action": action});
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    call(s, "intent_apply", args)
}

#[test]
fn caller_intent_is_labeled_and_discussion_leaves_project_inactive() {
    let d = dir("intent-discussion");
    project_with_floor(&d.0.join("project"));
    let s = server(&d);
    let intent = ok(
        &s,
        "intent_record",
        json!({"session_id": "codex-session", "prompt_id": "p-1", "prompt": "Explain the test layout"}),
    );
    assert_eq!(intent["source"], "caller-asserted");
    assert_eq!(intent["session_id"], "codex-session");
    assert!(intent["event_id"].is_string());
    assert!(intent["nonce"].is_string());
    assert!(intent["prompt_hash"].is_string());
    assert!(!intent["required_criteria"].as_array().unwrap().is_empty());

    let result = ok(
        &s,
        "intent_apply",
        json!({
            "event_id": intent["event_id"],
            "nonce": intent["nonce"],
            "prompt_hash": intent["prompt_hash"],
            "action": "discussion"
        }),
    );
    assert_eq!(result["action"], "discussion");
    assert_eq!(result["mode"], "advisory");
    assert!(lifecycle::active_task(s.state_dir()).unwrap().is_none());
}

#[test]
fn intent_replay_is_idempotent_and_changed_replay_is_refused() {
    let d = dir("intent-replay");
    let s = server(&d);
    let args = json!({"session_id": "codex-session", "prompt_id": "p-replay", "prompt": "Please explain this module"});
    let first = ok(&s, "intent_record", args.clone());
    let again = ok(&s, "intent_record", args);
    assert_eq!(again["event_id"], first["event_id"]);
    assert_eq!(again["nonce"], first["nonce"]);
    assert_eq!(again["prompt_hash"], first["prompt_hash"]);

    let changed = json!({"session_id": "codex-session", "prompt_id": "p-replay", "prompt": "Start implementation"});
    let text = refused(&s, "intent_record", changed);
    assert!(text.contains("refused"), "{text}");
}

#[test]
fn malformed_or_incomplete_intent_is_refused_without_creating_task() {
    let d = dir("intent-invalid");
    let s = server(&d);
    let (is_error, text) = call(&s, "intent_record", json!({"session_id": "s"}));
    assert!(is_error);
    assert!(!text.is_empty());
    assert!(lifecycle::active_task(s.state_dir()).unwrap().is_none());
    assert!(s.state_dir().join("events.jsonl").metadata().is_err());
}

#[test]
fn caller_intent_cannot_control_a_hook_owned_task() {
    let d = dir("intent-ownership");
    let project = d.0.join("project");
    project_with_floor(&project);
    let s = server(&d);
    let hook_event = supervisor_claude_code::autonomy::record_prompt(
        s.state_dir(),
        &project,
        &json!({"session_id": "hook-session", "prompt_id": "hook-start", "prompt": "Implement this feature"}),
    ).unwrap();
    let contract = json!({"id":"hook-task", "goal":"Implement this feature", "criteria":[{"id":"C1","check":"work","mandatory":true,"command":"cargo test"}], "forbidden_effects":[], "scope":["src/"], "max_repair_rounds":1});
    let started = supervisor_claude_code::autonomy::propose(
        s.state_dir(),
        &project,
        &json!({"event_id":hook_event["event_id"],"nonce":hook_event["nonce"],"prompt_hash":hook_event["prompt_hash"],"action":"start","contract":contract}),
    ).unwrap();
    let text = refused(
        &s,
        "intent_record",
        json!({"session_id":"hook-session","prompt_id":"caller-next","prompt":"Continue the task"}),
    );
    assert!(text.contains("trusted hook intent"), "{text}");
    let caller_event = event(&s, "caller-session", "caller-next", "Continue the task");
    let (is_error, text) = apply(
        &s,
        &caller_event,
        "continue",
        json!({"task_id": started["task_id"]}),
    );
    assert!(is_error);
    assert!(
        text.contains("cannot control a trusted hook task"),
        "{text}"
    );
}

#[test]
fn contract_revision_cannot_remove_existing_criteria() {
    let d = dir("intent-monotonic");
    let project = d.0.join("project");
    project_with_floor(&project);
    let s = server(&d);
    let first = event(&s, "codex-session", "start-1", "Implement the feature");
    let contract = json!({"id":"monotonic-task", "goal":"Implement the feature", "criteria":[{"id":"C1","check":"work","mandatory":true,"command":"cargo test"}], "forbidden_effects":[], "scope":["src/"], "max_repair_rounds":1});
    let (failed, started_text) = apply(&s, &first, "start", json!({"contract": contract}));
    assert!(!failed, "{started_text}");
    let started: Value = serde_json::from_str(&started_text).unwrap();
    let revision = event(&s, "codex-session", "revise-1", "Continue implementation");
    let weaker = json!({"id":"monotonic-task", "goal":"Implement the feature", "criteria":[{"id":"C1","check":"work","mandatory":true,"command":"cargo test"}], "forbidden_effects":[], "scope":["src/"], "max_repair_rounds":1});
    let (is_error, text) = apply(
        &s,
        &revision,
        "revise",
        json!({"task_id":started["task_id"],"contract":weaker}),
    );
    assert!(is_error);
    assert!(text.contains("removed a criterion"), "{text}");
}

#[test]
fn asserted_receipts_cannot_complete_a_hook_owned_autonomous_task() {
    let d = dir("hook-acceptance-authority");
    let project = d.0.join("project");
    project_with_floor(&project);
    let s = server(&d);
    let hook = supervisor_claude_code::autonomy::record_prompt(
        s.state_dir(),
        &project,
        &json!({"session_id":"hook","prompt_id":"start","prompt":"Implement the feature"}),
    )
    .unwrap();
    supervisor_claude_code::autonomy::propose(s.state_dir(), &project,
        &json!({"event_id":hook["event_id"],"nonce":hook["nonce"],"prompt_hash":hook["prompt_hash"],
        "action":"start","contract":{"id":"protected","goal":"Implement the feature",
        "scope":["src/"],"criteria":[{"id":"C1","check":"feature","command":"cargo test","mandatory":true}],
        "forbidden_effects":[],"max_repair_rounds":2}})).unwrap();
    common::observe(&s, "candidate", &["src/lib.rs"]);
    let state = common::task_state(&s);
    for criterion in state["criteria"].as_array().unwrap() {
        ok(
            &s,
            "receipt_record",
            common::receipt(
                criterion["id"].as_str().unwrap(),
                criterion["command"][0].as_str().unwrap(),
                json!(0),
                true,
            ),
        );
    }
    let refusal = refused(&s, "accept_try", json!({}));
    assert!(refusal.contains("observed hook evidence"), "{refusal}");
    assert_ne!(common::task_state(&s)["checkpoint"]["state"], "completed");
}

#[test]
fn advisory_acceptance_cannot_silently_replace_a_captured_npm_entrypoint() {
    let d = dir("npm-entrypoint-authority");
    let project = d.0.join("project");
    fs::write(
        project.join("package.json"),
        json!({"scripts":{"test":"node --test"}}).to_string(),
    )
    .unwrap();
    let s = server(&d);
    let prompt = event(&s, "codex", "start", "Implement the feature");
    let (failed, message) = apply(
        &s,
        &prompt,
        "start",
        json!({"contract":{
        "id":"npm","goal":"Implement the feature","scope":["package.json","src/"],
        "criteria":[{"id":"C1","check":"tests","command":"npm run test","mandatory":true}],
        "forbidden_effects":[],"max_repair_rounds":2}}),
    );
    assert!(!failed, "{message}");
    fs::write(
        project.join("package.json"),
        json!({"scripts":{"test":"node -e \"process.exit(0)\""}}).to_string(),
    )
    .unwrap();
    common::observe(&s, "changed-runner", &["package.json"]);
    let state = common::task_state(&s);
    for criterion in state["criteria"].as_array().unwrap() {
        ok(
            &s,
            "receipt_record",
            common::receipt(
                criterion["id"].as_str().unwrap(),
                criterion["command"][0].as_str().unwrap(),
                json!(0),
                true,
            ),
        );
    }
    let message = refused(&s, "accept_try", json!({}));
    assert!(message.contains("entrypoint"), "{message}");
}
