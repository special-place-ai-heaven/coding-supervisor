//! T2 scenarios from tests/scenarios.rs replayed through the MCP tool calls, in-process.
//! Each test carries the core test's name prefix. The core's `record_worker_claim` has no
//! tool; a self-reported claim reaches the gate here as a receipt the core refuses.

mod common;

use std::fs;

use common::{
    contract, dir, observe, ok, pass, receipt, refused, server, started, state, task_state,
};
use serde_json::{Value, json};

#[test]
fn a18_self_reported_pass_is_not_accepted() {
    let d = dir("a18");
    let s = server(&d);
    started(&s);
    let text = refused(
        &s,
        "receipt_record",
        receipt("C1", "tests passed", json!(0), false),
    );
    assert!(
        text.contains("receipt is not linked to an observed run"),
        "{text}"
    );
    let text = refused(&s, "accept_try", json!({}));
    assert!(text.ends_with("accept_try refused: missing C1"), "{text}");
    let now = task_state(&s);
    assert_eq!(state(&now), "verifying");
    assert_eq!(now["receipts"]["C1"], "missing");
    let claims = now["rejected_claims"].as_array().unwrap();
    assert_eq!(claims.len(), 1);
    assert!(
        claims[0]
            .as_str()
            .unwrap()
            .starts_with("rejected CHECK C1 | tests passed | exit 0")
    );
}

#[test]
fn a19_candidate_change_makes_receipt_stale() {
    let d = dir("a19");
    let s = server(&d);
    started(&s);
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&s, c);
    }
    observe(&s, "two", &["src/lib.rs"]);
    assert_eq!(task_state(&s)["receipts"]["C1"], "stale");
    let text = refused(&s, "accept_try", json!({}));
    assert!(text.ends_with("refused: stale C1"), "{text}");
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&s, c);
    }
    assert_eq!(state(&ok(&s, "accept_try", json!({}))), "ready_for_review");
    assert_eq!(state(&ok(&s, "task_complete", json!({}))), "completed");
}

#[test]
fn a20_restart_restores_contract_budget_findings_candidate_receipts() {
    let d = dir("a20");
    let s = server(&d);
    started(&s);
    ok(
        &s,
        "finding_raise",
        json!({"id": "F1", "severity": "high", "location": "src/lib.rs:1", "evidence_ref": "review-1"}),
    );
    assert_eq!(state(&ok(&s, "repair_request", json!({}))), "repairing");
    assert_eq!(state(&observe(&s, "one", &["src/lib.rs"])), "verifying");
    pass(&s, "C1");
    let before = task_state(&s);
    let restarted = server(&d);
    let after = task_state(&restarted);
    assert_eq!(before, after);
    assert_eq!(state(&after), "verifying");
    assert_eq!(after["checkpoint"]["consumed_repair_rounds"], 1);
    assert_eq!(after["checkpoint"]["open_findings"][0]["id"], "F1");
    assert_eq!(after["checkpoint"]["candidate"]["dirty_digest"], "one");
    assert_eq!(after["receipts"]["C1"], "current");
    let text = refused(&restarted, "accept_try", json!({}));
    assert!(text.ends_with("refused: missing C2"), "{text}");
}

#[test]
fn r23_repair_cap_reached_never_completes() {
    let d = dir("r23");
    let s = server(&d);
    started(&s);
    ok(
        &s,
        "finding_raise",
        json!({"id": "F1", "severity": "high", "location": "src/lib.rs:1", "evidence_ref": "review-1"}),
    );
    for digest in ["two", "three"] {
        ok(&s, "repair_request", json!({}));
        observe(&s, digest, &["src/lib.rs"]);
    }
    let text = refused(&s, "repair_request", json!({}));
    assert!(
        text.ends_with("refused: repair budget exhausted after 2 rounds"),
        "{text}"
    );
    let restarted = server(&d);
    let now = task_state(&restarted);
    assert_eq!(state(&now), "budget_exhausted");
    assert_eq!(now["checkpoint"]["open_findings"][0]["id"], "F1");
    let text = refused(&restarted, "task_complete", json!({}));
    assert!(
        text.ends_with("complete is not allowed in state budget_exhausted"),
        "{text}"
    );
}

#[test]
fn repair_cap_exhausted_refuses_every_later_step() {
    let d = dir("cap");
    let s = server(&d);
    ok(&s, "task_start", contract(&[("C1", true)], 0));
    observe(&s, "one", &["src/lib.rs"]);
    let text = refused(&s, "repair_request", json!({}));
    assert!(
        text.ends_with("repair budget exhausted after 0 rounds"),
        "{text}"
    );
    for (tool, args) in [
        (
            "receipt_record",
            receipt("C1", "cargo test", json!(0), true),
        ),
        ("accept_try", json!({})),
        ("task_complete", json!({})),
        ("repair_request", json!({})),
    ] {
        let text = refused(&s, tool, args);
        assert!(
            text.contains("not allowed in state budget_exhausted"),
            "{text}"
        );
    }
    assert_eq!(state(&task_state(&s)), "budget_exhausted");
}

