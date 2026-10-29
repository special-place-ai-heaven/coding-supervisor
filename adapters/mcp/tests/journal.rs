//! events.jsonl written by the MCP server is journal format v1 and is read by `measure`.

mod common;

use std::fs;

use common::{GOAL, dir, ok, pass, receipt, refused, server, started};
use serde_json::{Value, json};
use supervisor_claude_code::measure::collect;

#[test]
fn every_call_is_one_v1_line_without_goal_text_and_measure_reads_it() {
    let d = dir("journal");
    let s = server(&d);
    started(&s);
    refused(
        &s,
        "receipt_record",
        receipt("C1", "cargo test", Value::Null, true),
    );
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&s, c);
    }
    ok(&s, "accept_try", json!({}));
    ok(&s, "task_complete", json!({}));
    let calls = 2 + 1 + 4 + 2;

    let path = s.state_dir().join("events.jsonl");
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains(GOAL));
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), calls);
    for line in &lines {
        assert_eq!(line["v"], 1, "{line}");
        assert!(
            line["event"].as_str().unwrap().starts_with("mcp:"),
            "{line}"
        );
        assert_eq!(line["task_id"], "mcp-test");
        assert_eq!(line["state_location"], "env");
        assert_eq!(line["agent_id"], Value::Null);
        assert_eq!(line["session_id"], Value::Null);
        assert_eq!(line["tool_name"], Value::Null);
    }
    let rejected = &lines[2];
    assert_eq!(rejected["event"], "mcp:receipt_record");
    assert_eq!(rejected["decision"]["kind"], "system");
    let has = |line: &Value, tail: &str| {
        line["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().ends_with(tail))
    };
    assert!(
        has(rejected, "receipt has no exit code (mcp-asserted)"),
        "{rejected}"
    );
    let recorded = &lines[3];
    assert_eq!(recorded["receipts"]["recorded"], json!(["C1"]));
    assert!(has(recorded, "| exit 0 (mcp-asserted)"), "{recorded}");
    let last = &lines[calls - 1];
    assert_eq!(last["event"], "mcp:task_complete");
    assert_eq!(last["gate_state"], "completed");
    assert_eq!(last["accepted"], true);
    assert_eq!(lines.iter().filter(|l| l["accepted"] == true).count(), 1);

    let report = collect(&[path]);
    assert_eq!(report.unrecognized_lines, 0);
    let stats = &report.sources["gate-v1"];
    assert_eq!(stats.lines, calls as u64);
    assert_eq!(stats.accepted, 1);
    assert_eq!(stats.recorded, 4);
    assert_eq!(stats.refusals, 0, "advisory answers are not refusals");
}
