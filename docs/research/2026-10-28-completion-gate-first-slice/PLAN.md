# First implementation plan: deterministic completion gate and the Claude Code enforcement question

Date: 2026-10-28. Status: proposed; nothing below is implemented. Bounded to the first deployable slice defined in `CLAUDE.md`. Do not start on the live harness, the Jev policy, the agent briefs or the running measurement cohort.

## Goal

A completion gate that, given a task contract and a stream of events (candidate changed, check ran, finding raised, repair requested, checkpoint, resume), decides deterministically whether a completion claim may be accepted, must be repaired, or must stop. No model inside. Proven with fake providers. Then one honest answer to: can Claude Code be an `enforcing` adapter, and on which paths?

## Tasks

### T1 — Core records and state machine

Implement the five records and the task state machine in one small Python package (standard library only; `dataclasses`, `json`, `hashlib`).

- Records: `TaskContract` (id, goal, criteria list with ids and check names, forbidden effects, budget with max repair rounds), `Candidate` (base commit, dirty-tree digest, provenance), `VerificationReceipt` (criterion id, candidate identity, command, cwd, exit code, output reference, claim supported, timestamps), `Finding` (stable id, candidate identity, severity, location, evidence reference, disposition), `Checkpoint` (task state, candidate identity, open findings, consumed budget, next action).
- States: `queued → preparing → verifying → ready_for_review`; `verifying → repairing → verifying`; terminal `blocked`, `budget_exhausted`, `failed`, `cancelled`, `completed`.
- Rules: a receipt binds to exactly one candidate identity; a candidate change marks every affected receipt stale (conservative: all of them); `ready_for_review` requires a current, exit-0 receipt for every mandatory criterion; the repair cap is enforced before starting a repair, not after; resume loads the checkpoint and re-observes the candidate before any transition.

Acceptance: the scenario tests in T2 pass; the package has no dependency outside the standard library; no record can be created with an unknown field.

### T2 — Scenario tests with fake providers

`unittest` only (pytest is not installed). One test per scenario, each named by its id so the mapping to the inherited contracts is visible.

| Id | Scenario | Required result |
|---|---|---|
| A18 | A worker reports "tests passed" with no receipt | Not accepted; state stays `verifying`; reason recorded |
| A19 | Candidate changes after a passing receipt | Receipt stale; acceptance blocked until re-run |
| A20 | Resume from checkpoint | Contract, consumed budget, open findings and candidate identity restored; no budget reset |
| C1-C4 | Contract with the four Duet checks | Acceptance only when all four have current exit-0 receipts |
| R23 | Repair cap reached | `failed` or `blocked` with unresolved findings listed; never `completed` |
| R26 | Aggregate improves, one mandatory criterion regresses | Acceptance blocked |
| R28 | All receipts pass but an out-of-scope file changed | C4 fails; acceptance blocked |
| R35 | Worker says it would run the check | Treated as not run |
| X1 | Receipt cites a command whose exit code is missing | Receipt rejected as unsupported |
| X2 | Receipt for a different candidate identity | Receipt rejected |

Acceptance: all scenarios pass under `python -m unittest`; the run is recorded with command and exit code.

### T3 — Claude Code enforcement investigation (isolated fixture)

Purpose: establish, with evidence, whether Claude Code offers an interception point at which an unsupported acceptance can be blocked. Work only in a throwaway fixture project under `E:\project\coding-supervisor\fixtures\claude-code-enforcement\` with its own `.claude\settings.json`; never touch the user-level settings or the live briefs.

Distinguish four points and test each separately:

1. Blocking execution: a `PreToolUse` hook refusing a tool call. Known to exist; measure whether it can see enough to judge.
2. Result delivery: a `PostToolUse` hook on the `Agent` tool result. Can it alter or annotate the worker's result before the chair reads it? Can it block? What does it see for asynchronous dispatches, where the earlier analysis found the main transcript holds only launch metadata?
3. Explicit acceptance: is there any event when the chair accepts a worker's claim? If not, acceptance is implicit and only advisory handling is possible there.
4. Unsupported final completion claim: a `Stop` or equivalent hook that can refuse a turn ending with "done" when the gate state is not `completed`.

Tests to run in the fixture, each with a fake worker whose result is controlled:

- Missing receipt: worker returns "VERDICT: PASS" with "CHECKS RUN: 0". Expected under enforcing: the acceptance transition is blocked and the chair sees why.
- Stale candidate: a file changes after the receipt. Expected: the receipt is reported stale.
- Attempted bypass: the chair tries to end the turn claiming completion without the gate's `completed` state. Expected under enforcing: refused.

Acceptance: a written table of the four points with observed/not observed, the hook configuration used, the exact events seen (field names), and for each of the three tests the outcome. If no point can block, the Claude Code adapter is declared `advisory/audit` in its first line and T4 proceeds anyway.

### T4 — Claude Code adapter, first version

Based on T3: either an enforcing hook set or an advisory hook set that records receipts into the gate and warns. The adapter's first line states its mode and the covered paths. The adapter must not change model, effort or brief of any agent.

Acceptance: the T2 scenarios replayed through the adapter in the fixture give the same decisions as the core; the declared mode matches the T3 evidence.

### T5 — Receipt capture coverage statement

For the Claude Code adapter, document which events are observable (prompt submit, pre tool, post tool, stop, subagent files on disk) and which are not. This is the per-harness coverage declaration the product boundary requires.

### T6 — Wire the measurement

Make the gate's receipts readable by the existing measurement scripts in `C:\Users\rakovnik\.claude\robert\harness-measurement\scripts\` or write the one adapter needed, so that cost per accepted task and linked-receipt counts come from the gate's own records rather than transcript heuristics.

## Order and gates

T1 and T2 together, then T3, then T4 and T5, then T6. No step starts on the live harness. Each step ends with its acceptance evidence committed under `docs/research/2026-10-28-completion-gate-first-slice/` as a short results file with commands and exit codes.

## Explicitly not in this plan

Routing changes, the epoch-2 Jev roster correction, an MCP server, a CLI beyond `python -m unittest`, any multi-harness adapter beyond Claude Code, and any claim about verified-work-per-token improvement. That claim is tested by the brief-after cohort in the lab, not by this plan.
