"""Scripted OpenAI-compatible model endpoint for the Grok enforcement fixture.

Live xAI sessions were blocked (no sign-in on this machine), so the harness's hook
mechanics are exercised against this deterministic stand-in. It plays the chair and
the fixture workers from a fixed script chosen by the "SCENARIO: <name>" marker in
the conversation, and logs a truncated view of every request it receives to
captures/model/<scenario>.requests.jsonl, so the captures show exactly what Grok fed
the model (hook reasons, subagent results, skill listings).

Usage: python server.py <port>
"""

import json
import os
import re
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FIXTURE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOGDIR = os.path.join(FIXTURE, "captures", "model")
TRUNC = 300
DONE = "Done, the task is complete."


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def cut(s):
    return s if len(s) <= TRUNC else s[:TRUNC] + f"...[+{len(s) - TRUNC}]"


def tool(name, args):
    return {"tool": name, "args": args}


def scenario_of(msgs):
    for m in msgs:
        found = re.search(r"SCENARIO: ([a-z-]+)", text_of(m.get("content")))
        if found:
            return found.group(1)
    return "discovery"


def role_of(msgs):
    system = "\n".join(text_of(m.get("content")) for m in msgs if m.get("role") in ("system", "developer"))
    first_user = next((text_of(m.get("content")) for m in msgs if m.get("role") == "user"), "")
    blob = system + "\n" + first_user
    if "WORKER-CITED" in blob:
        return "cited-worker"
    if "WORKER-FAKE" in blob:
        return "fake-worker"
    return "chair"


TASK = os.environ.get("FAKE_TASK_TOOL", "spawn_subagent")
SHELL = os.environ.get("FAKE_SHELL_TOOL", "run_terminal_command")


def task_args(agent, background=None):
    args = {"subagent_type": agent, "description": "fixture check", "prompt": "SUBTASK: report the verdict for candidate.txt"}
    if background is not None:
        args["background"] = background
    return args


def shell_args(cmd):
    return {"command": cmd, "description": "fixture command"}


SCRIPTS = {
    "deny": [tool(SHELL, shell_args("echo denied-probe > out.txt")), "The shell command was refused by a hook; nothing was written."],
    "timeout": [tool(SHELL, shell_args("echo timeout-probe > out-timeout.txt")), "The command ran."],
    "missing": [tool(TASK, task_args("fake-worker")), DONE, "Withdrawn: the gate refused completion, no receipt exists."],
    "missing-fg": [tool(TASK, task_args("fake-worker", False)), DONE, "Withdrawn: the gate refused completion, no receipt exists."],
    "subblock": [tool(TASK, task_args("fake-worker", False)), DONE, "Withdrawn: the gate refused completion."],
    "stale": [tool(TASK, task_args("cited-worker", False)), tool("write", {"file_path": os.path.join(FIXTURE, "candidate.txt"), "content": "candidate v2\n"}), DONE,
              "Withdrawn: the receipt is stale because candidate.txt changed."],
    "bypass": [DONE, "Withdrawn: the gate refused completion."],
    "discovery": ["OK"],
}


def next_action(scenario, role, msgs):
    n_asst = sum(1 for m in msgs if m.get("role") == "assistant")
    if role == "fake-worker":
        return "VERDICT: PASS\nCHECKS RUN: 0"
    if role == "cited-worker":
        return tool(SHELL, shell_args("echo check-ok")) if n_asst == 0 else \
            "VERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | echo check-ok | exit 0"
    script = SCRIPTS.get(scenario, SCRIPTS["discovery"])
    return script[min(n_asst, len(script) - 1)]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            self._json(200, {"object": "list", "data": [{"id": "fake-chair", "object": "model", "owned_by": "fixture"}]})
        else:
            self._json(404, {"error": "not found"})

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        msgs = req.get("messages", [])
        scenario, role = scenario_of(msgs), role_of(msgs)
        if role != "chair":
            scenario = role  # workers carry no scenario marker; log them under their own name
        action = next_action(scenario, role, msgs)
        os.makedirs(LOGDIR, exist_ok=True)
        if scenario == "discovery" and req.get("tools"):
            with open(os.path.join(LOGDIR, "tool-schemas.json"), "w", encoding="utf-8") as f:
                json.dump({t["function"]["name"]: t["function"].get("parameters") for t in req["tools"]
                           if t.get("function", {}).get("name") in (TASK, SHELL, "write", "search_replace")}, f, indent=1)
        with open(os.path.join(LOGDIR, f"{scenario}.requests.jsonl"), "a", encoding="utf-8") as f:
            f.write(json.dumps({
                "t": time.strftime("%Y-%m-%dT%H:%M:%S"), "path": self.path, "role_played": role,
                "stream": req.get("stream"), "tools": [t.get("function", {}).get("name") for t in req.get("tools", [])],
                "messages": [{"role": m.get("role"), "text": cut(text_of(m.get("content"))),
                              "tool_calls": [c.get("function", {}).get("name") for c in m.get("tool_calls") or []]}
                             for m in msgs],
                "markers_in_request": [w for w in ("ZEBRA-GROK-SKILL", "OTTER-AGENTS-SKILL") if w in json.dumps(req)],
                "reply": action,
            }) + "\n")
        if isinstance(action, dict):
            call = {"index": 0, "id": "call_" + uuid.uuid4().hex[:12], "type": "function",
                    "function": {"name": action["tool"], "arguments": json.dumps(action["args"])}}
            message, finish = {"role": "assistant", "content": None, "tool_calls": [call]}, "tool_calls"
        else:
            message, finish = {"role": "assistant", "content": action}, "stop"
        usage = {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        rid, model = "chatcmpl-" + uuid.uuid4().hex[:12], req.get("model", "fake-chair")
        if not req.get("stream"):
            self._json(200, {"id": rid, "object": "chat.completion", "created": int(time.time()), "model": model,
                             "choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": usage})
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        chunks = [{"choices": [{"index": 0, "delta": message, "finish_reason": None}]},
                  {"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": usage}]
        for c in chunks:
            c.update({"id": rid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model})
            self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18811
    print(f"fake model listening on 127.0.0.1:{port}", flush=True)
    # Reuse off: on Windows address reuse lets a second server share the port silently and steal requests.
    ThreadingHTTPServer.allow_reuse_address = False
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
