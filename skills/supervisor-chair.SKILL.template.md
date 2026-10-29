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

`task_state` returns the contract's criteria (id, command, mandatory). On a
harness where the gate's hooks are installed (`supervisor install` writes
them for Claude Code only), the task exists as soon as the contract is in the
gate's state directory: do not call `task_start`. On a harness without hooks,
call `task_start` with the contract once, when `task_state` finds none.

1. Run each criterion's command exactly as the contract names it, on its
   own: nothing appended, piped or redirected (no `; echo $?`, no `| tail`,
   no `> log`). Any other text does not link to the criterion, and a
   pipeline reports the exit code of its last stage, not of the check.
2. With hooks: put the CHECK header at the top of your final message, the
   same header a worker uses: `VERDICT: PASS`, `CHECKS RUN: <n>`, then one
   `CHECK <id> | <command> | exit <code>` line per command you ran. The gate
   links each line to the run its hooks saw. Do this instead of MCP
   receipts, not in addition: the hooks ignore MCP receipts when they decide
   whether you may stop.
3. Without hooks, in this order and with no command run in between:
   `candidate_observe` with no identity fields (the gate reads git itself;
   pass `base_commit`, `dirty_digest` and `changed_paths` only on a harness
   without git), then one `receipt_record` per check, then `accept_try`,
   then `task_complete` when it accepts. `receipt_record` takes six fields:
   `criterion` (the id), `command` (the contract's command, verbatim), `cwd`
   (where it ran), `exit_code`, `output_ref` (an identifier for the run's
   output, such as a log path or tool call id) and `claim_supported` (true
   only when the run supports the criterion). Only commands that ran in this
   session; a command you did not run has no receipt.
4. Quote the gate's answer verbatim in your report, refusal or acceptance.
   Do not paraphrase a refusal into something softer.

{MCP_NOTE}

## When the gate refuses

The refusal names the reason: a missing mandatory receipt, a stale receipt,
a change outside the contract scope, an open high or critical finding, a
budget that is exhausted, or a tampered state file. Fix the cause, not the
wording. Rerun the check, cite or record it again, ask again. If the cause
cannot be fixed, report the task as blocked with the gate's text; do not
report it as done.

## When you delegate

Hand the worker the contract's criteria as a short table, one row per
criterion: id, command, mandatory. Take them from `task_state`. A criterion
with a list of commands gets all of them in its row. The worker runs and
cites a command from this table verbatim and alone, with nothing appended,
piped or redirected; the gate refuses any other text.

```text
| id | command                                   | mandatory |
| C2 | cargo test -p my-crate                    | yes       |
| C3 | cargo clippy --all-targets -- -D warnings | yes       |
```

Tell the worker to use the `supervisor-worker` handback: `VERDICT:`,
`CHECKS RUN:`, one `CHECK <id> | <command> | exit <code>` line per check.
With hooks, the gate reads the worker's header itself when the worker stops.
Without hooks, record its CHECK lines as receipts only if you saw those
commands run (in the worker's transcript or your own tools).
`CHECKS RUN: 0` with `VERDICT: PASS` is a claim, not evidence.

## Never

- Call `task_complete` after a refusal without a new receipt.
- Record a receipt for a command that did not run ("would pass", "should
  pass", "passed last time").
- Edit the gate's state files by hand. The gate detects it and refuses every
  later call for that task.
- Put prompt text, file contents or secrets into a receipt; receipts carry
  identifiers and commands only.
