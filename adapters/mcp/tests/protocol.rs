//! JSON-RPC and MCP framing: handshake, tool list, errors, notifications, input validation.

mod common;

use common::{call, dir, rpc, server};
use serde_json::{Value, json};
use supervisor_mcp::serve;

#[test]
fn initialize_echoes_a_known_version_and_names_the_server() {
    let d = dir("init");
    let s = server(&d);
    let reply = rpc(
        &s,
        "initialize",
        json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 7);
    let result = &reply["result"];
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert_eq!(result["capabilities"], json!({"tools": {}}));
    assert_eq!(result["serverInfo"]["name"], "supervisor-mcp");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let unknown = rpc(&s, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(unknown["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(rpc(&s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn tools_list_has_the_ten_tools_with_required_fields() {
    let d = dir("list");
    let s = server(&d);
    let tools = rpc(&s, "tools/list", json!({}))["result"]["tools"].clone();
    let tools = tools.as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "intent_record",
            "intent_apply",
            "task_start",
            "candidate_observe",
            "receipt_record",
            "finding_raise",
            "repair_request",
            "accept_try",
            "task_complete",
            "task_state"
        ]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(!tool["description"].as_str().unwrap().is_empty());
    }
    let required = |name: &str| -> Vec<String> {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        serde_json::from_value(tool["inputSchema"]["required"].clone()).unwrap()
    };
    assert_eq!(required("intent_record"), ["session_id", "prompt"]);
    assert_eq!(
        required("intent_apply"),
        ["event_id", "nonce", "prompt_hash", "action"]
    );
    assert_eq!(
        required("receipt_record"),
        [
            "criterion",
            "command",
            "cwd",
            "exit_code",
            "output_ref",
            "claim_supported",
            "task_id"
        ]
    );
    assert_eq!(required("candidate_observe"), ["task_id"]);
    assert_eq!(
        required("finding_raise"),
        ["id", "severity", "location", "evidence_ref", "task_id"]
    );
    assert_eq!(
        required("task_start"),
        [
            "id",
            "goal",
            "criteria",
            "forbidden_effects",
            "scope",
            "max_repair_rounds"
        ]
    );
}

#[test]
fn unknown_method_and_unknown_tool_are_rpc_errors() {
    let d = dir("unknown");
    let s = server(&d);
    let reply = rpc(&s, "resources/list", json!({}));
    assert_eq!(reply["error"]["code"], -32601);
    assert_eq!(reply["id"], 7);
    let reply = rpc(&s, "tools/call", json!({"name": "nope", "arguments": {}}));
    assert_eq!(reply["error"]["code"], -32602);
    assert!(!s.state_dir().join("events.jsonl").exists());
    assert!(
        supervisor_claude_code::lifecycle::active_task(s.state_dir())
            .unwrap()
            .is_none()
    );
    let parse: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);
}

#[test]
fn notifications_and_client_responses_get_no_reply() {
    let d = dir("notify");
    let s = server(&d);
    assert_eq!(
        s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
        None
    );
    assert_eq!(
        s.handle(&json!({"jsonrpc": "2.0", "method": "no/such/method"})),
        None
    );
    assert_eq!(
        s.handle(
            &json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "task_state"}})
        ),
        None
    );
    assert_eq!(
        s.handle(&json!({"jsonrpc": "2.0", "id": 3, "result": {}})),
        None
    );
    assert!(!s.state_dir().join("events.jsonl").exists());
}

#[test]
fn missing_or_ill_typed_fields_are_tool_errors_naming_the_field() {
    let d = dir("fields");
    let s = server(&d);
    let cases = [
        (
            "receipt_record",
            json!({"criterion": "C1", "command": "x", "cwd": ".", "exit_code": "zero", "output_ref": "o", "claim_supported": true}),
            "field exit_code",
        ),
        (
            "receipt_record",
            json!({"criterion": "C1", "command": "x", "cwd": ".", "exit_code": 0, "output_ref": "o"}),
            "missing field claim_supported",
        ),
        (
            "candidate_observe",
            json!({"base_commit": "b", "changed_paths": []}),
            "missing field dirty_digest",
        ),
        (
            "candidate_observe",
            json!({"base_commit": "b", "dirty_digest": "d", "changed_paths": [1]}),
            "field changed_paths",
        ),
        (
            "finding_raise",
            json!({"id": "F", "severity": "urgent", "location": "l", "evidence_ref": "e"}),
            "field severity",
        ),
        ("accept_try", json!([1]), "field arguments"),
    ];
    for (tool, args, named) in cases {
        let (is_error, text) = call(&s, tool, args);
        assert!(is_error, "{tool}: {text}");
        assert!(text.contains(named), "{tool}: {text}");
    }
    let mut contract = common::four();
    contract["criteria"][2]["command"] = json!([]);
    let (is_error, text) = call(&s, "task_start", contract);
    assert!(is_error);
    assert!(text.contains("criterion C3 command is empty"), "{text}");
    let mut contract = common::four();
    contract["max_repair_rounds"] = json!("two");
    let (is_error, text) = call(&s, "task_start", contract);
    assert!(is_error);
    assert!(text.contains("max_repair_rounds"), "{text}");
}

