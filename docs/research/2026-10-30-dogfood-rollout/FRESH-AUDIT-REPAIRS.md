# Fresh acceptance audit repairs

As of 2026-10-30, uncommitted repair record. The independent fresh audit rejected the prior autonomy candidate despite passing suites. Its original record is preserved at `C:\Target\coding-supervisor\fresh-acceptance-audit-20261030\CONSOLIDATED-AUDIT.md`. Do not interpret earlier acceptance records as covering these three defects.

The authorized repair covers execution-context binding (including per-command environment overrides), atomic prompt supersession and registration, and canonical worktree task discovery across root/subdirectory entry points. Command execution cwd remains separate from task identity. Cross-worktree and external-directory rejection remain required.

Repair evidence is separate: `C:\Target\coding-supervisor\audit-repairs-20261030`. `REPAIR-PLAN.md` records acceptance and ownership; `JOURNAL.md` distinguishes tool/test scaffolding friction from reproduced product defects. Regression failures must precede production changes. Final completion requires fresh reproductions, independent testing/code/security reviews, and current-candidate formatting, clippy, workspace tests and release build receipts.

The known-good installed supervisor supervises development through advisory MCP; candidate binaries are tested separately. No installed binary replacement, sibling-project activation, commits, pushes or releases are authorized.

## Repaired behavior and regression evidence

- Execution evidence now requires schema-3 provenance and an approved execution profile. Unapproved per-command environment/interpreter changes cannot make a skipped check count. The sole per-command environment exception must exactly match the nonempty host-inherited `CARGO_TARGET_DIR`, and only its digest is stored. Old profiles require fresh evidence. Ambient host/daemon settings remain trusted.
- Registration checks token/event ownership before updating event/latest state inside the same transaction. Superseded pending-event IDs remain remembered, so a failed older registration cannot become current through public retry. Historical duplicates do not displace newer pending intent; sessions retain separate ownership.
- Project identity and repository/npm discovery use the canonical physical worktree from root or subdirectory entry points. Actual execution cwd remains separate. Completion and rollover both select root-context evidence; a later nested run cannot displace a valid root check. The regression requires an actually active successor, a preserved predecessor archive, and rejection of unrelated linked-worktree evidence.
- Path normalization does not certify legacy nested discovery. Unknown historical root scope requires fresh evidence. Tests retain an old nested pass receipt while an additional real root Python check exits 23, and require both completion and rollover refusal. Explicit fresh-root cancellation provides archival recovery without legitimizing old receipts, including an already-completed legacy record; ordinary valid completed-task behavior is retained. Raw-root spelling/case compatibility remains covered.

The unit registration fixture's retained-lock setup error was repaired separately before its genuine race RED run. The reviewed one-line synthetic-data change was not an F2 fix. Zero-test runs, compile errors and test-fixture setup failures are excluded from behavioral RED evidence.

External evidence groups:

| Location under `C:\Target\coding-supervisor\audit-repairs-20261030` | Purpose |
|---|---|
| `execution-context/` | F1 RED/GREEN and owned-file hashes. |
| `registration/f2-f3-implementation-20261030/` | F2/F3 and independent-review follow-up regressions, checks and source hashes. |
| `registration/after-fixture-remediation-20261030/` | Preserved historical real-process F2 RED, build linkage and immutable binary snapshot. |
| `security/` | Provisional actual skipped-test reproduction; not final-candidate proof by itself. |
| `FINAL-REPAIR-EVIDENCE.json` | Final source/binary identity, current command receipts, reproductions, independent reviews and preservation checks. |
| `verify_repairs.py` | Fail-closed evidence validation; it does not itself execute product checks or perform reviews. |

Only the actual final record and linked current evidence establish acceptance. Historical/provisional passes do not certify a subsequently changed candidate. The installed advisory task is `task-18e34ff070d51400-c270-1` and its required final commands are fmt, clippy, workspace tests, release build and the external verifier.

## Tooling finding

SymForge's conservative token-like detection may intentionally include synthetic fixtures. The separate failure was an actual content refusal lacking `_meta["symforge/withheld"]`, finding ID and fingerprint required by its documented remediation workflow. Read-only inspection of SymForge 11.10.2's Feature 034 and tests confirmed the end-to-end contract mismatch; loss in the server versus an intermediary remains unlocalized. The local, unpublished report is `C:\Target\coding-supervisor\audit-repairs-20261030\SYMFORGE-REMEDIATION-METADATA-BUG.md`.

The owner explicitly authorized ordinary tools to continue engineering. No detector settings, file exemptions or dismissals were changed; flagged values were not disclosed. This authorization replaces the earlier admission-as-work-blocker restriction and is distinct from the automatic safety-review rejection below.

## Deferred and unverified checks

Live Claude model enforcement remains deferred due to quota. Preserve [the deferred scenarios and rerun instructions](AUTONOMY-DEFERRED-CLAUDE.md). Deterministic hook events and actual local subprocess results must retain their separate classifications and do not establish live enforcement.

Automatic safety review rejected an additional npm root-versus-nested semantic test for possible cybersecurity risk. That test remains unverified, establishing neither a defect nor a pass. It has not been retried through another route. A separate benign Python fixture executes ordinary successful checks in two directories and tests cwd discrimination; it does not establish npm-specific semantic behavior.

The pre-existing handling of TC `state=failed` can leave nonzero jobs pending rather than recording their exit code; completion still refuses. This reporting gap is documented separately and is not claimed repaired here. Final acceptance under the non-Claude scope does not establish live hook enforcement or the rejected npm-specific case.
