# T1 and T2 results: core records, state machine, scenario tests

Date: 2026-10-28. Status: accepted by the chair after independent verification. Scope: PLAN T1 and T2 only; T3 onward not started.

## What exists

| File | Content |
|---|---|
| `Cargo.toml` | package `coding-supervisor` 0.1.0, edition 2024, no dependency section of any kind (normal, dev, build, target-specific) |
| `Cargo.lock` | one `[[package]]` entry, the project itself |
| `src/lib.rs` | records `TaskContract`, `Candidate`, `VerificationReceipt`, `Finding`, `Checkpoint`; `TaskState` with the ten states; the `Task` gate with freshness invalidation, bounded repair, receipt validation and resume; `#![forbid(unsafe_code)]`; 438 lines |
| `tests/scenarios.rs` | `FakeRunner` fake provider, Duet contract helper (C1 to C4 mandatory, scope `src/` and `tests/`, two repair rounds), twelve tests |

All four files are LF (zero carriage-return bytes). No `target/` directory inside the checkout; artifacts under `C:\Target\coding-supervisor\main\target`.

## Executed verification (chair, Terminal Commander, all with `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, cwd `E:\project\coding-supervisor`)

Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, resolved from `rust-toolchain.toml`.

| Command | Exit code | Evidence |
|---|---|---|
| `cargo test` | 0 | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` (scenarios binary); lib unit tests: 0. An earlier run of the worker's handoff showed 11 passed before the chair's additions below. |
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
| (none) | state machine (T1 acceptance) | `illegal_transitions_return_typed_errors_and_preserve_state` | ok | `try_accept`, `request_repair`, `observe_candidate` from `Queued`; `start_preparing`, `finish_repair`, `complete` from `Verifying`; `record_receipt` from `Repairing` (no receipt stored, repair count unchanged); `block`, `fail`, `raise_finding`, `finish_repair` from `Cancelled`. Each returns `InvalidTransition { from, event }` and leaves state, candidate, receipts and findings unchanged. |

## Candidate identity: who owns it

`CandidateId { base_commit, dirty_digest }` is supplied by the caller. The core never reads a repository, never hashes a working tree and never computes a digest; it only compares identities for equality. Fingerprinting uncommitted content is therefore **not implemented** in this slice and remains an adapter obligation (T4 onward): the adapter must produce a `dirty_digest` that changes whenever tracked or untracked content it covers changes, and must state what it covers. The freshness tests (A19, R28, X2) use synthetic identities (`"one"`, `"two"`, `"other"`); they prove that the gate invalidates and rejects on an identity change, not that any real content change would be detected. No claim of candidate-change detection is made for a live repository until an adapter exists and is tested.

## Source checks (checks, not proofs)

These are mechanical scans and compiler runs. Passing them does not prove the absence of panics or memory errors; failing any of them would have blocked acceptance.

| Check | Method | Result |
|---|---|---|
| No explicit panic call in production source | `grep -nE 'panic!|unwrap\(|expect\(|unreachable!|todo!|unimplemented!|assert!|assert_eq!' src/lib.rs` | zero matches after the chair replaced the worker's single `expect` in `try_accept` with a typed `ReceiptMissingExitCode` arm |
| No unsafe code | `#![forbid(unsafe_code)]` is line 1 of `src/lib.rs`; a violation is a compile error | present; `cargo test` compiled |
| Clippy with warnings denied | `cargo clippy --all-targets -- -D warnings` | exit 0 |
| Arithmetic | one `+= 1` on `consumed_repair_rounds: u32`, executed only when `consumed < max_repair_rounds`, so it cannot overflow | by inspection |
| Indexing | no slice or `Vec` indexing in `src/lib.rs`; all lookups are iterator `find`/`any` | by inspection |