#[test]
fn serve_answers_line_by_line_over_a_byte_stream() {
    let d = dir("serve");
    let s = server(&d);
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\r\n\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n"
    );
    let mut out = Vec::new();
    serve(&s, input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 10);
}

/// WP2: `task_start` writes contract v2 and records itself as the writer.
#[test]
fn task_start_writes_contract_v2() {
    let d = dir("contract-v2");
    let s = server(&d);
    let (is_error, text) = call(&s, "task_start", common::four());
    assert!(!is_error, "{text}");
    let task = supervisor_claude_code::lifecycle::active_task(s.state_dir())
        .unwrap()
        .unwrap();
    let c: Value =
        serde_json::from_str(&std::fs::read_to_string(task.dir.join("contract.json")).unwrap())
            .unwrap();
    assert_eq!(c["schema_version"], 2, "{c}");
    assert_eq!(c["isolation"], "none", "{c}");
    assert_eq!(c["approval"]["via"], "mcp-task-start", "{c}");
    // The agent's own environment names the user; that must not read as a person's approval.
    assert_eq!(c["approval"]["by"], Value::Null, "{c}");
    // The project is not a git repository here.
    assert_eq!(c["base_sha"], Value::Null, "{c}");
}

/// What is saved is what is checked: a malformed v2 field refuses the start and writes nothing.
#[test]
fn task_start_refuses_a_contract_that_would_not_load_again() {
    let d = dir("contract-v2-bad");
    let s = server(&d);
    let mut contract = common::four();
    contract["limits"] = json!({"turns": 0});
    let (is_error, text) = call(&s, "task_start", contract);
    assert!(is_error, "{text}");
    assert!(
        supervisor_claude_code::lifecycle::active_task(s.state_dir())
            .unwrap()
            .is_none()
    );
    assert!(!s.state_dir().join("tasks").exists());
}

