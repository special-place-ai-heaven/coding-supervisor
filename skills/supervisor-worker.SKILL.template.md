---
name: supervisor-worker
description: >
  Evidence discipline for delegated work ({HARNESS}). Report actual commands,
  outcomes and job identifiers. The supervisor derives evidence from observed
  runs; users and workers do not need CHECK headers.
---

# Worker evidence ({HARNESS})

Use the task identity, scope and criteria table the chair gave you
(id, command, mandatory). Run the table's command verbatim, character for
character, and report its actual outcome.
You are not alone in the checkout: preserve others' edits and work only within
your assigned ownership. Do not start a replacement task or weaken a contract
to escape a refusal.

Run the required command exactly, with an explicit absolute working directory
and preserved argv boundaries. The command runs alone: nothing appended, piped
or redirected. A started Terminal Commander job is pending,
not passed. Retain its job ID and obtain its authoritative final status.
After restart, recover that job before considering a fresh run. Unknown,
failed, abandoned or unlinked executions are not successful evidence.

Report the change, actual commands and outcomes, job/output identifiers, and
remaining issues in ordinary prose. No fixed CHECK header is required. Under
Claude hooks, SubagentStop derives evidence from observed runs. Under advisory
MCP, the chair may record a receipt only for execution it actually witnessed.
A worker's assertion alone proves nothing.

If a check fails, report the actual exit code and repair only within the
remaining task budget. If the candidate changes after a check, rerun affected
checks. Do not claim a hypothetical, stale or prior-task result as current.
Never alter supervisor state directly, reset repair accounting or replace a
supervising binary with the candidate under test.

Keep prompt text, file contents and secrets out of receipts. Use identifiers
for logs and jobs; never reproduce credentials in commands or reports.
