# WP7 live run: a governed session for PR #96's own receipt (2026-11-11)

Outcome: **no receipt was produced.** The owner chose to exempt PR #96 with the
`receipt-exempt` label instead (decision 2026-11-11). This file records what the first live run
of the whole chain showed, so nothing here is claimed beyond what was observed.

## Setup

- Claude Code 2.1.322, headless (`claude -p`), on the owner's work PC (Windows, 20 threads,
  about 7 GB of 31 GB memory free during the runs).
- Worktree at the merge base `fefe3202fa76` with the branch tip's content as uncommitted
  changes, so the contract's `base_sha` is the point CI compares from.
- The seven gate hooks were supplied through a settings file outside the tree
  (`claude --settings <file>`), each calling the branch build of `supervisor hook`. No file
  was added to the work tree.
- Level: the default, `audit`.
- Checks ran through Terminal Commander jobs, which the hooks link to a criterion when the
  job's argv joined with spaces equals the criterion's command and the cwd is the project root.

## What worked

- The hooks fired in a headless session.
- The gate accepted an agent's contract proposal and added the repository's own checks under
  its own ids: `AUTO_FMT`, `AUTO_LINT`, `AUTO_TEST` (`cargo nextest run --workspace`),
  `AUTO_NPM_TEST_NPM`, `AUTO_DIFF`. Only a person can change that list.
- Five criteria hold passing hook-observed receipts on the candidate: the three receipt test
  files, the format check and clippy. `AUTO_TEST` holds a failing observed receipt.
- With `AUTO_TEST` failing, the gate refused each session's completion claim three times and
  then released the stop uncertified. `supervisor verdict` reports `fail`, `mandatory_unmet`.
- A new session took over the open task: its prompt must be exactly "resume", "resume task" or
  "resume this task", and its resume proposal must carry the task id.
- A job cut off by a restart of the Terminal Commander service was reported as abandoned and
  the gate recorded nothing from it.

## What did not

`AUTO_TEST` never passed on this machine. Every failure was a time limit, none an assertion
about behaviour, and the same command passed in CI on Linux and Windows for the same commit.

| Attempt | Parallel tests | Result |
|---|---|---|
| 1 | 20 (default) | five `cli/tests/verdict.rs` tests: git gave up after 20 to 25 s (`git_failed`, `head_unknown`) |
| 2 | 4 | `the_plugin_hook_node_suite_passes`: hook answered in 28.07 s against a 28 s limit |
| 3 | 4 | the same test, 29.8 s |
| 4 | 4 | 216 of 217 run tests passed; one `receipt_verify` test hit a git time limit |
| 5 | 1 | 554 of 701 passed, none failed, when the Terminal Commander service restarted at 69 minutes |

`AUTO_NPM_TEST_NPM` and `AUTO_DIFF` were never reached, so the task never completed and
`supervisor receipt export` was never run against a real task.

Five sessions, 16.87 USD in total.

## Not answered by this run

- Whether a task completes and exports at level `audit`.
- Why `supervisor verdict` reports `effective_level: enforce` while `supervisor status` reports
  `audit` for the same project.
- `supervisor receipt export` and CI verification of a bundle signed with the real machine key.
  The key itself was created and verified on a throwaway task with the 0.18.0 binary.

## Follow-ups

1. The repository's own floor requires the full test suite locally, and on a loaded developer
   machine two fixed time limits make it fail: the 28 s limit in `hooks/supervisor-hook.test.js`
   and the gate's 5 s git deadline as seen by tests that start many git processes. Ease these
   in a small PR; its own PR should carry the first complete live receipt.
2. The CI job re-runs every command in the bundled contract, floor included, which repeats most
   of the Linux test job. Decide whether the receipt job may skip commands the same CI run
   already ran on the same commit.
3. `supervisor receipt export` should run verify's scope check and refuse with the paths named,
   so a file changed before the task started and outside its scope fails at export, not in CI.
4. Two tests for the combination of the dirty-tree baseline (G48) and the receipts folder
   exclusion.
5. From the last review of the CI job: a pass reflects the default branch as it was when the
   job ran; "require branches to be up to date" in branch protection narrows that.
