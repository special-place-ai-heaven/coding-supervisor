#!/bin/bash
# Runs one headless Grok session in this fixture with every home-level source isolated.
# usage: run-scenario.sh <new-session-uuid> <prompt>
# Before the first run: start fake-model/server.py 18811, copy fake-model/grok-home-config.toml to
# $GROK_HOME/config.toml, and run `grok --trust inspect` once with the same environment
# (it writes $GROK_HOME/trusted_folders.toml, which is why GROK_HOME must not be the real ~/.grok).
export GROK_HOME="${GROK_HOME:-C:\temp\grok-t3\home}"
export USERPROFILE="${FAKE_HOME:-C:\temp\grok-t3\fakehome}" HOME="${FAKE_HOME:-C:\temp\grok-t3\fakehome}"
for v in AGENTS HOOKS MCPS RULES SKILLS; do export GROK_CLAUDE_${v}_ENABLED=false GROK_CURSOR_${v}_ENABLED=false; done
unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_SESSION_ID CLAUDE_CODE_SUBAGENT_MODEL CLAUDE_PROJECT_DIR
cd "$(dirname "$0")" || exit 9
sid=$1; prompt=$2; shift 2
# The debug log is evidence (hook timeouts and stop-hook results are logged there).
grok --no-auto-update -p "$prompt" --output-format json -m fake-chair --always-approve --max-turns 12 -s "$sid" \
  --debug-file "C:\temp\grok-t3\dbg-$sid.txt" "$@"
echo "GROK_EXIT=$?"
