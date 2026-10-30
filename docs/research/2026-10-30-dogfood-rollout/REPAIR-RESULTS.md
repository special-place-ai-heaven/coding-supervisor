# Audit repair execution

2026-10-30. Repair implementation record. Original A1–A6 in GOAL.md remain binding. Final acceptance is determined by the candidate-bound verification and independent re-audit artifacts listed below, not by intermediate suite success.

The independent audit is preserved at `C:\Target\coding-supervisor\audit-20261030\AUDIT.md`. This repair uses one native harness goal, with no token/time budget. Local implementation, scoped dogfooding and independent re-audit are authorized; commits, pushes, releases and remote mutations are not. Other seven projects remain inactive.

Inherited candidate: HEAD `8c2b77a779f3f597f3cf834e4572ba8c20f5fb20`, dirty digest `78ca15aea66255234318b91216be314908f8c278ff053ea4b72469c84b5aa216`. All inherited tracked and untracked work is preserved. Original audit hash, tracked patch and 48-file source/binary manifest are retained under `C:\Target\coding-supervisor\repair-20261030`.

Execution order and ownership:

1. A2 / Rust specialist: regressions before code for wrong-checkout evidence and durable reconstructed-completion recovery; actual TC provenance must be inspected, never inferred.
2. A3 / bounded lifecycle worker: archived task-start replay and explicit fallback legacy migration, after A2 verification.
3. A5 / bounded policy worker: actual persistent-shell `line` schema and equivalent advertised routes.
4. A4/A6 / status and documentation: human status, activation guidance and accurate coverage/results.
5. Independent verifier and original independent auditors: final candidate checks, original counterexamples, separate re-audit.

Each repair hypothesis has at most two attempts before new evidence and a changed hypothesis. Full build/test jobs are serialized through Terminal Commander with external `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`. Red/green receipts must retain command, cwd, candidate, exit/trust and job/artifact. Existing-suite success alone is insufficient.

All seven audit findings require fresh evidence and independent review. Original live traces remain historical evidence; they are not relabeled as execution on repaired code. Final re-audit is separate from the original audit.

## A2 repair cycle

Regression-first evidence: wrong-checkout test job `job_01a18c25b7e0770aba7b3077e131aa45`, reconstructed pending-completion test job `job_01a18c269b8c75e49897d4e13e9d9606`, both observed exit 101 before production edits. The adapter now binds execution to the canonical physical worktree and requires durable matching dispatch/job identity for reconstructed completion. Recovery retains `tc-reconstructed` provenance instead of relabeling it live observed. Masked pipelines and missing durable process-completion evidence remain rejected.

Affected adapter suites: `cargo test -p supervisor-claude-code`, job `job_01a18c28e96071d69d1d3a9c434f55ee`, observed exit 0. Actual isolated TC-daemon restart fixture: `python C:\Target\coding-supervisor\repair-20261030\a2_live_recovery.py`, job `job_01a18c2a685773e7bd6521e4bf2fd902`, observed exit 0. The real inner command `job_01a18c2a6d67714a9a783ca5b4bdeb41` completed; supervisor had only pending dispatch/job records until after the dedicated daemon restarted. The durable receipt, reconstructed status and accepted recovered receipt are retained in `a2-live-c2c875d3ba`. This uses actual command execution and daemon restart with scripted hook delivery, not a live model. The first fixture reader incorrectly expected a nested checkpoint; its observed exit 1 is retained, and only the reader was corrected before retry. The active TC daemon was untouched.

Independent preliminary source review found no A2 source blocker, but identified a vacuous pipeline fixture whose command was not approved by its contract. The test now asserts that an approved pipeline actually dispatches and binds a job before both observed and reconstructed completion are rejected. All 17 observation tests pass (`job_01a18c302f7f70b590de78a16036eb4c`, observed exit 0); production code was unchanged by this test correction.

## A3 repair cycle

Task-start replay now consults durable task metadata across active and archived history, without generating a second task or changing the active pointer. Same-root cwd fallback legacy state is detected and requires explicit migration. The migration source remains intact.

F3 red: `job_01a18c2b8543727eb922e41b57fded84`, observed exit 101. F4 corrected behavioral red: `job_01a18c2d7b8c70bb97f9ce984e037eed`, observed exit 101. Earlier F4 attempts included a Windows path-case test-setup mistake; those are retained and excluded as behavioral proof. Detailed per-command identities and outcomes are in `A3-RED.json` and `A3-GREEN.json` under the repair evidence directory.

Final formatted A3 checks, all observed exit 0: fmt `job_01a18c3041437691ac4cb690d5b8e4ca`; adapter lifecycle (10 tests) `job_01a18c304833736492e14f5a68d97712`; MCP replay (17 tests) `job_01a18c30522d76098568715e24d1db40`; CLI lifecycle (2 tests) `job_01a18c306ddc75f38218e326da6181ce`. These checks released A5; the final independent state review is recorded separately below.

