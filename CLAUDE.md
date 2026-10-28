# CLAUDE.md — coding-supervisor

Live-truth document for this repository. Current state and binding rules only. Volatile claims carry `as_of YYYY-MM-DD`. When a change falsifies a line here, updating it is part of the change.

## What this is

**A portable coding-agent supervisor that coordinates delegation and requires verifiable, current evidence before accepting task completion.**

It sits inside or alongside an existing coding harness (Claude Code, Codex CLI, others). The harness supplies the model, the tools and the execution environment. The supervisor supplies three connected responsibilities:

| Responsibility | What it does |
|---|---|
| Control completion | Track the required checks and their evidence. Accept completed work, request a bounded repair, or report a real blocker. |
| Coordinate work | Decide, within host-owned policy, whether a task stays in the main agent or goes to an available specialist. **Advisory only** in the first slices. |
| Measure outcomes | Record what actually happened and compare correctness, time, tokens, retries and cost per accepted task. |

## Scope of the first deployable slice (as_of 2026-10-28)

A **deterministic completion gate**. Records: task contract, candidate identity, verification receipt, finding, checkpoint. Rules: freshness invalidation when the candidate changes, bounded repair rounds, explicit terminal states (completed, blocked, budget exhausted, failed, cancelled). **No model inside the core.** Tested with fake providers before any live harness.

Out of scope for the first slice: a standalone loop engine, agent roster management as a goal in itself, routing changes, any MCP surface, any UI.

## Implementation principles

1. **Evidence before acceptance.** A completion claim without a receipt that names the command, the working directory, the candidate identity, the exit code and the claim it supports is not accepted. "Would run" and "should run" count as not run.
2. **Freshness.** A check binds to one candidate identity (commit plus dirty-tree digest). A changed candidate invalidates every affected check. Conservative invalidation first; dependency-aware reuse later, if ever.
3. **Bounded repair.** A configured cap on repair rounds. Reaching it produces an explicit unresolved outcome, never a pass.
4. **Preserve the inherited contracts.** From the Duet specification: checks C1 baseline reproduces the reported failure, C2 candidate passes the reported test, C3 configured regression suite passes, C4 diff within contract scope; scenarios A18 (self-reported checks cannot mark ready), A19 (candidate changed after a check makes that check stale), A20 (resume keeps contract, budget, pending effects and candidate identity).
5. **Adapters declare their mode on their first line.** `enforcing`: blocks a defined acceptance transition when evidence is insufficient. `advisory/audit`: records, evaluates and warns without blocking. A claim of enforcement is scoped to the paths it has been proven to cover.
6. **Delegation is advisory until it earns more.** The existing Python Jev engine at `C:\Users\rakovnik\.agents\jev` may be reused as the recommender. No second router, no SDK, no replacement adapter.
7. **Measurement uses the per-accepted-task metric** with failed attempts in the numerator and the acceptance rate beside it. Single-repetition flips are never attributed to architecture; four repetitions or a cross-variant table first.
8. **Smallest change that fully works.** No abstraction, option or configuration nobody asked for. Shortcuts carry a `shortcut:` comment stating the limit and when to upgrade.
9. **Secrets never appear in chat, logs, receipts or git.** Receipts carry identifiers, never prompt bodies or credentials.

## Current truth (as_of 2026-10-28)

- Repository state: preparation only. No product code exists yet. First implementation plan: `docs/research/2026-10-28-completion-gate-first-slice/PLAN.md`.
- The proving ground is the research and measurement repository at `C:\Users\rakovnik\.claude\robert` (git, branch main, commit `7829ebe`). It is not moved and not duplicated here. `HANDOVER.md` cites its authoritative files by path and hash.
- Claude Code enforcement is **unproven**. Whether the Claude Code adapter can be `enforcing` or must start `advisory/audit` is the first investigation in the plan.
- Jev routing: live in advisory mode on this machine; persona abstention rate 74 percent over 4,375 receipts; the cause is **unmeasured**. A roster correction (epoch 2) is deferred until the running brief-after cohort is measured.
- Known failing test in the Jev engine suite: `test_harnesses.py:349`, a stale pinned skill description unrelated to this project. The engine's full suite has not been run.

## Environment notes

- Windows host, Linux-style line endings enforced by `.gitattributes`. Python 3.14 at `python`; pytest is not installed for it, use `unittest`. PyYAML lives in the user site, so `python -I` cannot import it.
- Long-running commands go through Terminal Commander (`run_and_watch` for short runs, `command_start_combed` plus `bucket_wait` for long ones) and are cited by exit code and matched lines. Never pipe to tail, never redirect and grep, never sleep-poll.
- Codex worker lessons: dispatch with `background=true` (a foreground call over 180 s is cancelled, not timed out); each Codex turn has a ten-minute cap, so jobs must write results as they go or be split; wait in 150 s slices.
- Memory: decisions and lessons go to EverOS with the five-slot tag (project `TEAM` until a project tag exists); this file and `HANDOVER.md` must stand without it.

## Documentation hygiene

- `CLAUDE.md`: live truth (this file).
- `docs/research/<date-topic>/`: plans, findings, ideation, queued work; committed.
- `docs/solutions/`: compounded learnings once something is solved.
- `docs/archive/`: history; never trusted, never updated.
- `HANDOVER.md`: the self-contained state for the next session; superseded by later handovers, which replace it rather than append.
