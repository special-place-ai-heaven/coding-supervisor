//! Journal format v1: one events.jsonl line per hook call. Identifiers and counts only; no
//! prompt text, no worker message, no tool input beyond the CHECK lines already in `reasons`.

use coding_supervisor::{CandidateId, TaskState};
use serde_json::Value;

use crate::header::{self, Verdict};
use crate::state::{candidate_json, obj, state_name};

/// Facts from the gate that the payload does not carry.
#[derive(Default)]
pub(crate) struct Trace {
    pub task_id: Option<String>,
    pub candidate: Option<CandidateId>,
    pub state_before: Option<TaskState>,
    pub state_after: Option<TaskState>,
    pub tampered: bool,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// `(kind, text)` of a decision as emitted on stdout (`Value::Null` for no decision).
pub fn decision_parts(raw: &Value) -> (&'static str, Option<String>) {
    let owned = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
    if raw.is_null() {
        return ("none", None);
    }
    if text(raw, "decision") == Some("block") {
        return ("block", owned(raw.get("reason")));
    }
    if let Some(message) = raw.get("systemMessage") {
        return ("system", owned(Some(message)));
    }
    let specific = raw.get("hookSpecificOutput");
    if specific.and_then(|s| text(s, "permissionDecision")) == Some("deny") {
        return (
            "deny",
            owned(specific.and_then(|s| s.get("permissionDecisionReason"))),
        );
    }
    (
        "context",
        owned(specific.and_then(|s| s.get("additionalContext"))),
    )
}

/// The reason part of a refusal text: after `refused: ` / `refuses ...: `, without the
/// trailing instruction.
fn reason_part(text: &str) -> String {
    let rest = text
        .find("refuse")
        .and_then(|i| text.get(i..))
        .and_then(|s| s.split_once(": "))
        .map_or(text, |(_, r)| r);
    let end = [". ", "; record the check", "; restore the state"]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    rest.get(..end)
        .unwrap_or(rest)
        .trim_end_matches('.')
        .to_owned()
}

/// Criterion ids recorded, rejected by the core while linked, and unlinked, read from the
/// SubagentStop reasons. An unlinked line is also rejected by the core; it counts as unlinked only.
pub fn receipts_from_reasons(reasons: &[String]) -> [Vec<String>; 3] {
    let id = |rest: &str| rest.split(" | ").next().unwrap_or_default().to_owned();
    let (mut recorded, mut rejected, mut unlinked) = (Vec::new(), Vec::new(), Vec::new());
    for reason in reasons {
        if let Some(rest) = reason.strip_prefix("recorded CHECK ") {
            recorded.push(id(rest));
        } else if let Some(rest) = reason.strip_prefix("unlinked CHECK ") {
            unlinked.push(id(rest));
        } else if let Some(rest) = reason.strip_prefix("rejected CHECK ") {
            if !reason.contains("not linked to an observed run") {
                rejected.push(id(rest));
            }
        } else if let Some(rest) =
            reason.strip_prefix("not recorded, candidate not observed: CHECK ")
        {
            rejected.push(id(rest));
        }
    }
    [recorded, rejected, unlinked]
}

/// Worker usage from a completed foreground `Agent` result; `Null` otherwise.
pub fn usage(payload: &Value) -> Value {
    let response = payload.get("tool_response");
    let completed = text(payload, "hook_event_name") == Some("PostToolUse")
        && text(payload, "tool_name") == Some("Agent")
        && response.and_then(|r| text(r, "status")) == Some("completed");
    let Some(response) = response.filter(|_| completed) else {
        return Value::Null;
    };
    let field =
        |v: Option<&Value>, key: &str| v.and_then(|v| v.get(key)).cloned().unwrap_or(Value::Null);
    let inner = response.get("usage");
    obj([
        ("total_tokens", field(Some(response), "totalTokens")),
        ("input_tokens", field(inner, "input_tokens")),
        ("output_tokens", field(inner, "output_tokens")),
        (
            "cache_read_input_tokens",
            field(inner, "cache_read_input_tokens"),
        ),
        (
            "cache_creation_input_tokens",
            field(inner, "cache_creation_input_tokens"),
        ),
        ("duration_ms", field(Some(response), "totalDurationMs")),
        ("model", field(Some(response), "resolvedModel")),
    ])
}

pub fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Unverified => "UNVERIFIED",
    }
}

/// `{verdict, checks_run, lines, consistent}` of the worker header on SubagentStop; `Null` otherwise.
pub fn header_summary(payload: &Value) -> Value {
    if text(payload, "hook_event_name") != Some("SubagentStop") {
        return Value::Null;
    }
    match header::parse(text(payload, "last_assistant_message").unwrap_or_default()) {
        Some(h) => obj([
            ("verdict", verdict_name(h.verdict).into()),
            ("checks_run", h.checks_run.into()),
            ("lines", h.checks.len().into()),
            ("consistent", h.consistent.into()),
        ]),
        None => Value::Null,
    }
}

pub(crate) fn line(
    payload: &Value,
    state_location: &str,
    t: u64,
    decision: Option<Value>,
    reasons: &[String],
    trace: &Trace,
) -> Value {
    let event = text(payload, "hook_event_name").unwrap_or_default();
    let (kind, decision_text) = decision_parts(decision.as_ref().unwrap_or(&Value::Null));
    let reason = match (kind, &decision_text) {
        ("block" | "deny", Some(t)) => Some(reason_part(t)),
        _ => reasons.first().cloned(),
    };
    let [recorded, rejected, unlinked] = receipts_from_reasons(reasons);
    let accepted = event == "Stop"
        && trace.state_after == Some(TaskState::Completed)
        && trace.state_before != Some(TaskState::Completed);
    obj([
        ("v", 1.into()),
        ("t", t.into()),
        ("session_id", text(payload, "session_id").into()),
        ("prompt_id", text(payload, "prompt_id").into()),
        ("task_id", trace.task_id.clone().into()),
        ("state_location", state_location.into()),
        (
            "candidate",
            trace.candidate.as_ref().map(candidate_json).into(),
        ),
        ("event", event.into()),
        ("tool_name", text(payload, "tool_name").into()),
        (
            "agent_id",
            text(payload, "agent_id").unwrap_or("main").into(),
        ),
        ("agent_type", text(payload, "agent_type").into()),
        (
            "decision",
            obj([("kind", kind.into()), ("text", decision_text.into())]),
        ),
        ("reason", reason.into()),
        ("reasons", reasons.to_vec().into()),
        (
            "receipts",
            obj([
                ("recorded", recorded.into()),
                ("rejected", rejected.into()),
                ("unlinked", unlinked.into()),
            ]),
        ),
        ("header", header_summary(payload)),
        ("gate_state", trace.state_after.map(state_name).into()),
        ("accepted", accepted.into()),
        ("tampered", trace.tampered.into()),
        ("usage", usage(payload)),
    ])
}
