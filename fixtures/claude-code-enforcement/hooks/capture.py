"""Throwaway hook handler for PLAN T3. Not the adapter (T4 is Rust).

Every hook event in the fixture runs this script. It appends the event payload
(strings truncated, nothing secret) to captures/<session_id>.events.jsonl, then
applies the decision configured in hooks/policy.json for that event.
"""

import hashlib
import json
import os
import re
import sys
import time

FIXTURE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CAPTURES = os.path.join(FIXTURE, "captures")
POLICY = os.path.join(FIXTURE, "hooks", "policy.json")
GATE_STATE = os.path.join(FIXTURE, "gate-state.json")
CANDIDATE = os.path.join(FIXTURE, "candidate.txt")
TRUNCATE = 300


def truncate(value):
    if isinstance(value, str):
        return value if len(value) <= TRUNCATE else value[:TRUNCATE] + f"...[+{len(value) - TRUNCATE}]"
    if isinstance(value, list):
        return [truncate(v) for v in value]
    if isinstance(value, dict):
        return {k: truncate(v) for k, v in value.items()}
    return value


def digest():
    try:
        with open(CANDIDATE, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()[:16]
    except OSError:
        return "missing"


def load(path, default):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def emit(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.flush()


def worker_text(payload):
    resp = payload.get("tool_response")
    if isinstance(resp, dict):
        parts = [c.get("text", "") for c in resp.get("content", []) if isinstance(c, dict)]
        return "\n".join(parts)
    return ""


def main():
    raw = sys.stdin.read()
    try:
        payload = json.loads(raw)
    except ValueError:
        payload = {"unparsed": raw[:TRUNCATE]}
    event = payload.get("hook_event_name", "unknown")
    session = payload.get("session_id", "nosession")
    policies = load(POLICY, {})
    policy = policies.get(session, policies.get("default", {}))
    receipt_path = os.path.join(CAPTURES, f"{session}.receipt.json")
    os.makedirs(CAPTURES, exist_ok=True)
    record = {
        "t": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "event": event,
        "keys": sorted(payload.keys()),
        "payload": truncate(payload),
        "candidate_digest_now": digest(),
    }
    decision = None

    if event == "PreToolUse":
        tool = payload.get("tool_name")
        if tool in policy.get("pre_deny_tools", []):
            decision = {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": (
                        f"GATE: {tool} refused; gate state is "
                        f"{load(GATE_STATE, {}).get('state')} and no current receipt exists."
                    ),
                }
            }
    elif event == "PostToolUse" and payload.get("tool_name") == "Agent":
        text = worker_text(payload)
        resp = payload.get("tool_response") or {}
        unsupported = "CHECKS RUN: 0" in text or ("VERDICT" in text and "exit code" not in text.lower())
        if resp.get("status") == "completed" and "VERDICT: PASS" in text:
            with open(receipt_path, "w", encoding="utf-8") as f:
                json.dump({"candidate_digest": digest(), "claim": "VERDICT: PASS", "checks_run": 0,
                           "agent_id": resp.get("agentId"), "session": session}, f)
        mode = policy.get("post_agent_mode", "annotate")
        reason = "GATE: worker claim is unsupported (no check ran, no exit code cited); not accepted."
        if unsupported and mode == "annotate":
            decision = {"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": reason}}
        elif unsupported and mode == "block":
            decision = {"decision": "block", "reason": reason}
        elif unsupported and mode == "replace":
            decision = {
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "updatedToolOutput": {**resp, "content": [{"type": "text", "text": "[GATE] " + reason}]},
                }
            }
    elif event == "SubagentStop":
        if policy.get("subagent_block_once") and not payload.get("stop_hook_active"):
            decision = {"decision": "block", "reason": "GATE: run the check and cite its exit code before finishing."}
    elif event == "Stop":
        state = load(GATE_STATE, {}).get("state", "unknown")
        last = payload.get("last_assistant_message", "") or ""
        claims_done = re.search(r"\b(done|complete|completed|finished)\b", last, re.I) is not None
        receipt = load(receipt_path, None)
        stale = receipt is not None and receipt.get("candidate_digest") != digest()
        reasons = []
        if state != "completed":
            reasons.append(f"gate state is {state}, not completed")
        if stale:
            reasons.append(f"receipt is stale: candidate {receipt.get('candidate_digest')} != now {digest()}")
        if receipt is None:
            reasons.append("no receipt recorded")
        record["gate"] = {"state": state, "claims_done": claims_done, "stale": stale, "reasons": reasons}
        if policy.get("stop_mode", "block") == "block" and claims_done and reasons and not payload.get("stop_hook_active"):
            decision = {"decision": "block", "reason": "GATE refuses completion: " + "; ".join(reasons)}

    record["decision"] = decision
    with open(os.path.join(CAPTURES, f"{session}.events.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    if decision is not None:
        emit(decision)
    sys.exit(0)


if __name__ == "__main__":
    main()