Final-candidate verification and independent re-audit govern acceptance; intermediate checks do not establish it.

## A5 repair cycle and additional review

Real `line` state-guard and policy regressions failed before production changes. A distinct unknown-cwd route regression also failed (`job_01a18c33d2e8759ebf57cdb5404c37c7`, observed exit 101). The repair reads the real fields and refuses configured relative deletion/current-branch rewrite when daemon-owned cwd is unknown, while leaving routine commands silent. Policy/protection suites passed (`job_01a18c377c217498b8b3a66e17cdb52c`, observed exit 0).

Independent review then found generic command-field priority could inspect an extra `argv` instead of authoritative `line` or `shell_line`. The new regression failed (`job_01a18c39c30a7665a0d4df667fd0f7f2`, observed exit 101) before route-specific parsing was implemented. Policy/protection suites subsequently passed (`job_01a18c3ad43270efb5cb6a8b0923ca2a`, observed exit 0; 15 policy and 20 protection tests). Test compilation mistakes and a symbol-edit syntax error were repaired and are not counted as behavioral evidence.

The same review identified an analogous A2 observation-parser issue: TC's granular `shell_exec` ignores extra argv but executes shell_line, while supervisor could record the extra argv as the command identity. Its fresh-dispatch regression failed (`job_01a18c3f94e27655aedf6be0e7089878`, observed exit 101), then tool-specific identity parsing repaired it. Additional failing regressions covered persisted legacy, unmarked shell and marked malformed identities before schema-2 provenance validation was added. Old records stay intact but cannot authorize completion. Full adapter suites passed (`job_01a18c454fac71c5a855228681e619e1`, observed exit 0), with identical before/after candidate. Exact red/green pairs and snapshots are in `A2-SHADOW-RECEIPTS.json` and `A2-IDENTITY-SCHEMA-RECEIPTS.json`.

## A4/A6 implementation and final evidence

Human status now includes effective gate state and named completion paths. JSON and human output distinguish live observed, reconstructed, asserted and unverified receipt provenance. The chair template no longer implies that writing a contract activates a task. Regressions failed before production edits; the reconstructed-count fixture was updated for schema-2 provenance and revalidated. CLI tests use the existing workspace core as a dev-dependency only; no new external dependency was added.

Affected A4/status/template tests passed (`job_01a18c47211b746e98f2a89121f23255`, observed exit 0); current policy/protection tests also passed (`job_01a18c47069775f8a8ff07f47da51e17`, observed exit 0). The original gate lookup during worker handback reported no active contract, as intended for implementation phases; it was not presented as gate acceptance.

Final evidence is external to avoid a receipt altering its own candidate fingerprint. Directory: `C:\Target\coding-supervisor\repair-20261030`:

- `verification-final.json`: independent serialized fmt, clippy, workspace tests, release build, dependency-tree and diff checks, each with exact argv, cwd, candidate and observed outcome.
- `REAUDIT-STATE.json` / `REAUDIT-STATE.md`: original independent state auditor repeats archived replay and actual CLI fallback migration counterexamples, preserving evidence bytes.
- `REAUDIT-OBSERVATION-POLICY.json` / `REAUDIT-OBSERVATION-POLICY.md`: original independent auditor repeats wrong-checkout and schema-shadow command cases, real policy-field cases, actual isolated TC restart recovery and human/JSON status parity against the final release binary.
- `live-one.jsonl`, `live-two.jsonl`, their summaries, task inspections and source/binary manifest: fresh sequential Claude validation tasks on repaired code; actual model traces remain distinct from scripted negative fixtures.
- `FINAL-RESULTS.json`: final acceptance summary, task archive hashes, original-audit preservation, other-project inactivity and links to final receipts.

Use these artifacts' actual verdicts and exact candidates when judging completion. Any subsequent source edit invalidates affected evidence. No commit, push, published release or remote mutation is part of this repair. The selected project returns inactive after scoped validation, and the other seven remain inactive. Codex evidence demonstrates advisory integration only. Same-user writable hooks remain a limited boundary; persistent shell sessions are unavailable on this Windows TC daemon, so their policy checks are schema-faithful fixtures.

The first independent final attempt is retained as `verification-attempt1.json`:
clippy rejected a blank line after a restored doc comment, and the installer
regression required explicit guidance not to start an already-active task again.
Both were repaired narrowly; the installer test was not weakened. The full final
verification is rerun after these changes. A SymForge secret-detector refusal on
the installer test source was respected; its existing failure output supplied the
exact assertion without reading the withheld file through another route.