#[test]
fn r26_mandatory_regression_blocks_despite_aggregate() {
    let d = dir("r26");
    let s = server(&d);
    ok(
        &s,
        "task_start",
        contract(
            &[
                ("C1", true),
                ("C2", true),
                ("C3", true),
                ("C4", true),
                ("optional", false),
            ],
            2,
        ),
    );
    observe(&s, "one", &["src/lib.rs"]);
    for c in ["C1", "C2", "C4", "optional"] {
        pass(&s, c);
    }
    ok(
        &s,
        "receipt_record",
        receipt("C3", "cargo test", json!(1), true),
    );
    let text = refused(&s, "accept_try", json!({}));
    assert!(text.ends_with("refused: failing C3 exit 1"), "{text}");
    assert_eq!(state(&task_state(&s)), "verifying");
}

#[test]
fn r28_out_of_scope_change_fails_c4() {
    let d = dir("r28");
    let s = server(&d);
    started(&s);
    observe(&s, "two", &["src/lib.rs", "docs/out.md"]);
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&s, c);
    }
    let text = refused(&s, "accept_try", json!({}));
    assert!(
        text.ends_with("refused: out of scope docs/out.md"),
        "{text}"
    );
    // The changed paths survive a restart, so the scope check still applies.
    let text = refused(&server(&d), "accept_try", json!({}));
    assert!(
        text.ends_with("refused: out of scope docs/out.md"),
        "{text}"
    );
}

#[test]
fn r35_would_run_counts_as_not_run() {
    let d = dir("r35");
    let s = server(&d);
    started(&s);
    let text = refused(
        &s,
        "receipt_record",
        receipt("C1", "would run cargo test", Value::Null, true),
    );
    assert!(
        text.ends_with("refused: receipt has no exit code"),
        "{text}"
    );
    let text = refused(&s, "accept_try", json!({}));
    assert!(text.ends_with("refused: missing C1"), "{text}");
    assert_eq!(state(&task_state(&s)), "verifying");
}

#[test]
fn x1_receipt_without_exit_code_rejected() {
    let d = dir("x1");
    let s = server(&d);
    started(&s);
    let text = refused(
        &s,
        "receipt_record",
        receipt("C1", "cargo test", Value::Null, true),
    );
    assert!(
        text.ends_with("refused: receipt has no exit code"),
        "{text}"
    );
    let now = task_state(&s);
    assert_eq!(state(&now), "verifying");
    assert_eq!(now["receipts"]["C1"], "missing");
}

/// The core's ReceiptForOtherCandidate cannot be reached through MCP: the server binds every
/// receipt to the current candidate. This checks that binding instead.
#[test]
fn x2_receipt_is_bound_to_the_current_candidate() {
    let d = dir("x2");
    let s = server(&d);
    started(&s);
    pass(&s, "C1");
    observe(&s, "two", &["src/lib.rs"]);
    pass(&s, "C1");
    let gate: Value =
        serde_json::from_str(&fs::read_to_string(s.state_dir().join("gate.json")).unwrap())
            .unwrap();
    let receipts = gate["receipts"].as_array().unwrap();
    assert_eq!(receipts[0]["candidate"]["dirty_digest"], "one");
    assert_eq!(receipts[1]["candidate"]["dirty_digest"], "two");
    assert_eq!(receipts[1]["output_ref"], "mcp-asserted:out");
    assert_eq!(task_state(&s)["receipts"]["C1"], "current");
}

#[test]
fn tampered_gate_is_refused_and_never_reset() {
    let d = dir("tamper");
    let s = server(&d);
    started(&s);
    pass(&s, "C1");
    let path = s.state_dir().join("gate.json");
    let edited = fs::read_to_string(&path).unwrap().replace(
        "\"consumed_repair_rounds\":0",
        "\"consumed_repair_rounds\":1",
    );
    fs::write(&path, &edited).unwrap();
    for (tool, args) in [
        ("task_state", json!({})),
        ("accept_try", json!({})),
        (
            "candidate_observe",
            json!({"base_commit": "base", "dirty_digest": "x", "changed_paths": []}),
        ),
        ("task_start", common::four()),
    ] {
        let text = refused(&s, tool, args);
        assert!(
            text.contains("gate state tampered (checkpoint)"),
            "{tool}: {text}"
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);
}

#[test]
fn calls_before_task_start_are_refused() {
    let d = dir("nocontract");
    let s = server(&d);
    let text = refused(&s, "task_state", json!({}));
    assert!(text.contains("call task_start"), "{text}");
    ok(&s, "task_start", common::four());
    let text = refused(&s, "task_start", common::four());
    assert!(
        text.contains("already holds the contract of task mcp-test"),
        "{text}"
    );
    let text = refused(&s, "accept_try", json!({}));
    assert!(
        text.ends_with("refused: candidate identity not observed"),
        "{text}"
    );
}
