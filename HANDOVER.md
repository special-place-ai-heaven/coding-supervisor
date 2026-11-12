# HANDOVER — coding-supervisor, prepared 2026-11-12 (updated the same day after 0.21.0)

For the next session in this repository. It replaces the handover of 2026-10-28/29. `CLAUDE.md`
is the live truth and wins where the two disagree. The earlier handover (lab paths and hashes,
harness cohort numbers, Jev routing details, Phase A and B notes) is in git:
`git show 4326a2f:HANDOVER.md`.

## 1. The product

coding-supervisor is a completion gate for coding agents. It runs inside a harness (Claude Code
today, through hooks) and accepts a task as done only when current, observed evidence shows every
required check passed on the exact code being handed in. It can export that evidence as a signed
receipt that CI checks again before a pull request is merged.

## 2. Merged and released

Latest release: 0.21.0 (tag `v0.21.0`, 2026-11-12; `CHANGELOG.md`). Governance Phase 1 plan:
`docs/research/2026-11-09-governance-phase1/PLAN.md`. Ledger: `GATES.md` in the same folder.

| WP | What | PR | Pinned by |
|---|---|---|---|
| 0 | CI reports every pull request; docs-only PRs skip the heavy jobs | #62 (one docs list: #64) | `GATES.md` W0.1, W0.2 (CI behaviour, no Rust test) |
| D | Plan, ledger, `check.mjs` | #67 | — |
| 1 | Adapter errors fail closed at `enforce` and `strict` | #74 | `adapters/claude-code/tests/fail_closed.rs`, `cli/tests/fail_closed_escape.rs` |
| 2 | Contract v2 and the v1 migration | #76 | `adapters/claude-code/tests/contract_v2.rs` |
| 3 | `task start --worktree` and `task cleanup` | #106 | `cli/tests/worktree_create.rs` |
| 4 | Turn and wall-clock limits | #80 (MCP limit fix in #103) | `adapters/claude-code/tests/limits.rs`, `adapters/mcp/tests/protocol.rs` |
| 5 | Reviewer record | #82 | `adapters/claude-code/tests/review_record.rs` |
| 6 | Signed receipts | #87 (machine key: #93) | `cli/tests/receipt_sign.rs` |
| 7 | Receipt bundles and the CI job `Verify supervisor receipts` | #96 (follow-ups: #99) | `cli/tests/receipt_verify.rs`, `cli/tests/receipt_in_tree.rs`, `cli/tests/receipt_exemption.rs` |
| 8 | The 20-case results matrix, `RESULTS.md` | #108 | the tests named per case in `RESULTS.md` |

Field fixes merged beside the plan: #73 (session takeover), #78, #79, #83, #91, #94, #103 (early
gate feedback, limits and paths), #110 and #112 (session friction, ledger G53 to G60 in
`docs/research/2026-11-02-gate-modes/LEDGER.md`). `CHANGELOG.md` and `OPEN-ITEMS.md` describe them.

