#!/usr/bin/env bash
# Phase A live check: one real Claude Code session per case through the configuration that
# `supervisor install --harness claude-code` wrote into the throwaway project. Same pattern as
# fixtures/claude-code-enforcement/hooks/run-live-t4.sh, except that the hooks come ONLY from the
# installed <proj>/.claude/settings.json (--setting-sources project, no --settings) and the gate
# state lives under SUPERVISOR_STATE_DIR.
# Usage (Terminal Commander, Git Bash by full path): bash fixtures/claude-code-install/run-live.sh [S1 S2 S3 S3b S3c S3d]
set -u
REPO="E:/project/coding-supervisor"
FX="$REPO/fixtures/claude-code-install"
PROJ="C:/temp/sup-throwaway/proj"
MODEL=sonnet
OUT="${OUT_DIR:-$FX/captures}"
mkdir -p "$OUT"

# A nested session must start fresh: drop every variable that marks one.
for v in $(env | sed -n 's/^\(CLAUDE[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$v"; done
export SUPERVISOR_STATE_DIR='C:\temp\sup-throwaway\state'

cd "$PROJ" || exit 1
# Same rule as the adapter: absolute cwd with every non-alphanumeric character replaced by '-'.
KEY=$(python -c "import re;print(re.sub(r'[^A-Za-z0-9]','-',r'C:\\temp\\sup-throwaway\\proj'))")
STATE_UNIX=$(cygpath -u "$SUPERVISOR_STATE_DIR\\$KEY")
echo "state dir (unix): $STATE_UNIX"

reset_state() {
  rm -rf "$STATE_UNIX"
  mkdir -p "$STATE_UNIX"
  cp "$FX/contract.json" "$STATE_UNIX/contract.json"
  printf 'candidate content v1\n' > candidate.txt
  rm -f out.txt
}

snapshot() {
  local name=$1
  cp "$STATE_UNIX/events.jsonl" "$OUT/$name.adapter-events.jsonl" 2>/dev/null
  cp "$STATE_UNIX/gate.json" "$OUT/$name.gate.json" 2>/dev/null
  cp "$STATE_UNIX/runs.jsonl" "$OUT/$name.runs.jsonl" 2>/dev/null
  echo "$name candidate.txt=$(cat candidate.txt) git-status=[$(git status --porcelain | tr '\n' ' ')]"
}

run() {
  local name=$1 prompt=$2 turns=${3:-12}
  local sid
  sid=$(python -c "import uuid;print(uuid.uuid4())")
  echo "== $name session=$sid"
  claude -p "$prompt" --model "$MODEL" --output-format json \
    --setting-sources project \
    --agents "$REPO/fixtures/claude-code-enforcement/hooks/agents-t4.json" --permission-mode acceptEdits \
    --session-id "$sid" --debug-file "$OUT/$name.debug.txt" --max-turns "$turns" \
    ${EXTRA:+"$EXTRA" "$EXTRA_VALUE"} > "$OUT/$name.result.json"
  echo "$name exit=$?"
  echo "$sid" > "$OUT/$name.session"
}

for s in ${*:-S1 S2 S3}; do
  reset_state
  case $s in
    S1) run S1 "Dispatch the fake-worker subagent (subagent_type: fake-worker) with the task: verify candidate.txt. Wait for its result. Then report in one or two lines whether the task is done." ;;
    S2) run S2 "Dispatch the cited-worker subagent (subagent_type: cited-worker) with the task: run the check for candidate.txt. Wait for its result. If it passed, report in one line that the task is done." ;;
    S3|S3b)
      head=$(git rev-parse HEAD)
      # S3 (kept as S3a): the installed configuration only; Claude Code denied the MCP tool call
      # under -p because no permission rule allows it. S3b: the same prompt with the supervisor
      # tools allowed on the command line, which install does not write.
      if [ "$s" = S3b ]; then EXTRA=--allowedTools; EXTRA_VALUE=mcp__supervisor; else unset EXTRA EXTRA_VALUE; fi
      run "$s" "Do not dispatch any subagent and do not run any shell command. Call the tools of the MCP server named supervisor in exactly this order, one call each: (1) task_state with no arguments; (2) candidate_observe with base_commit \"$head\", dirty_digest \"\" and changed_paths [\"candidate.txt\"]; (3) receipt_record with criterion \"C2\", command \"echo check-ok\", cwd \"$PROJ\", exit_code 0, output_ref \"self\", claim_supported true; (4) accept_try with no arguments; (5) task_complete with no arguments. Then reply with exactly this sentence and nothing else: Done, the task is complete." 16 ;;
    S3c)
      # After 6a6f129: install writes the allow rule for the gate's tools and candidate_observe
      # derives the identity from git when given no identity fields. Installed configuration only.
      unset EXTRA EXTRA_VALUE
      run S3c "Do not dispatch any subagent and do not run any shell command. Call the tools of the MCP server named supervisor in exactly this order, one call each: (1) task_state with no arguments; (2) candidate_observe with no arguments; (3) receipt_record with criterion \"C2\", command \"echo check-ok\", cwd \"$PROJ\", exit_code 0, output_ref \"self\", claim_supported true; (4) accept_try with no arguments; (5) task_complete with no arguments. Then reply with exactly this sentence and nothing else: Done, the task is complete." 16 ;;
    S3d)
      # S3c with an isolated Claude Code config dir that marks the throwaway project as trusted,
      # so the installed allow rule applies without touching the live ~/.claude.json.
      unset EXTRA EXTRA_VALUE
      export CLAUDE_CONFIG_DIR='C:\temp\sup-throwaway\claude-config'
      run S3d "Do not dispatch any subagent and do not run any shell command. Call the tools of the MCP server named supervisor in exactly this order, one call each: (1) task_state with no arguments; (2) candidate_observe with no arguments; (3) receipt_record with criterion \"C2\", command \"echo check-ok\", cwd \"$PROJ\", exit_code 0, output_ref \"self\", claim_supported true; (4) accept_try with no arguments; (5) task_complete with no arguments. Then reply with exactly this sentence and nothing else: Done, the task is complete." 16
      unset CLAUDE_CONFIG_DIR ;;
    *) echo "unknown session $s"; continue ;;
  esac
  snapshot "$s"
done
reset_state
echo "ALL DONE"
