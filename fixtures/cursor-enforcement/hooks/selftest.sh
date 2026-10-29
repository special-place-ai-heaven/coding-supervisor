#!/bin/sh
# Offline self-test of hooks/capture.py, fed the way Cursor feeds hooks on Windows:
# the hook command string runs under PowerShell with the JSON payload on stdin.
# Not Cursor evidence: it proves the fixture handler, not the harness. Captures it
# writes (captures/st-*) are deleted at the end. Run from Git Bash in the fixture.
set -u
cd "$(dirname "$0")/.."
H='python E:/project/coding-supervisor/fixtures/cursor-enforcement/hooks/capture.py'
fire() { printf '%s' "$1" > hooks/active-test.txt; printf '%s :: ' "$1"; printf '%s' "$2" | powershell -NoProfile -Command "$H"; echo; }
cp hooks/policy.json hooks/policy.json.bak
python - <<'PY'
import json; p=json.load(open("hooks/policy.json"))
for k in ("deny","stale","missing","bypass"): p["st-"+k]=p[k]
json.dump(p,open("hooks/policy.json","w"),indent=1)
PY
fire st-deny '{"hook_event_name":"preToolUse","tool_name":"Write","tool_input":{"path":"out.txt"}}'
fire st-deny '{"hook_event_name":"beforeShellExecution","command":"echo hi","cwd":"x","sandbox":false}'
fire st-stale '{"hook_event_name":"postToolUse","tool_name":"Shell","tool_input":{"command":"echo check-ok"},"tool_output":"{\"exitCode\":0}"}'
fire st-stale '{"hook_event_name":"subagentStop","status":"completed","loop_count":0,"summary":"VERDICT: PASS\nCHECKS RUN: 1\nCHECK C2 | echo check-ok | exit 0"}'
fire st-stale '{"hook_event_name":"afterAgentResponse","text":"Done, the task is complete."}'
fire st-stale '{"hook_event_name":"stop","status":"completed","loop_count":0}'
cp candidate.txt candidate.txt.bak; printf 'candidate content v2\n' > candidate.txt
fire st-stale '{"hook_event_name":"stop","status":"completed","loop_count":0}'
cp candidate.txt.bak candidate.txt
fire st-bypass '{"hook_event_name":"afterAgentResponse","text":"Done, the task is complete."}'
fire st-bypass '{"hook_event_name":"stop","status":"completed","loop_count":0}'
cp hooks/policy.json.bak hooks/policy.json; rm -f captures/st-* hooks/active-test.txt hooks/policy.json.bak candidate.txt.bak
