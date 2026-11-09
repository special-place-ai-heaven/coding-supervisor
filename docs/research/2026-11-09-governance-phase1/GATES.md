# Gates: governance Phase 1 (local only)

OWNS: .github/workflows/**, src/**, tests/**, adapters/**, cli/**, docs/**, CLAUDE.md, HANDOVER.md, supervisor-keys.json, supervisor-receipts/**

Scope: every work package in PLAN.md is merged into main or abandoned here with a reason, and main passes the supervisor gate's own commands.

How to run: from the main checkout at `E:\project\coding-supervisor` after `git pull`, with `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, through Terminal Commander:
`node C:\Users\rakovnik\.claude\skills\unlazy\scripts\gate-check.mjs --timeout 3600 --root . --cwd . docs/research/2026-11-09-governance-phase1/GATES.md`.
The cargo gates use the commands the supervisor contract (`supervisor-contract.json` C3) and `ci.yml` check. Where a command prints nothing on success, `&& echo <marker>` is appended and the command itself is unchanged. The `merged` oracle also fails unless the checkout is at `origin/main`, so every later gate measures the merged code. The `--test <file>` names are part of this contract: each package must use exactly those test-file names. `docs-only-ci` has no passing control before WP0 lands. Its control is PR D itself.

- [ ] G0: this ledger states outcomes that can fail
  CHECK: node C:\Users\rakovnik\.claude\skills\unlazy\scripts\gate-lint.mjs docs/research/2026-11-09-governance-phase1/GATES.md
  EXPECT: LINT OK
  EVIDENCE: pending

- [ ] W0.1: the CI change that reports every pull request is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged ci/report-every-pr
  EXPECT: merged into main: ci/report-every-pr
  EVIDENCE: pending

- [ ] W0.2: a docs-only PR got all three CI checks, the decide job passing and both heavy jobs skipped
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs docs-only-ci docs/governance-phase1-plan
  EXPECT: heavy ones skipped
  EVIDENCE: pending

- [ ] W0.3: the owner was told the exact check names to require in ruleset 26854669
  EVIDENCE: pending

- [ ] W1.1: fail-closed by default is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/fail-closed
  EXPECT: merged into main: feat/fail-closed
  EVIDENCE: pending

- [ ] W1.2: an adapter error blocks a governed task and the explicit fail-open setting is logged as advisory
  CHECK: cargo test -p supervisor-claude-code --test fail_closed
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W1.3: while the hooks fail closed, turning the gate off from a plain terminal succeeds and the next hook call no longer blocks (spec section 9 case 18)
  CHECK: cargo test -p supervisor --test fail_closed_escape
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W2.1: contract v2 is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/contract-v2
  EXPECT: merged into main: feat/contract-v2
  EVIDENCE: pending

- [ ] W2.2: v2 contracts load, v1 contracts load only through the migration, and path escapes are rejected
  CHECK: cargo test -p supervisor-claude-code --test contract_v2
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W2.3: the core comment no longer claims forbidden_effects is unenforced
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs stale-comment-fixed
  EXPECT: no longer claims it is unenforced
  EVIDENCE: pending

- [ ] W4.1: time and turn limits are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/task-limits
  EXPECT: merged into main: feat/task-limits
  EVIDENCE: pending

- [ ] W4.2: reaching a limit denies new work and leaves the diff and journal unchanged
  CHECK: cargo test -p supervisor-claude-code --test limits
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W5.1: the reviewer record is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/review-record
  EXPECT: merged into main: feat/review-record
  EVIDENCE: pending

- [ ] W5.2: a builder-identity review is stored caller-asserted and does not satisfy an independent-review contract
  CHECK: cargo test -p supervisor-claude-code --test review_record
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W6.1: signed receipts are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/signed-receipts
  EXPECT: merged into main: feat/signed-receipts
  EVIDENCE: pending

- [ ] W6.2: a receipt signs and verifies, a changed byte or revoked key fails, and a missing key yields unsigned
  CHECK: cargo test -p supervisor --test receipt_sign
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W7.1: receipt export, verify and the CI job are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/receipt-verify-ci
  EXPECT: merged into main: feat/receipt-verify-ci
  EVIDENCE: pending

- [ ] W7.2: verify rejects a wrong signature, contract hash, out-of-scope path and stale tree
  CHECK: cargo test -p supervisor --test receipt_verify
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W7.3: the receipt verification job ran and passed in CI on its own PR
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs ci-passed feat/receipt-verify-ci "Verify supervisor receipts"
  EXPECT: job passed on PR
  EVIDENCE: pending

- [ ] W7.4: committing a receipt into supervisor-receipts/ leaves the candidate identity unchanged, so the receipt still verifies against the commit that contains it
  CHECK: cargo test -p supervisor --test receipt_in_tree
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W7.5: a PR to main with no receipt and no exemption fails the receipt job in CI
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs ci-failed test/receipt-required "Verify supervisor receipts"
  EXPECT: job failed on PR
  EVIDENCE: pending

- [ ] W7.6: the exemption rule is decided in CI only by docs-only, release-please or an owner-applied label, never by branch name
  CHECK: cargo test -p supervisor --test receipt_exemption
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W3.1: worktree creation is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/worktree-create
  EXPECT: merged into main: feat/worktree-create
  EVIDENCE: pending

- [ ] W3.2: a created worktree is verified for path, branch, base and cleanliness, and cleanup keeps unreceipted work
  CHECK: cargo test -p supervisor --test worktree_create
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] W8.1: the barrier matrix is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged test/barrier-matrix
  EXPECT: merged into main: test/barrier-matrix
  EVIDENCE: pending

- [ ] W8.2: RESULTS.md records all 20 spec section 9 cases with an integer exit code or a server N/A reason
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs matrix
  EXPECT: matrix: 20 cases recorded
  EVIDENCE: pending

- [ ] F1: main passes the formatting check
  CHECK: cargo fmt --all -- --check && echo fmt-clean
  EXPECT: fmt-clean
  EVIDENCE: pending

- [ ] F2: main passes clippy with warnings as errors
  CHECK: cargo clippy --workspace --all-targets -- -D warnings && echo clippy-clean
  EXPECT: clippy-clean
  EVIDENCE: pending

- [ ] F3: main passes the supervisor regression criterion
  CHECK: cargo test --workspace
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: pending

- [ ] F4: the core crate still has no dependencies
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs core-no-deps
  EXPECT: core crate has no dependencies
  EVIDENCE: pending

- [ ] F5: CLAUDE.md and HANDOVER.md describe the merged state, with every enforcement claim traced to a named test
  EVIDENCE: pending

- [ ] F6: every deferral is listed in RESULTS.md with its reason, and each one appears here as an ABANDON line
  EVIDENCE: pending
