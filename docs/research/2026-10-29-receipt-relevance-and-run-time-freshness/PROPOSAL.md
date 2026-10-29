# Proposal: bind receipts to the contract's command and to the candidate at run time

Prepared 2026-10-29 from a read of the repository at commit `9cff50c` (core `src/lib.rs`, adapter `adapters/claude-code/src/{adapter,candidate,claim,guard,header}.rs`, fixture settings and contract, README, CLAUDE.md, HANDOVER.md). It is a proposal for consideration, not a decision. Nothing in the live harness, the Jev policy, the agent briefs or the running cohort is touched by it.

## Summary

The gate today proves two things about a worker's `CHECK` line: that the harness observed a command with that text and exit code run by that agent, and that the gate's state has not been hand-edited. It does not prove a third thing that the acceptance rule depends on: that the observed command is the command the contract meant by that criterion. And it binds the resulting receipt to the candidate as it stands at handback, not as it stood when the command ran. Both gaps let a sloppy or self-deceiving worker produce a linked, current, accepted receipt for a check that never exercised the candidate. Closing them is a contract-format change plus two small adapter changes. The first should land before the gate meets a real project, because it changes the contract file and the journal; the second can follow the first cohort.

## Gap 1: the receipt proves a command ran, not that the right command ran

### What the code does

`Criterion` in `src/lib.rs` is `{ id, check, mandatory }`. `check` is a name such as `candidate_passes_reported_test`; nothing in the contract says which command satisfies it. At `SubagentStop`, `adapter.rs` links each cited `CHECK <id> | <command> | exit <code>` to a journaled run by three equalities: same `agent_id`, `run.command.trim() == cited.command.trim()`, same exit code. If a run matches, the receipt is recorded with `claim_supported: true` and the core's `try_accept` is satisfied for that criterion.

### Why that is not enough

The worker chooses the command text. The gate only confirms the text it chose was executed. Three examples, all of which produce a linked and accepted receipt for C2 under the fixture contract:

- The worker runs `cargo test || true`, sees exit 0, cites `CHECK C2 | cargo test || true | exit 0`.
- The worker runs `cargo test -p coding-supervisor --no-run` (compiles, runs nothing), cites it with exit 0.
- The worker runs `echo ok`, cites `CHECK C2 | echo ok | exit 0`.

None of these is adversarial in the sense the README excludes. The first is a habit many agents have when a command is flaky; the second is a worker that misread the contract; the third is the self-deception the gate exists to catch. In each case the gate's journal will say the pass was supported, and `measure` will count it as a supported pass. The acceptance rule is "every mandatory check has a current receipt produced by a command the harness observed"; the word "check" silently carries the assumption that the command is the one that checks, and that assumption is currently the model's word.

### Proposed change

Give each criterion a command in the contract. Exact text by default; an explicit list of accepted texts where a project needs more than one.

```json
{
  "id": "fixture-t4",
  "goal": "Prove the candidate file carries the fix",
  "criteria": [
    {
      "id": "C2",
      "check": "candidate_passes_reported_test",
      "mandatory": true,
      "command": "cargo test -p coding-supervisor --test scenarios"
    },
    {
      "id": "C3",
      "check": "regression_suite_passes",
      "mandatory": true,
      "command": ["cargo test --workspace", "cargo test --workspace --release"]
    }
  ],
  "forbidden_effects": [],
  "scope": ["candidate.txt", "captures/"],
  "max_repair_rounds": 2
}
```

Linkage then requires a fourth equality: the cited command, after trimming, equals the criterion's command (or one of its listed commands). A `CHECK` line whose command does not match is recorded with `claim_supported: false` and a new refusal reason, so the worker is told exactly what to run:

```
GATE: handback refused (refusal 1 of 3): C2 cites `cargo test || true` but the contract
requires `cargo test -p coding-supervisor --test scenarios`. Run that command in this
session and cite it as CHECK C2 | cargo test -p coding-supervisor --test scenarios | exit 0.
```

The core should own the rule, not the adapter, because the core is the part that is tested with fake providers and the part other adapters will reuse. `VerificationReceipt` already carries `command`; `Task::record_receipt` can compare it to the criterion's command and return a typed error (`WrongCommand { criterion, cited, required }`) alongside the existing `MissingReceipt`, `StaleReceipt`, `FailingReceipt` and `OutOfScope`. The adapter renders it like the others.

Matching is exact after trimming. Normalising shells, quoting, path separators or environment prefixes is a rabbit hole; a project that needs a variant lists it. The `cwd` the receipt already records stays as it is.

### What this costs

- Contract format gains a required field. Existing contracts without it fail to load with a clear error; there are none outside the fixture, so no migration.
- Journal format v1 gains one refusal reason. `measure` should count it in the refusals-by-reason table; nothing else changes.
- One more scenario in `tests/scenarios.rs` and one more replay test in the adapter, in the style of the lying-worker case.

### What this does not solve

A worker can still run the required command in a way that proves nothing, for example by editing the test it is supposed to pass. That is the job of C1 (baseline reproduces the failure) and C4 (diff within scope), not of this change. The change closes the specific case where the command named in the receipt is not the command the contract asked for.

## Gap 2: the receipt is bound to the candidate at handback, not at run time

