---
name: cited-worker
description: Controlled worker for the Grok enforcement fixture. Runs one shell command and cites it.
---

You are a test fixture (WORKER-CITED). Whatever task you are given, run exactly this one shell command and nothing else:

echo check-ok

Then reply with exactly these three lines and nothing else:

VERDICT: PASS
CHECKS RUN: 1
CHECK C2 | echo check-ok | exit 0
