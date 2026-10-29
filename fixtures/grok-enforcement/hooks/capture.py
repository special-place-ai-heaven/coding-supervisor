"""Throwaway hook handler for the Grok Build enforcement investigation (Phase B).

Adapted from fixtures/claude-code-enforcement/hooks/capture.py. Every hook event in
this fixture runs this script. It appends the payload (strings truncated to 300
characters, nothing secret) to captures/<session>.events.jsonl, then applies the
decision configured in hooks/policy.json for that session. Grok sends camelCase
fields (hookEventName, sessionId, toolName, toolInput); Claude-style snake_case is
read as a fallback.
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
ENV_KEYS = ("GROK_HOOK_EVENT", "GROK_HOOK_NAME", "GROK_SESSION_ID", "GROK_WORKSPACE_ROOT")


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


def field(payload, camel, snake):
    return payload.get(camel, payload.get(snake))


def main():
    raw = sys.stdin.read()
    try:
        payload = json.loads(raw)
    except ValueError:
        payload = {"unparsed": raw[:TRUNCATE]}
    # hook_event_name carries the PascalCase name ("Stop"); hookEventName carries Grok's snake_case ("stop").
    event = payload.get("hook_event_name") or payload.get("hookEventName") or os.environ.get("GROK_HOOK_EVENT", "unknown")
    session = field(payload, "sessionId", "session_id") or os.environ.get("GROK_SESSION_ID", "nosession")
    policies = load(POLICY, {})
    policy = policies.get(session, policies.get("default", {}))
    # One receipt per run at fixture level: worker hooks run under the worker's own session id,
    # so a per-session file would be invisible to the chair's Stop. The runner clears it before each run.
    receipt_path = os.path.join(CAPTURES, "current.receipt.json")
    os.makedirs(CAPTURES, exist_ok=True)
    record = {
        "t": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "event": event,
        "keys": sorted(payload.keys()) if isinstance(payload, dict) else [],
        "env": {k: os.environ.get(k) for k in ENV_KEYS},
        "payload": truncate(payload),
        "candidate_digest_now": digest(),
    }
    decision = None
    exit_code = 0
    tool = field(payload, "toolName", "tool_name")

    if event == "PreToolUse":
        if policy.get("pre_sleep_seconds"):
            # Timeout test: sleep past the configured hook timeout, then try to deny.
            time.sleep(policy["pre_sleep_seconds"])
            decision = {"decision": "deny", "reason": "GATE: late deny after sleeping past the hook timeout."}
        elif tool and re.search(policy.get("pre_deny_regex", r"(?!)"), tool):
            decision = {
                "decision": "deny",
                "reason": (
                    f"GATE: {tool} refused; gate state is "
                    f"{load(GATE_STATE, {}).get('state')} and no current receipt exists."
                ),
            }
    elif event == "PostToolUse" and tool == "spawn_subagent":
        result = payload.get("toolResult") or {}
        text = result.get("output", "") if isinstance(result, dict) else str(result)
        reason = "GATE: worker claim is unsupported (no observed check run); not accepted."
        if "VERDICT: PASS" in text and "CHECKS RUN: 0" in text:
            mode = policy.get("post_agent_mode")
            if mode == "annotate":
                decision = {"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": reason}}
            elif mode == "replace":
                decision = {"hookSpecificOutput": {"hookEventName": "PostToolUse", "updatedToolOutput": "[GATE] " + reason}}
    elif event == "PostToolUse":
        # Receipt: an observed shell run of the check command, with the candidate digest at that moment.
        tool_input = field(payload, "toolInput", "tool_input") or {}
        if tool == "run_terminal_command" and "echo check-ok" in str(tool_input.get("command", "")):
            with open(receipt_path, "w", encoding="utf-8") as f:
                json.dump({"candidate_digest": digest(), "tool": tool, "command": tool_input.get("command"),
                           "session": session, "subagentType": payload.get("subagentType")}, f)
    elif event == "SubagentStop":
        if policy.get("subagent_block") and not field(payload, "stopHookActive", "stop_hook_active"):
            decision = {"decision": "block", "reason": "GATE: run the check and cite its exit code before finishing."}
            exit_code = policy.get("block_exit_code", 0)
    elif event == "Stop":
        state = load(GATE_STATE, {}).get("state", "unknown")
        receipt = load(receipt_path, None)
        stale = receipt is not None and receipt.get("candidate_digest") != digest()
        reasons = []
        if state != "completed":
            reasons.append(f"gate state is {state}, not completed")
        if stale:
            reasons.append(f"receipt is stale: candidate {receipt.get('candidate_digest')} != now {digest()}")
        if receipt is None:
            reasons.append("no receipt recorded")
        record["gate"] = {"state": state, "stale": stale, "reasons": reasons}
        # Only a genuine turn end can be gated; the session-end Stop (reason channel_closed/shutdown) is observe-only.
        turn_end = payload.get("reason") in (None, "end_turn")
        if policy.get("stop_mode", "block") == "block" and reasons and turn_end and (policy.get("stop_always") or not field(payload, "stopHookActive", "stop_hook_active")):
            decision = {"decision": "block", "reason": "GATE refuses completion: " + "; ".join(reasons)}
            exit_code = policy.get("block_exit_code", 0)

    record["decision"] = decision
    record["exit_code"] = exit_code
    with open(os.path.join(CAPTURES, f"{session}.events.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    if decision is not None:
        sys.stdout.write(json.dumps(decision))
        sys.stdout.flush()
        if exit_code == 2:
            sys.stderr.write(decision["reason"])
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
