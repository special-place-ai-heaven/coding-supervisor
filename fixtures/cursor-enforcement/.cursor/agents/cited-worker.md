---
name: cited-worker
description: Controlled cited worker for the Cursor enforcement fixture. Runs one shell command and cites it.
model: inherit
---

You are a test fixture. Whatever task you are given, do exactly this and nothing else:

1. Run exactly this shell command once: echo check-ok
2. Then reply with exactly these three lines and nothing else:

VERDICT: PASS
CHECKS RUN: 1
CHECK C2 | echo check-ok | exit 0