### What the code does

`observed_run` journals shell results and Terminal Commander calls as `{ agent_id, command, exit_code, t, reference }` in `runs.jsonl`. The candidate is observed once per hook call, at the top of `process`, by `GitCandidate::observe` (HEAD plus a digest of the dirty tree). At `SubagentStop` every receipt is stamped with that one observation.

### Why that is not enough

The sequence that defeats it is ordinary, not hostile:

1. Worker runs `cargo test --workspace`, exit 0. Candidate at that moment: `abc123 + digest D1`.
2. Worker notices a lint, edits `src/lib.rs`. Candidate is now `abc123 + digest D2`.
3. Worker hands back with `VERDICT: PASS` and `CHECK C3 | cargo test --workspace | exit 0`.

The gate observes the candidate at step 3 (`D2`), links the cited line to the run from step 1 (same agent, same text, same exit), and records a receipt bound to `D2`. The tests never ran against `D2`. Scenario A19 ("candidate changed after a check makes that check stale") is preserved in the core and defeated by the adapter inside a single worker turn, because the adapter never told the core the check happened before the change.

This is the same class of failure the T4 live run already caught across turns (the L4 case where a stale candidate produced `StaleReceipt` after resume). It is simply not caught within a turn.

### Proposed change

Observe the candidate when the run is journaled, and make it part of the run record:

```json
{"agent_id":"a1","command":"cargo test --workspace","exit_code":0,"t":1761732000,
 "reference":"run-0007","candidate":{"base_commit":"abc123","dirty_digest":"D1"}}
```

Linkage at `SubagentStop` then requires `run.candidate == current candidate`, in addition to the three (or four, after Gap 1) existing equalities. A run that matches on everything but candidate is reported with its own reason, so the worker understands that re-running is the fix, not re-citing:

```
GATE: handback refused (refusal 1 of 3): C3 was run against candidate abc123/D1 but the
candidate is now abc123/D2 (changed after the run: src/lib.rs). Run the check again on the
current candidate and cite it.
```

The receipt that is finally recorded carries the candidate from the run, which is by construction the current one. The core's freshness rule then has nothing to do at handback, which is the point: the adapter no longer asserts freshness, it observes it.

### Where the observation happens

`PostToolUse` and `PostToolUseFailure` for `Bash`, `PowerShell` and `mcp__terminal_commander__*`, which is exactly where `observed_run` already fires. The cost is one `git rev-parse HEAD`, one `git rev-parse --show-prefix` and one `git status --porcelain -z` per shell call, plus a read of each dirty file. On a repository with a large dirty tree that is noticeable; on this one it is milliseconds. If it becomes a problem, hashing only files under the contract's `scope` is the right narrowing, because a change outside scope is already an `OutOfScope` failure at acceptance.

`GitCandidate` currently limits `git status` to the hook's cwd (`-- .`). A worker whose cwd is a subdirectory can edit a file outside it without changing the digest. Observing at run time does not fix that; observing from the repository root does. Worth changing in the same pass, since both touch `candidate.rs`, but it is a separate decision: a monorepo may want the subtree behaviour on purpose.

### What this costs

- `runs.jsonl` gains a field. Old run records without it cannot be linked; since runs are per task and the gate has not supervised real work, none exist that matter.
- One replay test in the adapter: run, edit, hand back, expect refusal; run again, hand back, expect acceptance.
- The existing `restore` grouping logic in `adapter.rs`, which groups stored receipts by candidate and replays them in order, becomes simpler, because receipts now arrive already bound. It need not be rewritten; it just stops being the only place freshness is reconstructed.

## Two smaller observations, not part of this proposal

Recorded here so the next session does not rediscover them. Neither blocks real use.

**`fail_open` on contract read or journal write.** `handle` returns a `systemMessage` and no decision when the state directory cannot be read or written. `PreToolUse` already fails closed when the candidate cannot be observed; the same treatment for an unreadable contract would mean a permissions hiccup on the state directory pauses writes rather than silently removing the gate. The `shortcut:` comment already schedules this after a month of clean journals; that schedule is reasonable.

**`claims_completion` is a vocabulary.** The `Stop` hook refuses only when the final message contains one of eight phrases without a negation in the same clause. A chair that ends with "Here is the PR" or "Pushed to the branch" is never examined. The README already states that no acceptance event exists; this is the practical edge of that statement. The inverse rule is stricter and has no vocabulary: while a contract is loaded and the task is not in a terminal state, block any `Stop` unless the message explicitly says the task is not complete. Whether the extra refusals are worth it is a measurement question for the after-cohort, not something to decide now.

## Suggested order

1. Gap 1 (contract command, core rule, adapter linkage, one refusal reason, tests). Land before step 1 of the README's "what still needs to happen", because it changes the contract format.
2. First real-project use and the after-cohort measurement, as already planned.
3. Gap 2 (run-time candidate on run records, linkage equality, tests), with the root-vs-cwd `git status` decision taken at the same time.
4. Revisit `fail_open` and the `Stop` vocabulary with cohort numbers in hand.

Each step is a single commit touching at most four files, with `cargo test --workspace`, `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` cited by exit code in the results file, as the existing tasks do.
