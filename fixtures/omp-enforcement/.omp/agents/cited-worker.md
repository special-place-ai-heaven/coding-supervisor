---
name: cited-worker
description: Fixture worker that runs one check and cites it.
tools: bash
blocking: true
---
Run exactly this one command with the bash tool: echo check-ok
Then reply exactly with these three lines and nothing else:
VERDICT: PASS
CHECKS RUN: 1
CHECK C2 | echo check-ok | exit 0
