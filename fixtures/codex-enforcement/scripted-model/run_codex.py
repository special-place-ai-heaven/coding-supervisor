"""Run one fixture test: set the hook policy and the scripted-model scenario, run codex exec, keep its JSONL output
in MOCK_STATE_DIR. Usage: python -I run_codex.py <test> <scenario> "<prompt>" (env T3_SANDBOX overrides the sandbox).
Start scripted-model/mock_model.py 28431 first."""
import json
import os
import shutil
import subprocess
import sys

HERE = os.environ.get("MOCK_STATE_DIR", "C:/temp/codex-t3")
FIXTURE = r"E:\project\coding-supervisor\fixtures\codex-enforcement"
test, scenario, prompt = sys.argv[1], sys.argv[2], sys.argv[3]
extra = sys.argv[4:]

pol_path = os.path.join(FIXTURE, "hooks", "policy.json")
pol = json.load(open(pol_path, encoding="utf-8"))
pol["active"] = test
with open(pol_path, "w", encoding="utf-8", newline="\n") as f:
    f.write("{\n" + ",\n".join(f"  {json.dumps(k)}: {json.dumps(v)}" for k, v in pol.items()) + "\n}\n")
open(os.path.join(HERE, "scenario.txt"), "w").write(scenario)

env = dict(os.environ)
for k in ("CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_CHILD_SESSION",
          "CLAUDE_CODE_SUBAGENT_MODEL", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_PID"):
    env.pop(k, None)
argv = [shutil.which("node"), r"C:\Users\rakovnik\.npm-global\node_modules\@openai\codex\bin\codex.js", "exec", "--cd", FIXTURE, "-m", "gpt-5.6-luna", "--json", "--ephemeral", "--ignore-user-config",
        "--dangerously-bypass-hook-trust", "--enable", "multi_agent", "-s", os.environ.get("T3_SANDBOX", "workspace-write"),
        "-c", 'approval_policy="never"',
        "-c", "projects={ '" + FIXTURE + "' = { trust_level = \"trusted\" } }",
        "-c", 'model_providers={ mock = { name = "mock", base_url = "http://127.0.0.1:28431/v1", wire_api = "responses" } }',
        "-c", 'model_provider="mock"', *extra, prompt]
out = os.path.join(HERE, f"{test}.jsonl")
with open(out, "w", encoding="utf-8") as f:
    rc = subprocess.run(argv, cwd=FIXTURE, env=env, stdout=f, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
for line in open(out, encoding="utf-8"):
    try:
        e = json.loads(line)
    except ValueError:
        print("RAW", line.rstrip()[:300])
        continue
    if e.get("type") in ("thread.started", "turn.completed", "turn.failed", "error") or e.get("type", "").startswith("item."):
        print("EV", json.dumps(e)[:400])
print("EXIT", rc)
