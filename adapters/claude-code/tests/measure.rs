use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use supervisor_claude_code::measure::{Report, collect};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path =
        std::env::temp_dir().join(format!("supervisor-measure-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    Dir(path)
}
fn write(path: &Path, lines: &[Value]) {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(path, text).unwrap();
}

fn v1(event: &str, extra: Value) -> Value {
    let mut line = json!({
        "v": 1, "t": 1, "session_id": "s1", "prompt_id": "p1", "task_id": "t", "state_location": "env",
        "candidate": null, "event": event, "tool_name": null, "agent_id": "main", "agent_type": null,
        "decision": {"kind": "none", "text": null}, "reason": null, "reasons": [],
        "receipts": {"recorded": [], "rejected": [], "unlinked": []}, "header": null,
        "gate_state": "verifying", "accepted": false, "tampered": false, "usage": null
    });
    for (k, v) in extra.as_object().unwrap() {
        line[k] = v.clone();
    }
    line
}

fn synthetic(d: &Dir) -> Report {
    let pass = json!({"verdict": "PASS", "checks_run": 1, "lines": 1, "consistent": true});
    write(
        &d.0.join("events.jsonl"),
        &[
            v1(
                "SubagentStop",
                json!({"agent_type": "judge", "header": pass,
                "receipts": {"recorded": ["C1"], "rejected": [], "unlinked": []}}),
            ),
            v1(
                "SubagentStop",
                json!({"agent_type": "judge", "header": pass,
                "receipts": {"recorded": [], "rejected": [], "unlinked": ["C1"]},
                "decision": {"kind": "block", "text": "GATE: handback refused (refusal 1 of 3): unlinked CHECK C1 | x | exit 0 (no run); missing C1. Run the check."}}),
            ),
            v1(
                "SubagentStop",
                json!({"agent_type": "fixer", "header": null,
                "decision": {"kind": "block", "text": "GATE: handback refused: no header. Start with VERDICT, then CHECKS RUN: <n>."}}),
            ),
            v1(
                "PostToolUse",
                json!({"tool_name": "Agent", "usage": {"total_tokens": 100}}),
            ),
            v1(
                "PreToolUse",
                json!({"decision": {"kind": "deny", "text": "GATE: Write refused: gate state tampered (receipt 0); restore the state from the journal or start a new task"}}),
            ),
            v1("Stop", json!({"accepted": true, "gate_state": "completed"})),
        ],
    );
    write(
        &d.0.join("L1.adapter-events.jsonl"),
        &[
            json!({"t": 1, "event": "SubagentStop", "agent_id": "a", "decision": null,
                "reasons": ["recorded CHECK C2 | echo ok | exit 0", "accepted: ready_for_review"]}),
            json!({"t": 2, "event": "SubagentStop", "agent_id": "b", "decision": null,
                "reasons": ["verdict is not PASS; journaled, not judged"]}),
            json!({"t": 3, "event": "SubagentStop", "agent_id": "c",
                "decision": {"decision": "block", "reason": "GATE: handback refused (refusal 1 of 3): unlinked CHECK C2 | echo ok | exit 0 (no run). Run the check."},
                "reasons": ["rejected CHECK C2 | echo ok | exit 0: receipt is not linked to an observed run", "unlinked CHECK C2 | echo ok | exit 0 (no run)"]}),
            json!({"t": 4, "event": "Stop", "agent_id": "main",
                "decision": {"decision": "block", "reason": "GATE refuses completion (refusal 1 of 3, harness cap 8): state verifying; stale C2; record the check with CHECK lines or report the task as not complete."},
                "reasons": ["stale C2"]}),
            json!({"t": 5, "event": "Stop", "agent_id": "main", "decision": null, "reasons": ["completed"]}),
        ],
    );
    let payload = |event: &str, extra: Value| {
        let mut p = json!({"session_id": "t3s", "hook_event_name": event});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    write(
        &d.0.join("abc.events.jsonl"),
        &[
            json!({"event": "SubagentStop", "decision": null, "payload": payload("SubagentStop",
                json!({"agent_type": "fake-worker", "last_assistant_message": "VERDICT: PASS\nCHECKS RUN: 0"}))}),
            json!({"event": "PostToolUse", "decision": {"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "x"}},
                "payload": payload("PostToolUse", json!({"tool_name": "Agent",
                    "tool_response": {"status": "completed", "totalTokens": 7086}}))}),
            json!({"event": "Stop", "decision": {"decision": "block", "reason": "GATE refuses completion: no receipt recorded"},
                "payload": payload("Stop", json!({}))}),
        ],
    );
    // Not a journal by name; must be ignored.
    write(&d.0.join("L1.runs.jsonl"), &[json!({"agent_id": "a"})]);
    collect(std::slice::from_ref(&d.0))
}

#[test]
fn three_shapes_are_counted_separately() {
    let d = dir("shapes");
    let report = synthetic(&d);
    assert_eq!(report.files_read.len(), 3);
    assert_eq!(report.unrecognized_lines, 0);

    let v1 = &report.sources["gate-v1"];
    assert_eq!(v1.sessions.len(), 1);
    assert_eq!(v1.dispatches, 3);
    assert_eq!(v1.by_agent_type["judge"], 2);
    assert_eq!(v1.headers, 2);
    assert_eq!(
        (v1.pass_supported, v1.pass_unsupported, v1.no_header),
        (1, 1, 1)
    );
    assert_eq!((v1.recorded, v1.rejected, v1.unlinked), (1, 0, 1));
    assert_eq!(v1.refusals, 3);
    assert_eq!(v1.categories["unlinked"], 1);
    assert_eq!(v1.categories["no_header"], 1);
    assert_eq!(v1.categories["tampered"], 1);
    assert_eq!(v1.tamper_refusals, 1);
    assert_eq!(v1.accepted, 1);
    assert_eq!((v1.usage_lines, v1.total_tokens), (1, 100));

    let legacy = &report.sources["gate-legacy"];
    assert_eq!(legacy.sessions.len(), 1);
    assert_eq!(legacy.dispatches, 3);
    assert_eq!(legacy.by_agent_type["unknown"], 3);
    assert_eq!(
        (
            legacy.pass_supported,
            legacy.pass_unsupported,
            legacy.not_pass_unknown
        ),
        (1, 1, 1)
    );
    // The unlinked line is also a core rejection; it counts once, as unlinked.
    assert_eq!(
        (legacy.recorded, legacy.rejected, legacy.unlinked),
        (1, 0, 1)
    );
    assert_eq!(legacy.refusals, 2);
    assert_eq!(legacy.categories["stale"], 1);
    assert_eq!(legacy.categories["unlinked"], 1);
    assert_eq!(legacy.accepted, 1);
    assert_eq!(legacy.usage_lines, 0);

    let t3 = &report.sources["t3-capture"];
    assert_eq!(t3.sessions.len(), 1);
    assert_eq!(t3.dispatches, 1);
    assert_eq!(t3.by_agent_type["fake-worker"], 1);
    assert_eq!((t3.headers, t3.pass_unjudged), (1, 1));
    assert_eq!(t3.refusals, 1);
    assert_eq!(t3.categories["other"], 1);
    assert_eq!(t3.accepted, 0);
    assert_eq!((t3.usage_lines, t3.total_tokens), (1, 7086));

    let md = report.markdown();
    assert!(md.contains("no USD cost"));
    assert!(md.contains("| gate-derived worker tokens (final request only) | n/a (0 usage lines) | 100/1 = 100 (1 usage lines) | n/a (1 usage lines) |"));
    for line in md
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| measure"))
    {
        assert!(line.starts_with("| gate-derived "), "{line}");
    }
    let json = report.to_json();
    assert_eq!(
        json["sources"]["gate-v1"]["worker_tokens_per_accepted_task"],
        100
    );
    assert_eq!(
        json["sources"]["t3-capture"]["worker_tokens_per_accepted_task"],
        Value::Null
    );
}

#[test]
fn run3_live_journals() {
    let run3 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code-enforcement/captures/run3");
    let report = collect(&[run3]);
    assert!(report.unreadable.is_empty());
    assert_eq!(report.files_read.len(), 7);
    assert_eq!(report.sources.len(), 1);
    let legacy = &report.sources["gate-legacy"];
    // L2b is L2's journal plus the resume: all 7 of L2's lines repeat there and count once.
    assert_eq!(legacy.duplicates_dropped, 7);
    assert_eq!(legacy.lines, 52);
    // L1..L6 plus L2b, one file each; L2b keeps its 3 resume lines.
    assert_eq!(legacy.sessions.len(), 7);
    // SubagentStop: L1 4, L2 1, L3 3, L4 2, L6 1.
    assert_eq!(legacy.dispatches, 11);
    assert_eq!(legacy.headers, 10);
    assert_eq!(
        (
            legacy.pass_supported,
            legacy.pass_unsupported,
            legacy.no_header
        ),
        (4, 6, 1)
    );
    assert_eq!(
        (legacy.recorded, legacy.rejected, legacy.unlinked),
        (4, 0, 2)
    );
    // Blocks: L1 3 missing, L3 2 unlinked, L4 Stop stale, L5 Stop missing; deny: L2b terminal state.
    assert_eq!(legacy.refusals, 8);
    assert_eq!(legacy.categories["missing"], 4);
    assert_eq!(legacy.categories["unlinked"], 2);
    assert_eq!(legacy.categories["stale"], 1);
    assert_eq!(legacy.categories["terminal_state"], 1);
    // "completed" in L2, L4 and L6.
    assert_eq!(legacy.accepted, 3);
    assert_eq!(legacy.usage_lines, 0);
}

#[test]
fn identical_lines_in_two_files_count_once() {
    let d = dir("dedup");
    let line = v1("Stop", json!({"accepted": true, "gate_state": "completed"}));
    write(&d.0.join("a.events.jsonl"), std::slice::from_ref(&line));
    write(&d.0.join("b.events.jsonl"), &[line]);
    let report = collect(std::slice::from_ref(&d.0));
    assert_eq!(report.files_read.len(), 2);
    let v1 = &report.sources["gate-v1"];
    assert_eq!((v1.lines, v1.accepted, v1.duplicates_dropped), (1, 1, 1));
    assert!(report.markdown().contains("Duplicate lines dropped: 1."));
    assert_eq!(
        report.to_json()["sources"]["gate-v1"]["duplicates_dropped"],
        1
    );
}

#[test]
fn unreadable_paths_are_listed_and_skipped() {
    let d = dir("missing");
    let report = collect(&[d.0.join("nope.events.jsonl")]);
    assert_eq!(report.unreadable.len(), 1);
    assert!(report.files_read.is_empty());
}

#[test]
fn mcp_lines_count_apart_from_sessions_and_hook_acceptances() {
    let d = dir("mcp");
    let mcp = |event: &str, extra: Value| {
        let mut line = v1(event, extra);
        line["session_id"] = Value::Null;
        line["prompt_id"] = Value::Null;
        line["agent_id"] = Value::Null;
        line
    };
    write(
        &d.0.join("events.jsonl"),
        &[
            mcp("mcp:task_start", json!({})),
            mcp("mcp:accept_try", json!({"gate_state": "ready_for_review"})),
            mcp(
                "mcp:task_complete",
                json!({"accepted": true, "gate_state": "completed"}),
            ),
            v1(
                "Stop",
                json!({"decision": {"kind": "block", "text": "GATE refuses completion (refusal 1 of 3, harness cap 8): state completed; missing C2; record the check with CHECK lines or report the task as not complete."}}),
            ),
        ],
    );
    let report = collect(std::slice::from_ref(&d.0));
    let v1 = &report.sources["gate-v1"];
    assert_eq!(v1.lines, 4);
    assert_eq!(v1.sessions.len(), 1);
    assert_eq!(v1.no_session_lines, 3);
    assert_eq!(v1.accepted, 0);
    assert_eq!(v1.accepted_mcp, 1);
    let md = report.markdown();
    assert!(md.contains("| gate-derived sessions | 1 |"), "{md}");
    assert!(md.contains("| gate-derived tool-server lines (no session) | 3 |"));
    assert!(md.contains("| gate-derived accepted tasks | 0/1 |"));
    assert!(md.contains("| gate-derived accepted via MCP (advisory) | 1 |"));
    let json = &report.to_json()["sources"]["gate-v1"];
    assert_eq!(json["accepted_tasks"], 0);
    assert_eq!(json["accepted_via_mcp_advisory"], 1);
    assert_eq!(json["tool_server_lines_no_session"], 3);
}
