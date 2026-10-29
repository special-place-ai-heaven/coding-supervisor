"""Scripted stand-in for the Responses API so Codex hooks can be observed without a live model.

Serves POST /v1/responses as server-sent events; Codex is pointed at it per run with
-c model_providers / model_provider (see run_codex.py). Workers are recognised by their
developer instructions; the chair follows the steps in scripts.json for the scenario
named in $MOCK_STATE_DIR/scenario.txt. Every request body is logged to
$MOCK_STATE_DIR/requests.jsonl (default C:/temp/codex-t3), never into the repository,
because it carries the user's global instructions.
"""

import json
import os
import sys
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
# Request logs hold the user's global instructions; they stay outside the repository.
STATE = os.environ.get("MOCK_STATE_DIR", "C:/temp/codex-t3")
LOG = os.path.join(STATE, "requests.jsonl")


def scenario():
    try:
        return open(os.path.join(STATE, "scenario.txt"), encoding="utf-8").read().strip()
    except OSError:
        return "probe"


def texts(item):
    out = []
    c = item.get("content")
    if isinstance(c, str):
        out.append(c)
    elif isinstance(c, list):
        for p in c:
            if isinstance(p, dict) and isinstance(p.get("text"), str):
                out.append(p["text"])
    if isinstance(item.get("output"), str):
        out.append(item["output"])
    return "\n".join(out)


def msg(text):
    return {"type": "message", "role": "assistant", "id": "msg_" + uuid.uuid4().hex[:12],
            "content": [{"type": "output_text", "text": text, "annotations": []}]}


def call(name, args):
    return {"type": "function_call", "id": "fc_" + uuid.uuid4().hex[:12], "call_id": "call_" + uuid.uuid4().hex[:12],
            "name": name, "arguments": json.dumps(args)}


def tool_names(body):
    names = []
    for t in body.get("tools", []):
        names.append(t.get("name") or (t.get("function") or {}).get("name") or t.get("type"))
    return names


def xcall(js):
    return {"type": "custom_tool_call", "id": "ctc_" + uuid.uuid4().hex[:12], "call_id": "call_" + uuid.uuid4().hex[:12],
            "name": "exec", "input": js}


def wrap(js):
    return "try {\n" + js + "\n} catch (e) { text('REJECTED: ' + String(e && e.message || e)); }"




def decide(body):
    sc = scenario()
    inp = [i for i in body.get("input", []) if isinstance(i, dict)]
    alltext = (body.get("instructions") or "") + "\n" + "\n".join(texts(i) for i in inp)
    outputs = [i for i in inp if i.get("type") in ("custom_tool_call_output", "function_call_output")]
    users = [texts(i) for i in inp if i.get("type") == "message" and i.get("role") == "user"]
    last_user = users[-1] if users else ""
    if "CHECKS RUN: 0" in alltext and "Use no tools" in alltext:
        return [msg("VERDICT: PASS\nCHECKS RUN: 0")]
    if "Run exactly one shell command" in alltext:
        if not outputs:
            return [xcall(wrap("const r = await tools.exec_command({cmd: 'echo check-ok'}); text(JSON.stringify(r));"))]
        return [msg("VERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | echo check-ok | exit 0")]
    s = json.load(open(os.path.join(HERE, "scripts.json"), encoding="utf-8")).get(sc, {"steps": [], "final": "unknown scenario"})
    if "GATE" in last_user and s.get("withdraw"):
        return [msg(s["withdraw"])]
    if len(outputs) < len(s["steps"]):
        return [xcall(wrap(s["steps"][len(outputs)]))]
    return [msg(s["final"])]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        body = json.dumps({"data": [], "models": []}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n)
        try:
            body = json.loads(raw)
        except ValueError:
            body = {}
        items = decide(body)
        with open(LOG, "a", encoding="utf-8") as f:
            f.write(json.dumps({"path": self.path, "scenario": scenario(), "body": body, "reply": items}) + "\n")
        rid = "resp_" + uuid.uuid4().hex[:12]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()

        def ev(obj):
            self.wfile.write(("event: " + obj["type"] + "\ndata: " + json.dumps(obj) + "\n\n").encode())

        ev({"type": "response.created", "response": {"id": rid}})
        for it in items:
            ev({"type": "response.output_item.added", "item": it})
            ev({"type": "response.output_item.done", "item": it})
        ev({"type": "response.completed", "response": {"id": rid, "usage": {
            "input_tokens": 0, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 0,
            "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 0}}})
        self.wfile.flush()


if __name__ == "__main__":
    class Exclusive(ThreadingHTTPServer):
        allow_reuse_address = False  # never share the port with another listener

        def server_bind(self):
            import socket
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            super().server_bind()

    Exclusive(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