Not checked: stack exhaustion, allocation failure, or panics inside the standard library.

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
- Chair: design decisions, independent re-run of every check through Terminal Commander, code read, the `expect` removal, the illegal-transition test, this file, commits `b90b565` (worker's crate plus this file) and the follow-up commit with the chair's additions.

## Follow-up: state-machine correctness repair (same day, after `ec632a8`)

### Mapping to Duet spec section 9 (`docs/archive/inherited-specs/Duet-Hermes-Ultra-Harness-Implementation-Spec-v1.md`, lines 240 to 246)

What the spec states explicitly:

- Path `queued -> preparing -> verifying -> ready_for_review`; "verification failure may enter `repairing -> verifying`"; other terminal/paused states `blocked`, `cancelled`, `budget_exhausted`, `failed`.
- "`ready_for_review` requires all mandatory checks passing on the current candidate."
- "v1 conservatively invalidates all code-check results whenever candidate content changes."
- Resume "reobserves the workspace before mutating"; "restarting does not reset consumed budgets"; a stopped or budget-exhausted task "produces a partial receipt with missing criteria and a resume reference".

| Transition in the core | Spec basis | Label |
|---|---|---|
| `observe_candidate` in `ready_for_review`; identity change marks every receipt stale and returns the task to `verifying` | Staleness: explicit ("invalidates all code-check results whenever candidate content changes"; "passing on the current candidate"). The return arrow `ready_for_review -> verifying` is not drawn in the spec. | Staleness explicit; the state return is an implementation interpretation serving A19 |
| `observe_candidate` in `ready_for_review` with the same identity: no change | Follows from the same two sentences | Explicit by implication |
| `request_repair` from `ready_for_review` | Spec draws `repairing` only from a verification failure. A finding raised during review is not a verification failure in the spec's words. | Implementation interpretation serving bounded repair; cap and `budget_exhausted` behaviour unchanged |
| `block`, `fail`, `cancel` without prior re-observation after resume | Spec: resume "reobserves the workspace before mutating". Stopping mutates task state, not the workspace, and `on_block: checkpoint_and_report` needs a stop that works when the workspace is gone. | Implementation interpretation serving A20 and `checkpoint_and_report`; all other transitions keep the re-observation precondition |
| `completed` state, reached by `complete()` from `ready_for_review` with a fresh acceptance check | **Not in the spec's state list.** Added by this repository's `CLAUDE.md` (first-slice scope) as the explicit reviewer-accept terminal. | Repository addition; reported, not rewritten |
| `blocked` treated as terminal | Spec calls it "terminal/paused". Un-blocking is not modelled; a checkpoint taken in `blocked` resumes as `blocked`. | Narrower than the spec's wording; reported, not changed |

No conflict with the explicit transitions. Two differences from the spec's state list are reported above and left as they are.

### `forbidden_effects`: declared, unenforced

The field exists on `TaskContract` and is stored. Nothing in the core observes an effect, so a forbidden effect neither blocks acceptance nor is prevented. Documented in a doc comment on the field, in `CLAUDE.md` and here. Requirements for a later slice, not implemented now:

1. Adapter-supplied observations: the core cannot see effects; an adapter must report each observed effect (kind, target, time, evidence reference) as a typed record.
2. Effect history scoped to the task or attempt, not the candidate: it must survive candidate changes and resume, because an effect that happened is not undone by a new candidate or a restart.
3. Explicit observation coverage per adapter: which effect kinds the adapter can see and which it cannot, so an absent observation is not read as "no effect".
4. Two distinct capabilities, declared separately: rejecting acceptance after a forbidden effect is observed (gate-side, possible once 1 to 3 exist) versus preventing the effect (needs an interception point in the harness, the subject of T3).

### Changes made (`src/lib.rs`, `tests/scenarios.rs` only)

| Gap | Change | Test | Result |
|---|---|---|---|
| 3a | `observe_candidate` allowed in `ReadyForReview`; an identity change marks every receipt stale and returns the task to `Verifying` | `review_candidate_change_invalidates_receipts_and_reverts_to_verifying`, `review_same_candidate_preserves_evidence` | ok: after the change `complete()` is refused (`InvalidTransition` from `Verifying`) and `try_accept()` reports `StaleReceipt(C1)`; the same identity leaves state and receipts untouched and `complete()` succeeds |
| 3b | `request_repair` allowed from `ReadyForReview` | `review_finding_enters_bounded_repair` | ok: a Low finding raised in review enters `Repairing` (consumed 1), the cycle repeats to the cap, the third request returns `RepairBudgetExhausted { max: 2 }` with state `BudgetExhausted`, consumed still 2, the finding still open, `receipt_status(C1)` still `Current`, `complete()` refused. "Without state mutation" is read as: no change to consumed rounds, receipts or findings; the state itself follows the existing R23 contract and becomes `BudgetExhausted`. |
| 3c | `block`, `fail`, `cancel` no longer require re-observation; terminal-state restriction kept; no other precondition changed | `stopping_a_resumed_task_needs_no_workspace` | ok: on a resumed, unobserved task `try_accept()` still returns `CandidateNotObserved`, each stop succeeds, the reason lands in `checkpoint().next_action`, and `complete()` afterwards returns `CandidateNotObserved` (the observation precondition is checked before the state check; a typed error with no mutation, not `InvalidTransition`) |
| item 1 | none (tests only) | `block_fail_cancel_are_terminal_and_refuse_complete` | ok: from each terminal state `complete`, `try_accept`, `request_repair` return `InvalidTransition { from, event }`; state, receipt status, open findings and consumed rounds unchanged |
| item 1 | none (tests only) | `resume_from_repairing_reobserves_before_transition` | ok: resume restores `Repairing` and consumed 1; `finish_repair` is refused until `observe_candidate`; afterwards receipts are absent (`MissingReceipt(C1)`) |
| item 2 | doc comment on `forbidden_effects` | none | documented, unenforced |

Worker brief errors, corrected by the chair rather than worked around: the 3b sequence as dispatched used a High finding, which blocks acceptance by design, so the worker's version stopped at `OpenFinding`; the chair rewrote the test with a Low finding so the cycle reaches the cap. The 3c brief expected `InvalidTransition` after a stop on an unobserved task; the code returns `CandidateNotObserved` first, which the chair accepted as the typed, non-mutating result it is.

### Verification (chair, Terminal Commander, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`)

| Command | Exit code | Evidence |
|---|---|---|
| `cargo test` | 0 | `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo fmt --check` | 0 | no diff |
| `cargo clippy --all-targets -- -D warnings` | 0 | `Finished dev profile` |
| `cargo tree -e normal,build,dev --target all` | 0 | single line, the project itself |
| `grep -nE 'panic!|unwrap\(|expect\(|unreachable!|todo!|unimplemented!|assert!|assert_eq!' src/lib.rs` | no match | zero explicit panic sites in production source (a check, not a proof) |
| `grep -nE 'dependencies' Cargo.toml` | no match | zero dependency sections of any kind |

Worker: Codex session `session-mvs8rj0i-kvghu2ah`, background, high reasoning, about 8 minutes. Test count went from 12 to 18.
