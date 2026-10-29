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
fn tools_list_has_the_eight_tools_with_required_fields() {
    let d = dir("list");
    let s = server(&d);
    let tools = rpc(&s, "tools/list", json!({}))["result"]["tools"].clone();
    let tools = tools.as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
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
    assert_eq!(
        required("receipt_record"),
        [
            "criterion",
            "command",
            "cwd",
            "exit_code",
            "output_ref",
            "claim_supported"
        ]
    );
    assert!(required("candidate_observe").is_empty());
    assert_eq!(
        required("finding_raise"),
        ["id", "severity", "location", "evidence_ref"]
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
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 8);
}