The ledger `GATES.md` is closed (#109, all 34 gates met). "All met" means every work package is
merged, not that every barrier of the spec is complete: see `RESULTS.md`.

## 3. How a code pull request gets merged

Every pull request needs a verified receipt bundle in `supervisor-receipts/`, unless:

- every changed path is on the docs list `.github/docs-paths.txt` (docs-only), or
- it is release-please's version-only pull request, or
- the owner applied the `receipt-exempt` label after the last push (then re-run the job).

The rule is `supervisor receipt required`; the CI job is `Verify supervisor receipts`. Details and
limits: the WP7 bullet in `CLAUDE.md` and `docs/adopting-receipt-verification.md`.

Signing key: Windows Credential Manager entry `coding-supervisor/receipt-signing-key`, key id
`56fefcdc588ccb5b`, listed in `supervisor-keys.json`. CI reads that list from `main`.

Recipe that has worked (bundles for #99, #103, #106, #110 and #112 are in `supervisor-receipts/`; `GATES.md`
W7.8 records a pass without the label):

1. Make a worktree at the merge base of the branch and `main`, with the branch content as
   uncommitted changes. CI compares from the merge base.
2. Run a headless governed Claude Code session there.
3. Run every check through Terminal Commander, with the same `CARGO_TARGET_DIR` in the session's
   environment as the hook sees. A different value is refused at `enforce` and `strict`.
4. Close each job with a `command_status` call that returns its exit code. An exit seen only
   through `bucket_wait` is not recorded.
5. Run `supervisor receipt export` before the work is committed. After the commit it refuses.
6. Run `supervisor receipt verify --bundle <file>` locally and expect `valid`, exit 0.
7. Commit the work, then the bundle (so far as its own `chore: add the supervisor receipt ...`
   commit), then open the pull request.

Order that avoids paying for a session twice (a code change after export makes the receipt
stale): independent review passes, `cargo test --workspace` finishes locally, push and open the
pull request, the Linux job is green (it is the first compile of unix-only test code), then run
the session, export, commit the bundle. Start the session as a detached process, not as a child
of Terminal Commander: a daemon restart killed one mid-run on 2026-11-12. A killed session can be
continued with `claude -p --resume <session id>`; the gate keeps the task.

A second machine needs its own signing key. `supervisor receipt sign` creates one at first use
and prints the entry for `supervisor-keys.json`; CI reads that list from `main`, so the entry has
to be merged before a receipt signed on that machine verifies. `supervisor-keys.json` is not on
the docs list, so the pull request that adds the key needs the owner's `receipt-exempt` label (as
#93 did) or a receipt from a machine whose key is already listed.

`docs/research/2026-11-09-governance-phase1/WP7-LIVE-RUN.md` records the first attempt. It
produced no receipt. Read it for the setup (merge-base worktree, hooks from a settings file), how
a Terminal Commander job links to a criterion, and the time-limit failures on a loaded machine.

Owner's rules:

- No branch protection is used. The owner vets every committer.
- No agent merges while a check is red. A red result is fixed until the merge is clean.
- Only the owner applies the `receipt-exempt` label.

## 4. The results matrix

`docs/research/2026-11-09-governance-phase1/RESULTS.md` records the 20 cases of spec section 9,
each run as named existing tests on Windows, all with exit 0. 8 cases are shown (3, 7, 9, 10, 11,
16, 17, 20), 11 are partly shown (1, 2, 4, 5, 6, 8, 12, 14, 15, 18, 19) and 1 is not built (13:
no local commit barrier). Exit 0 means the named tests held, not that the barrier is complete.
The gaps are in its section "Deferrals and gaps". Most evidence is replay, not a live session.

## 5. Open work, in priority order

1. Friction fixes from field use (`docs/research/2026-11-11-open-items/OPEN-ITEMS.md`). Merged in
   #110 and #112: the read-only shell allow-list read the way Bash reads a command, the
   per-session intent comparison, the harmless environment variable list, the `intent_apply`
   wording, the pending-job hold that is not a numbered refusal, the acceptance sentence at the
   next prompt, and a check run kept when a poll cannot read the code version. Still open:
   - a `Workflow` launch counted as outstanding background work, like `Agent` (plan item 4), and a
     teammate's message mid-turn not treated as a changed prompt (5b). Both wait for a captured
     real hook payload;
   - an acceptance line at the completing Stop itself (measured: it changes the answer in 56
     tests; left for the owner);
   - a foreground shell run whose own PostToolUse saw no code version is still lost (G60).
2. `CLAUDE.md`: the bullets for WP7, WP6, WP4 and the first friction batch still carry review
   history. The WP3 bullet was condensed in #112 and is the model; check every statement against
   the code when condensing (the first draft of the WP3 rewrite was wrong in several places).
3. Re-test the items of the 2026-11-06 friction report (A1.1 to A3 in `OPEN-ITEMS.md`) against
   `main`. None has been re-checked.
4. The matrix gaps, listed in `OPEN-ITEMS.md` section "Gaps recorded by the results matrix
   (2026-11-12)". Biggest: approval is not checked (2), base-checkout writes by shell or MCP are
   not refused (6), no local commit barrier (13), no per-artifact hashes (14), an agent proposal
   can bind an existing worktree (8, 19).
5. One task per git worktree, so two sessions can work in one project (decided 2026-11-11, option
   b; not built beyond a person's `task start --worktree`).
6. Empty scope for a task that changes nothing (decided 2026-11-11; not built: an empty scope is
   still refused); clearer `revise` messages.
7. Smaller: CI has no job timeout; `supervisor status` shows `registration: not_configured` while
   hooks fire; the receipt job re-runs every bundled command (WP7-LIVE-RUN follow-up 2); the
   hook's own 20 s limit under load; the skills that `supervisor install` renders into a checkout
   go stale until it is run again after a release.
8. Other harnesses: Codex, Cursor and Grok live-model runs were never done
   (`docs/research/2026-10-29-installable/`).

## 6. Limits that matter most

- Same OS user: an agent running as the person can read the signing key and, past the guard,
  edit the state. The guard matches command text, so a command spelled another way gets past it.
  Known misses: `npx coding-supervisor@<version>`, `node bin/launcher.js` inside the package
  folder, and exempt names that resolve to a prepared shim. A signature is an attestation, not
  proof; the CI re-run is the check that counts.
- With `isolation: worktree`, only Write, Edit, MultiEdit and NotebookEdit into the base checkout
  are refused. Shell commands and MCP write tools are not.
- Write-time scope refusal is at `strict` only; at `enforce` an out-of-scope path refuses the
  handback and the Stop instead.
- A hook that crashes or hangs at Stop or SubagentStop lets the stop through (Claude Code ignores
  `onFailure` there).
- Most features have replay tests only, no live Claude Code run.
- Linux paths (symlinks, file modes) are mostly witnessed only by CI. The matrix ran on Windows.

## 7. Environment notes that bite

- Windows host, Linux targets. `.gitattributes` forces LF; the "LF will be replaced by CRLF"
  warning is expected.
- Set `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target` (or a per-branch slot under
  `C:\Target\coding-supervisor\`) before any cargo command. Never build inside the checkout.
- Run builds and tests through Terminal Commander, one full suite at a time.
- `cargo test --workspace` takes about 25 to 30 minutes locally and is the reliable local run.
  `cargo nextest run --workspace` hit time limits on a loaded machine (WP7-LIVE-RUN.md).
- The dogfood gate in the main checkout refuses a shell command whose text names a person-only
  command or the gate's state folder. Put such text into files with the file-writing tools, not
  through shell.
- This checkout's root is also the Claude Code plugin: no root `.mcp.json` (see `CLAUDE.md`).
