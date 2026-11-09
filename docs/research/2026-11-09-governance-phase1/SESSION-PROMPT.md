# Session prompt: governance Phase 1 (local only) for coding-supervisor

Paste everything below the line into a new Claude Code session started in `E:\project\coding-supervisor`.

---

You are working in the coding-supervisor repository (`E:\project\coding-supervisor`, branch `main`, version 0.13.1 at `e25a0d3`). The owner decided on 2026-11-09: **extend coding-supervisor with local governance features only. The central server is postponed.** Instead of a server, signed receipts travel with the code and CI verifies them. Your job is to plan and build that, in small reviewed steps.

## Read first

1. `CLAUDE.md` and `HANDOVER.md` in this repo. They are binding.
2. `docs/research/2026-11-09-governance-phase1/PHASE0-REPORT.md`: the read-only discovery that led here. Use §3 (what exists vs the spec, with file:line), §5 "Local supervisor" and "Receipt signing options", §6 (what each layer can and cannot enforce) and §8. Ignore everything about the VPS, the server crate, remote MCP and deployment.
3. The spec `C:\Users\rakovnik\Downloads\CLAUDE_CODE_HARNESS_BOOTSTRAP.md`: sections 3 (four planes), 4 (hooks and fail-closed), 5 (limits), 9 (test matrix) and Addendum G (receipts). The Addendum's server parts are out of scope.

## Binding constraints

- **Product rule (owner, 2026-10-29):** coding-supervisor works on its own; the user surface stays install, on/off and status. Do not add user steps. Anything that would need a new manual step goes to the owner as a question.
- **Core stays standard-library only.** Signing, keystore and any new dependency go in `cli` or a new non-core crate, each with a one-line reason.
- Windows and Linux both. Secrets never appear in receipts, logs, git or chat.
- Builds and tests go through Terminal Commander with `CARGO_TARGET_DIR=C:\Target\coding-supervisor\<worktree>\target`. Only one full test suite runs at a time on this machine. Never pipe to tail, never sleep-poll.
- Conventional commit subjects. One branch and one PR per work package, branched from current `main`. Merging into `main` is the owner's call every time (releases are automatic from `main`).
- Every behaviour change is regression-first: a failing test, then the fix.
- When a change falsifies a line in `CLAUDE.md` or `HANDOVER.md`, updating it is part of the change.

## Out of scope

The server crate, the VPS, Hostinger, remote MCP transport, Jev changes, and installs into other projects.

## Work packages (proposed order; adjust in your plan and say why)

0. **CI must report on every PR.** Today `ci.yml` skips docs-only changes through `paths-ignore` on the trigger, so a docs-only PR gets no check at all. If a required check were turned on, such a PR would wait forever. Move the docs-only skip into the existing `changes` job ("Decide whether to run"), so the workflow always runs and the other jobs report as skipped, which GitHub counts as passing. Keep the owner's "no waste" rule: nothing heavy runs for docs-only or release-please version-only changes. Then report the exact job names to require. `main` already has a ruleset (id 26854669) blocking force-push and deletion; the owner adds the required checks to it.

1. **Fail closed by default.** Today the adapter lets an action through when a validator errors (`adapter.rs:194-197`). Governed tasks must block when a required check is unavailable, crashes or returns malformed output (spec §4). Fail-open stays possible only as an explicit per-project setting that is logged and labelled advisory.
2. **Contract v2.** Add schema version, base commit SHA, an approval record (who and when), wall-clock and turn limits, required artifacts and reviewer policy. v1 contracts must still load through an explicit migration. Path rules reject traversal, absolute escapes, symlink escapes and ambiguous globs after canonical resolution. Label the approval as "recorded, not proven": an agent with a shell can run the same command a person would.
3. **Worktree creation.** Create a task branch and worktree from the contract's base SHA, then verify the path, branch, base and clean state before work starts (binding exists today, creation does not). One task per worktree. Cleanup never removes unreceipted work. Never run `git worktree remove` on a tree that contains junctions without unlinking them first.
4. **Time and turn limits.** Define exactly what a turn is. On hitting a limit, stop scheduling new work and keep the diff and logs. Do not claim a spend ceiling: hook payloads carry no cost data.
5. **Reviewer record.** Store reviewer identity, model, decision (APPROVED or REJECTED only), findings and time. A review submitted with the builder's own identity is stored as "caller-asserted" and does not count as independent. A supervisor-started reviewer run is a proposal for the owner, not part of this phase.
6. **Signed receipts.** Ed25519 through a maintained, widely used crate (check the current version before adding it). The machine key lives in the OS credential store (Windows Credential Manager; on Linux the secret service, or a 0600 file as a labelled fallback). Record the key ID. Define rotation and revocation through a public-key list in the repo. If signing is not set up, the receipt says "unsigned"; never fake it. State the limit honestly: a same-user agent can reach the key, so a local signature is an attestation, not proof.
7. **Receipts with the code, verified in CI.**
   - Export a receipt bundle into the repo (for example `.supervisor/receipts/<task>.json`) or as a PR artifact; pick one and justify it.
   - Add `supervisor receipt verify`, which checks the signature, contract hash, changed-path manifest against the contract, and the candidate tree.
   - Add a CI job that runs it and re-runs the acceptance commands; that re-run is the authoritative check. Document how another repo adopts it.
   - Branch protection is not your task: tell the owner the exact check name to require.
8. **Spec §9 test matrix.** All 20 barrier cases, on disposable fixtures, never on a real working tree. For each case record setup, action, expected result, actual result, exit code and evidence location. Cases that need the central server are marked N/A with the reason.

Also fix the stale core comment at `src/lib.rs:40-43`: `forbidden_effects` is enforced for typed entries in strict mode.

## How to work

- Route the feature work through `ecc:orch-add-feature`. Its plan approval and commit confirmation go to the advisor; the owner sees the plan once before you start building.
- Before asking the owner to merge each PR, run `ecc:orch-review` and fix what it finds. Builder and verifier must be different agents.
- Commit this folder (`docs/research/2026-11-09-governance-phase1/`) in your first docs PR.

## Stop points

1. After the plan: a one-page plan with work-package order, crate placement, new dependencies with reasons, and open questions. Wait for the owner.
2. Each PR ready and reviewed: report, then wait for the owner to merge.
3. At the end: a results file in this folder with the §9 matrix and real exit codes, an updated `CLAUDE.md` and `HANDOVER.md`, and a list of anything deferred with the reason.

Done means: every work package is merged or explicitly deferred with a reason, the matrix report shows real exit codes, and nothing claims enforcement beyond what a test proved.
