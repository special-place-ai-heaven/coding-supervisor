---
name: supervisor-chair
description: >
  Manage supervisor lifecycle for ordinary user requests ({HARNESS}). Before
  edits, interpret the current request and submit a scoped contract proposal.
  Handle verification, rollover and recovery without asking the user to manage
  contracts or CHECK headers. Never claim completion the gate has refused.
---

# Chair discipline ({HARNESS})

The completion gate (`supervisor`) keeps the task contract, the current
candidate identity and one receipt per check. It decides whether work is
accepted. You decide what to do about its answer. You never override it.

## At each ordinary request

Users give tasks and goals, not supervisor commands. Operate the lifecycle
yourself. Questions and discussion do not activate a task. Ask only about
material ambiguity or missing authorization; never ask users to prepare a
contract, choose internal IDs or write CHECK headers.

With Claude hooks, use the prompt-bound event, nonce and digest supplied by
UserPromptSubmit. Without hooks, use `intent_record` with a stable session ID,
unique prompt ID and the current user's request; this is explicitly
caller-asserted advisory intent. Do not claim that Codex observes user intent
through an enforcing hook. Never put secrets in a proposed contract.

Call `intent_apply` before edits with that event's identity and the appropriate
action: `discussion`, `start`, `continue`, `revise`, `pause`, `resume` or
`cancel`. For `start` or `revise`, propose a contract with concrete goal,
relative path scope, mandatory criteria and exact verification commands. Keep
the repository verification floor returned by the gate. The model proposes;
the deterministic validator decides whether the proposal is permitted.

Use `continue` for a follow-up to unfinished work. Use `start` for a new task
after the previous task completed; the gate archives and rolls over. Never
discard unfinished work to start a replacement. Pause and cancellation require
the user's intent. Revisions retain prior requirements and history and
invalidate evidence. A refused revision is not permission to create a weaker
task through another interface. Preserve task/session identity and pending
jobs across restart; do not rerun a pending job merely because a tool call
or supervisor process restarted.

After restart, read `task_state.pending_jobs` and query those exact job IDs.
Retain their original dispatch/candidate/session linkage. A scope revision can
leave a recoverable job invalidated; its result does not satisfy the revised
contract. A new session needs an explicit user resume before taking ownership;
an unrelated sibling-session prompt must not seize unfinished work.

## Verification and completion

Installation does not activate a task. Ordinary conversation needs no contract
or test gate. Use `supervisor status --json` to distinguish registration,
observed connection and active task; a connection alone does not prove that a
model session loaded the integration. MCP remains advisory on every harness.

`task_state` returns the contract's criteria (id, command, mandatory).
Installing hooks does not create or activate a task. A validated proposal for
an actionable request activates it; do not infer an active task from a draft
contract file in the gate's state directory.
When `task_state` identifies an active task, do not call `task_start` again;
resume that task or use the supported explicit rollover command.

Keep the returned generated `task_id` and send it on MCP mutations. Never
substitute the contract's caller-owned id. Explicit `supervisor task start`,
`task resume`, `task next`, `task deactivate` and `task rebind` remain recovery
interfaces for pre-existing manual tasks, not steps to assign to the user.
An unfinished task needs an explicit disposition; do not delete its history.

1. Run each criterion's command exactly as the contract names it, on its
   own: nothing appended, piped or redirected (no `; echo $?`, no `| tail`,
   no `> log`). Any other text does not link to the criterion, and a
   pipeline reports the exit code of its last stage, not of the check.
2. With hooks: report normally. Stop/SubagentStop derive receipts from
   observed executions; no CHECK header is required. The hooks ignore
   caller-asserted MCP receipts. A claimed success never certifies execution.
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

For Terminal Commander commands, supply an explicit absolute `cwd` and
preserve argv boundaries. Keep the returned job ID and obtain its final
authoritative status; an asynchronous start is not a successful check.
The hooks bind runs to the task and candidate at dispatch. A changed candidate
requires a new run. Lost or unresolved degraded jobs, masked pipelines and
reconstructed statuses without original trusted completion evidence do not
satisfy a check. Polling or a supervisor process restart does not require
rerunning a still-running job.

{MCP_NOTE}

## When the gate refuses

The refusal names the reason: a missing mandatory receipt, a stale receipt,
a change outside the contract scope, an open high or critical finding, a
budget that is exhausted, or a tampered state file. Fix the cause, not the
wording. Repair within the remaining budget, rerun the check and ask again. If the cause
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

Tell the worker to use `supervisor-worker`: report actual commands, outcomes
and job identifiers without a special header. With hooks, observed runs supply
the evidence. Without hooks, record receipts only when you saw the commands
run in the worker transcript or your own tools. A worker's prose is not proof.

## Never

- Call `task_complete` after a refusal without a new receipt.
- Record a receipt for a command that did not run ("would pass", "should
  pass", "passed last time").
- Edit the gate's state files by hand. The gate detects it and refuses every
  later call for that task.
- Put prompt text, file contents or secrets into a receipt; receipts carry
  identifiers and commands only.
