# Autonomous workflow evidence index

Scope: the owner's 2026-10-30 quota amendment permits completion after
implementation and independent non-Claude verification. Live Claude acceptance
is deferred, not proven by replay. See [the effective goal](AUTONOMY-GOAL.md)
and [deferred rerun instructions](AUTONOMY-DEFERRED-CLAUDE.md).

This source document is written before final verification so documenting a result
does not change the candidate it describes. The final verdict, exact source and
binary identities, command receipts and independent review records are external:

- `C:\Target\coding-supervisor\autonomy-20261030\FINAL-EVIDENCE.json`
- `C:\Target\coding-supervisor\autonomy-20261030\SUBPROCESS-RESULTS.json`
- `C:\Target\coding-supervisor\autonomy-20261030\verify_acceptance.py`

Absent, failed or stale evidence is not a pass. Preliminary review files retain
the findings that drove repairs; final review records in FINAL-EVIDENCE identify
their replacement reports. Run the external verifier from this repository to
check the supplied evidence against the current candidate.

| Requirement | Implementation and evidence |
|---|---|
| Ordinary intent and scoped contract | `autonomy.rs` registers prompt-bound proposals, validates scope and preserves a captured verification floor; lifecycle/MCP tests cover discussion, source authority and monotonic revisions. Subprocess fixture supplies scripted interpretations of ordinary prompts, not model behavior. |
| Observed execution and freshness | Headerless adapter maps observed runs to criteria; existing observation suites and new completion tests cover stale, wrong-worktree, cross-task, fabricated, failed and unknown evidence. External fixture executes real checks and delivers hook events deterministically. |
| Refusal and bounded repair | Completion tests cover missing checks, newest failed/unknown run, repair exhaustion and pause. External subprocess fixture retains actual failed check, repair and successful recheck receipts. |
| Consecutive tasks and controls | Lifecycle tests cover follow-ups, immutable archives, duplicate rollover, cancellation, discussion, pause/resume and scope revision. External fixture uses one simulated session for two tasks. Live Claude equivalent remains deferred. |
| Restart and concurrency | OS-locked transactions, durable intent replay and pending-job recovery preserve identities. New-owner recovery retains original job provenance and rejects obsolete-session mutations; the ownership race has a deterministic regression. External isolated TC recovery is recorded separately from simulated status payloads. |
| No quiet weakening | Contract revisions preserve requirements; captured npm entry-point hashes reject script replacement. MCP assertions cannot accept hook-owned autonomous tasks, and rollover requires observed evidence. These checks do not attest arbitrary transitive test semantics or create a same-user security boundary. |
| Independent verification | Required test_runner, code-reviewer and security_reviewer reports plus final fmt, clippy, workspace tests and release build must all match the delivered source/binary in FINAL-EVIDENCE. |
| Preservation and release limits | Verifier checks inherited draft/config hashes and known-good installed binary. Seven sibling projects remain inactive. No commit, push, release or new enforcing harness adapter is part of this work. |

The candidate installer registers all tool-event routes; this does not broaden
the supported action-policy categories. Codex MCP remains advisory. Installed
0.1.1 continues supervising development; candidate binaries and fixture
installations are separate. An installed-path or tool-list check is not proof
that a live model loaded hooks.

Focused entry-point/race repair evidence before final formatting:
`job_01a18c96f70675dcafb6a645f6267931` and
`job_01a18c986e10733fa6189c4c4cb7ca8e` failed before fixes;
`job_01a18c9998227483ad4db0b8a756f424` passed all 37 focused tests afterward.
These intermediate results do not substitute for the final candidate checks.
