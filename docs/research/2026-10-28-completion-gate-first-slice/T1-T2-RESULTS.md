# T1 and T2 results: core records, state machine, scenario tests

Date: 2026-10-28. Status: accepted by the chair after independent verification. Scope: PLAN T1 and T2 only; T3 onward not started.

## What exists

| File | Content |
|---|---|
| `Cargo.toml` | package `coding-supervisor` 0.1.0, edition 2024, no dependency section of any kind (normal, dev, build, target-specific) |
| `Cargo.lock` | one `[[package]]` entry, the project itself |
| `src/lib.rs` | records `TaskContract`, `Candidate`, `VerificationReceipt`, `Finding`, `Checkpoint`; `TaskState` with the ten states; the `Task` gate with freshness invalidation, bounded repair, receipt validation and resume; `#![forbid(unsafe_code)]`; 438 lines |
| `tests/scenarios.rs` | `FakeRunner` fake provider, Duet contract helper (C1 to C4 mandatory, scope `src/` and `tests/`, two repair rounds), eleven tests; 243 lines |

All four files are LF (zero carriage-return bytes). No `target/` directory inside the checkout; artifacts under `C:\Target\coding-supervisor\main\target`.

## Executed verification (chair, Terminal Commander, all with `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, cwd `E:\project\coding-supervisor`)

Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, resolved from `rust-toolchain.toml`.

| Command | Exit code | Evidence |
|---|---|---|
| `cargo test` | 0 | `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` (scenarios binary); lib unit tests: 0 |
| `cargo fmt --check` | 0 | no diff lines |
| `cargo clippy --all-targets -- -D warnings` | 0 | `Finished dev profile`, no warning or error lines |
| `cargo tree -e normal,build,dev --target all` | 0 | single line `coding-supervisor v0.1.0 (E:\project\coding-supervisor)`; no other package on any edge kind or target |

The Codex worker reported the same three exit codes and the same test line before handoff; the chair re-ran every command independently.

## Scenario mapping: PLAN T2 rows to tests

The T2 table has ten rows covering thirteen inherited identifiers (the `C1-C4` row names four). Ten tests cover the ten rows; the eleventh test covers a gap found during implementation.

| PLAN row | Inherited ids | Test name | Result | Asserted outcome |
|---|---|---|---|---|
| A18 | A18 | `a18_self_reported_pass_is_not_accepted` | ok | claim logged in `rejected_claims`, `try_accept` fails `MissingReceipt(C1)`, state stays `Verifying` |
| A19 | A19 | `a19_candidate_change_makes_receipt_stale` | ok | receipt status `Stale` after new candidate, `StaleReceipt(C1)`, acceptance only after four fresh receipts |
| A20 | A20 | `a20_resume_restores_contract_budget_findings_candidate` | ok | state, candidate identity, one consumed repair round and the open finding survive resume; `CandidateNotObserved` before re-observation; `MissingReceipt` after it (receipts are not checkpointed) |
| C1-C4 | C1, C2, C3, C4 | `c1_c4_all_four_current_receipts_required` | ok | three receipts give `MissingReceipt(C4)`; four exit-0 receipts give `ReadyForReview`, then `Completed` |
| R23 | R23 | `r23_repair_cap_reached_never_completes` | ok | third repair request refused with `RepairBudgetExhausted { max: 2 }`, state `BudgetExhausted` (the plan's explicit terminal for this case, chosen over the row's `failed`/`blocked` wording), open finding still listed |
| R26 | R26 | `r26_mandatory_regression_blocks_despite_aggregate` | ok | four of five receipts pass including an optional one; one mandatory exit 1 gives `FailingReceipt { C3, 1 }` |
| R28 | R28, C4 | `r28_out_of_scope_change_fails_c4` | ok | all four receipts exit 0, candidate changed `docs/out.md`; `OutOfScope(["docs/out.md"])`; C4 is enforced by the gate from the candidate's changed paths against the contract scope, in addition to the C4 receipt |
| R35 | R35 | `r35_would_run_counts_as_not_run` | ok | "would run" claim logged, `MissingReceipt(C1)` |
| X1 | X1 | `x1_receipt_without_exit_code_rejected` | ok | `ReceiptMissingExitCode` |
| X2 | X2 | `x2_receipt_for_other_candidate_rejected` | ok | `ReceiptForOtherCandidate { expected, got }` |
| (none) | A20 spirit | `resumed_review_requires_fresh_receipts` | ok | see deviation below |

Not covered by any test here: `forbidden_effects` (stored on the contract, not evaluated; no scenario in T2 asks for it), `block`, `fail`, `cancel` transitions, `UnknownCriterion`, `UnknownFinding`, `ReceiptUnsupported` (a receipt with `claim_supported = false`).

## Deviation from the dispatched design

The chair's brief restricted `record_receipt` to `Verifying` and made `complete()` a plain `ReadyForReview -> Completed` transition. That left a hole: a checkpoint taken in `ReadyForReview` carries no receipts, so after resume `complete()` would have succeeded with no evidence. The worker's change: `complete()` re-runs the full acceptance check (fresh exit-0 receipt per mandatory criterion, scope, open findings) and `record_receipt` is also accepted in `ReadyForReview`. The eleventh test proves a resumed review cannot complete until fresh receipts are recorded. Accepted by the chair as consistent with principle 1 (evidence before acceptance) and A20.

## Environment changes made during this session

1. **Named toolchain `1.99.1-x86_64-pc-windows-msvc` installed** at 15:23:43 local time by the other session running on this machine (explicit `rustup toolchain install`, reported by that session). Before it, the chair's first `rustc --version` inside the repository failed with "toolchain is not installed": rustup on this machine did not auto-install the pin from `rust-toolchain.toml`. The HANDOVER claim "pinned by rust-toolchain.toml" was true only after that install.
2. **`C:\Users\rakovnik\.rustup\settings.toml` written** at 15:24:02 by the chair running `rustup set auto-install` with no argument while diagnosing item 1. The file now contains `auto_install = "enable"`. The previous content of that line, and therefore the previous effective setting, was not established before the write. Not reverted; recorded here so the next session knows the file changed.
3. `C:\Target\coding-supervisor\main\target` created and populated by the first build.

Nothing in the live harness, the Jev policy, the agent briefs or the measurement cohort was touched.

## Execution record

- Worker: one Codex session `session-mvs7j3rf-fumluyl7`, background, high reasoning, full write access restricted by brief to the checkout and the target directory, no git writes; about 15 minutes wall time including the cold build.
- Chair: design decisions, independent re-run of every check through Terminal Commander, code read, this file, commit.
