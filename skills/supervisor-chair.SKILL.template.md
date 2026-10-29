---
name: supervisor-chair
description: >
  Completion discipline for the agent that owns a task ({HARNESS}). Use before
  saying a task is done, complete, verified or ready for review: ask the gate
  first, quote its answer verbatim, and never claim completion the gate has
  refused. Also use when delegating work, so the worker's handback carries the
  header the gate reads.
---

# Chair discipline ({HARNESS})

The completion gate (`supervisor`) keeps the task contract, the current
candidate identity and one receipt per check. It decides whether work is
accepted. You decide what to do about its answer. You never override it.

## Before you say "done"

1. Observe the candidate: call `candidate_observe` with no identity fields.
   The gate reads git itself (the current commit, a digest of the uncommitted
   changes and the changed paths), the same way its hooks do. Pass
   `base_commit`, `dirty_digest` and `changed_paths` only on a harness
   without git. If the files changed since the last receipt, the gate marks
   those receipts stale.
2. Record each check as a receipt: `receipt_record` with the criterion id,
   the exact command, its working directory, its exit code and whether it
   supports the claim. Only commands that ran in this session. A command you
   did not run has no receipt.
3. Ask: `accept_try`. Then `task_complete` when it accepts.
4. Quote the gate's answer verbatim in your report, refusal or acceptance.
   Do not paraphrase a refusal into something softer.

{MCP_NOTE}

## When the gate refuses

The refusal names the reason: a missing mandatory receipt, a stale receipt,
a change outside the contract scope, an open high or critical finding, a
budget that is exhausted, or a tampered state file. Fix the cause, not the
wording. Rerun the check, record the new receipt, ask again. If the cause
cannot be fixed, report the task as blocked with the gate's text; do not
report it as done.

## When you delegate

Tell the worker to use the `supervisor-worker` handback: `VERDICT:`,
`CHECKS RUN:`, one `CHECK <id> | <command> | exit <code>` line per check.
When the handback comes back, record its CHECK lines as receipts only if you
saw those commands run (in the worker's transcript or your own tools).
`CHECKS RUN: 0` with `VERDICT: PASS` is a claim, not evidence.

## Never

- Call `task_complete` after a refusal without a new receipt.
- Record a receipt for a command that did not run ("would pass", "should
  pass", "passed last time").
- Edit the gate's state files by hand. The gate detects it and refuses every
  later call for that task.
- Put prompt text, file contents or secrets into a receipt; receipts carry
  identifiers and commands only.
