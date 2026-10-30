# Sequential task operation and recovery

Local development interface, as of 2026-10-30. See RESULTS.md for verification
status and COVERAGE.md for the action-policy boundary. Installed npm binaries
remain at their released version until explicitly replaced or reconfigured.

## Installation and activation

Installing a harness integration does not activate a contract. Check both the
registration and task state:

```text
supervisor status --project <project> --harness claude-code --json
supervisor status --project <project> --harness codex --json
```

The reporting binary and configured binary may differ. A configured process is
not an observed connection, and a direct MCP connection is not proof that a
model session loaded it. Unknown observations remain unknown. Codex is advisory;
it has no enforcing supervisor hook adapter in this rollout.

Prepare the contract outside protected state, with exact commands, scope and
repair policy for the actual task. `init` prepares an editable
`supervisor-contract.json` in the project (or the `--output` path); it does not
start a task. Then activate deliberately:

```text
supervisor task start --project <project> --contract <contract.json>
supervisor task resume --project <project> --task <generated-task-id>
```

Keep the generated `task_id`. It differs from the contract's caller-owned `id`.
MCP mutations must identify the active generated task. Resume preserves repairs,
candidate, receipts and history; it does not reset a failed check.

## Rollover and recovery

After completion, start the next scoped task using the current task identity:

```text
supervisor task next --project <project> --task <old-id> --contract <next.json>
```

An unfinished task requires `--disposition cancelled`, `failed`, `blocked` or
`budget_exhausted`. The previous evidence remains unchanged; archive metadata
records the explicit disposition and successor. Recovery creates a successor
with a recovery link. It never edits a terminal core checkpoint to pretend the
old task continued or passed.

Existing hook sessions remain bound to their prior task. Prefer a fresh session
for a successor. To deliberately reuse one, bind it explicitly:

```text
supervisor task rebind --project <project> --task <active-id> --session <session-id>
```

Rebinding does not turn an earlier task's pending command into evidence for the
new task. Once scoped work is finished, return to inactive conversation:

```text
supervisor task deactivate --project <project> --task <active-id>
```

Deactivating unfinished work also requires an explicit disposition. Do not delete
state directories to escape a refusal. Tampered state requires authentic evidence
recovery; task rollover is not a tamper reset.

## Identity and migration

Each canonical physical worktree has a separate state namespace with a SHA-256
suffix. Windows case and path aliases normalize; linked worktrees remain separate.
`SUPERVISOR_STATE_DIR` selects the containing namespace, not a shared task directory
for unrelated projects. Keep its value consistent across cooperating processes.

Old punctuation-only names can collide. When legacy state exists, explicitly
select the source after checking that it belongs to this project:

```text
supervisor task migrate --project <project> --legacy <old-state-directory>
```

Migration preserves the selected source. An ambiguous legacy directory is never
chosen automatically. Do not overwrite or merge two projects' histories.

## Durable jobs and crash recovery

Use TC with an explicit absolute cwd. Preserve argv boundaries, retain the job ID
and retrieve final status from that same job. A start result, a line in stdout,
or a model's exit-code assertion cannot satisfy a check. The dispatch binds task,
candidate and cwd; edits while a job runs make its receipt stale for acceptance.

A supervisor process restart does not discard pending jobs. Original observed
completion evidence can be reused after restart only with its original linkage.
A reconstructed TC status by itself is insufficient. A pending dispatch may be
completed after a TC restart only when its job, bucket, task, session, physical
worktree, command and candidate linkage were already persisted and the matching
TC durable status proves terminal process observation and cleanup. Pipeline
masking is checked from the original dispatch because reconstructed TC status
does not retain that flag reliably. Recovered evidence is labeled
`tc-reconstructed`, separately from live observations. Missing/lost jobs,
unbound reconstructed results and unresolved degraded observation require
investigation or a fresh run. Pre-upgrade journals without the execution-worktree
binding are preserved as history but cannot authorize completion; rerun their checks.
The same rule applies to command records without authoritative identity schema 2.
The adapter reads the field actually executed by each tool (`argv`, `shell_line`,
or `command`); an extra field cannot substitute another approved command.

Harness-generated locks and queues are not source code. If a separate harness
writes runtime files into the checkout, identify them before starting a task and
exclude only that generated runtime location locally. This dogfood run uses
`.git/info/exclude` for `.claude/autoharness/`; it preserves the files and keeps
their hooks enabled. Do not exclude actual source to make stale checks pass.

OS file locks serialize cooperating hook/MCP processes and release on process
exit. The next transaction replays a durable unfinished write before using state.
If recovery reports corruption or stale journal identity, retain the evidence and
resolve the reported mismatch; never remove the journal merely to get a pass.

These are process-crash recovery mechanisms, not protection against a malicious
same-user process that can rewrite hooks, binaries and their storage.