/// A caller's newer schema is refused, not silently stored as v2.
#[test]
fn task_start_refuses_a_newer_schema() {
    let d = dir("contract-v3");
    let s = server(&d);
    let mut contract = common::four();
    contract["schema_version"] = json!(3);
    let (is_error, text) = call(&s, "task_start", contract);
    assert!(is_error, "{text}");
    assert!(text.contains("schema_version 3"), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    assert!(!s.state_dir().join("tasks").exists());
}

/// WP4: task_state shows a task's limits with turns used and elapsed seconds, never a cost.
#[test]
fn task_state_shows_limits_and_their_use() {
    let d = dir("limits");
    let s = server(&d);
    let mut contract = common::four();
    contract["limits"] = serde_json::json!({"turns":5,"wall_clock_s":600});
    common::ok(&s, "task_start", contract);
    let state = common::task_state(&s);
    let limits = &state["limits"];
    assert_eq!(limits["turns"], 5, "{state}");
    assert_eq!(limits["wall_clock_s"], 600, "{state}");
    // MCP calls are not turns: only judged PreToolUse hook calls count.
    assert_eq!(limits["turns_used"], 0, "{state}");
    assert!(limits["elapsed_s"].as_u64().is_some(), "{state}");
    assert!(!state.to_string().contains("cost"), "{state}");

    let d = dir("no-limits");
    let s = server(&d);
    common::ok(&s, "task_start", common::four());
    assert_eq!(common::task_state(&s).get("limits"), None);
}

struct Fake;
impl supervisor_claude_code::candidate::CandidateProvider for Fake {
    fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
        Ok(coding_supervisor::Candidate {
            id: coding_supervisor::CandidateId {
                base_commit: "base".into(),
                dirty_digest: "one".into(),
            },
            provenance: "fake".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}

/// WP4 review: completing a person-started task through MCP on caller-asserted receipts does not
/// lift its limit; the hooks still refuse shell work past it and leave the task completed.
#[test]
fn a_task_completed_through_mcp_still_refuses_hook_work_past_its_limit() {
    use supervisor_claude_code::adapter::{Decision, Env, handle};
    let d = dir("limits-completed");
    let s = server(&d);
    let mut contract = common::four();
    contract["limits"] = json!({"turns":1});
    common::ok(&s, "task_start", contract);
    common::observe(&s, "one", &["src/lib.rs"]);
    for criterion in ["C1", "C2", "C3", "C4"] {
        common::pass(&s, criterion);
    }
    common::ok(&s, "accept_try", json!({}));
    assert_eq!(
        common::state(&common::ok(&s, "task_complete", json!({}))),
        "completed"
    );
    let mut env = Env {
        state_dir: s.state_dir().to_path_buf(),
        state_location: "test",
        candidate: Box::new(Fake),
        clock: supervisor_claude_code::adapter::now,
        level: supervisor_claude_code::level::Setting {
            level: supervisor_claude_code::level::Level::Enforce,
            ..Default::default()
        },
    };
    let bash = |id: &str| {
        json!({"hook_event_name":"PreToolUse","cwd":d.0.join("project"),"session_id":"session",
            "agent_id":"main","tool_use_id":id,"tool_name":"Bash",
            "tool_input":{"command":"echo x > out.txt"}})
    };
    assert_eq!(handle(&bash("b1"), &mut env), Decision::None);
    let denied = handle(&bash("b2"), &mut env);
    assert!(
        matches!(&denied, Decision::Deny(why) if why.contains("task limit reached: 1 of 1 turns used")),
        "{denied:?}"
    );
    assert_eq!(common::state(&common::task_state(&s)), "completed");
}

/// WP4 review round 3: a wall-clock-only contract shows no turn count.
#[test]
fn task_state_shows_no_turn_count_for_a_wall_clock_only_limit() {
    let d = dir("limits-wall-only");
    let s = server(&d);
    let mut contract = common::four();
    contract["limits"] = json!({"wall_clock_s":600});
    common::ok(&s, "task_start", contract);
    let state = common::task_state(&s);
    assert_eq!(state["limits"]["wall_clock_s"], 600, "{state}");
    assert_eq!(state["limits"]["turns"], Value::Null, "{state}");
    assert_eq!(state["limits"]["turns_used"], Value::Null, "{state}");
}

/// WP5: `task_state` shows the review policy and the latest review for the checkpoint's
/// candidate; the MCP server records no review, so a review-required task does not reach review.
#[test]
fn task_state_shows_the_review_policy_and_the_latest_review() {
    use coding_supervisor::{ReviewDecision, ReviewProvenance, ReviewRecord};
    use supervisor_claude_code::state::{StoredReview, Stored, load_gate, read_contract, save_gate};
    let d = dir("review");
    let s = server(&d);
    let mut contract = common::four();
    contract["review"] = json!({"required":true,"independent":false});
    common::ok(&s, "task_start", contract);
    common::observe(&s, "one", &["src/lib.rs"]);
    for c in ["C1", "C2", "C3", "C4"] {
        common::pass(&s, c);
    }
    let why = common::refused(&s, "accept_try", json!({}));
    assert!(why.contains("review"), "{why}");
    let state = common::task_state(&s);
    assert_eq!(
        state["review"],
        json!({"required":true,"independent":false,"latest":null}),
        "{state}"
    );
    let task = supervisor_claude_code::lifecycle::active_task(s.state_dir())
        .unwrap()
        .unwrap();
    let (_, digest) = read_contract(&task.dir).unwrap();
    let Stored::Valid(mut gate) = load_gate(&task.dir, &digest).unwrap() else {
        panic!("no valid gate");
    };
    let candidate = gate.checkpoint.candidate.clone().unwrap();
    gate.reviews.push(StoredReview {
        review: ReviewRecord {
            reviewer: "r1".into(),
            model: Some("claude-haiku-5-5".into()),
            decision: ReviewDecision::Approved,
            findings: vec![],
            at: 5,
            candidate,
            provenance: ReviewProvenance::Observed,
        },
        tree: None,
    });
    save_gate(&task.dir, &gate, &digest).unwrap();
    let state = common::task_state(&s);
    assert_eq!(
        state["review"]["latest"],
        json!({"reviewer":"r1","model":"claude-haiku-5-5","decision":"approved","provenance":"observed","findings":0,"at":5}),
        "{state}"
    );
    common::ok(&s, "accept_try", json!({}));
}
