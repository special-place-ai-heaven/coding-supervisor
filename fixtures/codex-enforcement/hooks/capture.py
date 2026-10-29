"""Throwaway Codex CLI hook handler for the Phase B investigation. Not the adapter.

Every hook event in the fixture runs this script. It appends the payload
(strings truncated to 300 characters, credential-looking keys dropped) to
captures/<session_id>.events.jsonl, then applies the decision configured in
hooks/policy.json under the test named by policy["active"].

Codex output rules this follows (developers.openai.com/codex/hooks):
- PreToolUse deny: hookSpecificOutput.permissionDecision = "deny".
- PostToolUse: additionalContext, or decision "block" (replaces the tool result).
- SubagentStop / Stop: JSON only on stdout; decision "block" + reason continues.
- Exit 0 with no output means "no decision".
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
SECRET_KEY = re.compile(r"(key|token|secret|password|auth|bearer|cookie)", re.I)
CLAIM = re.compile(r"\b(done|complete|completed|finished)\b", re.I)
NEGATED = re.compile(r"\b(not|n't|cannot|no|unable)\b[^.\n]{0,40}\b(done|complete|completed|finished)\b", re.I)
CHECK_LINE = re.compile(r"CHECK\s+(\S+)\s*\|\s*(.+?)\s*\|\s*exit\s+(-?\d+)", re.I)


def clean(value):
    if isinstance(value, str):
        return value if len(value) <= TRUNCATE else value[:TRUNCATE] + f"...[+{len(value) - TRUNCATE}]"
    if isinstance(value, list):
        return [clean(v) for v in value]
    if isinstance(value, dict):
        return {k: ("[dropped]" if SECRET_KEY.search(k) else clean(v)) for k, v in value.items()}
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


def save(path, obj):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f)


def as_text(value):
    return value if isinstance(value, str) else json.dumps(value)


def claims_done(text):
    return CLAIM.search(text) is not None and NEGATED.search(text) is None


def main():
    raw = sys.stdin.read()
    try:
        payload = json.loads(raw)
    except ValueError:
        payload = {"unparsed": raw[:TRUNCATE]}
    event = payload.get("hook_event_name", "unknown")
    session = payload.get("session_id", "nosession")
    policies = load(POLICY, {})
    test = policies.get("active", "default")
    policy = {**policies.get("default", {}), **policies.get(test, {})}
    os.makedirs(CAPTURES, exist_ok=True)
    receipt_path = os.path.join(CAPTURES, f"{session}.receipt.json")
    state_path = os.path.join(CAPTURES, f"{session}.hookstate.json")
    hookstate = load(state_path, {"bash_commands": [], "stop_blocks": 0, "subagent_blocks": 0})
    record = {
        "t": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "test": test,
        "event": event,
        "keys": sorted(payload.keys()),
        "payload": clean(payload),
        "candidate_digest_now": digest(),
    }
    decision = None
    tool = payload.get("tool_name")

    if event == "PreToolUse" and tool in policy.get("pre_deny_tools", []):
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
    elif event == "PostToolUse":
        response = as_text(payload.get("tool_response"))
        if tool == "Bash":
            hookstate["bash_commands"].append(as_text(payload.get("tool_input", {}).get("command", "")))
        if "VERDICT" in response:
            # Result delivery to the chair: record a receipt, then annotate or block.
            cited = CHECK_LINE.findall(response)
            observed = any(c[1] in cmd for c in cited for cmd in hookstate["bash_commands"])
            save(receipt_path, {"candidate_digest": digest(), "via": f"PostToolUse:{tool}",
                                "cited_checks": cited, "cited_command_observed": observed})
            unsupported = not cited or not observed
            reason = ("GATE: worker claim is unsupported (no check ran, or the cited command was not observed); "
                      "not accepted.")
            mode = policy.get("post_result_mode", "none")
            if unsupported and mode == "annotate":
                decision = {"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": reason}}
            elif unsupported and mode == "block":
                decision = {"decision": "block", "reason": reason}
            record["gate"] = {"cited": cited, "observed": observed, "unsupported": unsupported}
    elif event == "SubagentStop":
        last = payload.get("last_assistant_message") or ""
        if "VERDICT" in last and not os.path.exists(receipt_path):
            cited = CHECK_LINE.findall(last)
            observed = any(c[1] in cmd for c in cited for cmd in hookstate["bash_commands"])
            save(receipt_path, {"candidate_digest": digest(), "via": "SubagentStop",
                                "cited_checks": cited, "cited_command_observed": observed})
        if policy.get("subagent_block_once") and not payload.get("stop_hook_active"):
            hookstate["subagent_blocks"] += 1
            decision = {"decision": "block", "reason": "GATE: run the check and cite its exit code before finishing."}
    elif event == "Stop":
        state = load(GATE_STATE, {}).get("state", "unknown")
        last = payload.get("last_assistant_message") or ""
        receipt = load(receipt_path, None)
        now = digest()
        stale = receipt is not None and receipt.get("candidate_digest") != now
        reasons = []
        if state != "completed":
            reasons.append(f"gate state is {state}, not completed")
        if receipt is None:
            reasons.append("no receipt recorded")
        elif stale:
            reasons.append(f"receipt is stale: candidate {receipt.get('candidate_digest')} != now {now}")
        elif not receipt.get("cited_command_observed"):
            reasons.append("receipt cites no observed command")
        claimed = claims_done(last)
        record["gate"] = {"state": state, "claims_done": claimed, "stale": stale, "reasons": reasons}
        mode = policy.get("stop_mode", "block")
        if mode == "always" and hookstate["stop_blocks"] < policy.get("stop_cap", 10):
            # Continuation-cap probe: ignore stop_hook_active, bounded by our own cap.
            hookstate["stop_blocks"] += 1
            decision = {"decision": "block", "reason": f"GATE probe continuation {hookstate['stop_blocks']}: reply with the single word again."}
        elif mode == "block" and claimed and reasons and not payload.get("stop_hook_active"):
            hookstate["stop_blocks"] += 1
            decision = {"decision": "block", "reason": "GATE refuses completion: " + "; ".join(reasons)}

    record["decision"] = decision
    save(state_path, hookstate)
    with open(os.path.join(CAPTURES, f"{session}.events.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    if decision is not None:
        sys.stdout.write(json.dumps(decision))
        sys.stdout.flush()
    sys.exit(0)


if __name__ == "__main__":
    main()
