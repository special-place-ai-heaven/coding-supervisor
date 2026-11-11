# HANDOVER — coding-supervisor, prepared 2026-11-12

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

Latest release: 0.20.0 (tag `v0.20.0`, 2026-11-11; `CHANGELOG.md`). Governance Phase 1 plan:
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
| 8 | The 20-case results matrix, `RESULTS.md` | branch `test/barrier-matrix`, no PR yet | the tests named per case in `RESULTS.md` |

Field fixes merged beside the plan: #73 (session takeover), #78, #79, #83, #91, #94, #103 (early
gate feedback, limits and paths). `CHANGELOG.md` and `OPEN-ITEMS.md` describe them.

Ledger rows still open in `GATES.md`: W3.1, W3.2, W8.1, W8.2, F5 and F6. F6 needs one `ABANDON:`
line per deferral listed in `RESULTS.md`.

## 3. How a code pull request gets merged

Every pull request needs a verified receipt bundle in `supervisor-receipts/`, unless:

- every changed path is on the docs list `.github/docs-paths.txt` (docs-only), or
- it is release-please's version-only pull request, or
- the owner applied the `receipt-exempt` label after the last push (then re-run the job).

The rule is `supervisor receipt required`; the CI job is `Verify supervisor receipts`. Details and
limits: the WP7 bullet in `CLAUDE.md` and `docs/adopting-receipt-verification.md`.

Signing key: Windows Credential Manager entry `coding-supervisor/receipt-signing-key`, key id
`56fefcdc588ccb5b`, listed in `supervisor-keys.json`. CI reads that list from `main`.

Recipe that has worked (bundles for #99, #103 and #106 are in `supervisor-receipts/`; `GATES.md`
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

1. Friction fixes from field use (`docs/research/2026-11-11-open-items/OPEN-ITEMS.md`; the
   follow-up brief's items are not approved by the owner yet):
   - a read-only shell allow-list before an intent and in discussion turns;
   - an acceptance line telling the session its task was accepted (task id, state, receipts);
   - "pending: job <id> still running" instead of a refusal while a check runs;
   - a `Workflow` launch counted as outstanding background work, like `Agent`;
   - a teammate's message mid-turn not treated as a changed prompt;
   - clearer wording than "advisory" in `intent_apply` answers.
2. Re-test the items of the 2026-11-06 friction report (A1.1 to A3 in `OPEN-ITEMS.md`) against
   `main`. None has been re-checked.
3. The matrix gaps, listed in `OPEN-ITEMS.md` section "Gaps recorded by the results matrix
   (2026-11-12)". Biggest: approval is not checked (2), base-checkout writes by shell or MCP are
   not refused (6), no local commit barrier (13), no per-artifact hashes (14), an agent proposal
   can bind an existing worktree (8, 19).
4. One task per git worktree, so two sessions can work in one project (decided 2026-11-11, option
   b; not built beyond a person's `task start --worktree`).
5. Decided 2026-11-11 (`OPEN-ITEMS.md`): empty scope for a task that changes nothing (not built:
   an empty scope is still refused); one built-in list of harmless environment variables and
   clearer `revise` messages (check `main` before starting).
6. Smaller: CI has no job timeout; `supervisor status` shows `registration: not_configured` while
   hooks fire; the receipt job re-runs every bundled command (WP7-LIVE-RUN follow-up 2); the
   hook's own 20 s limit under load.
7. Other harnesses: Codex, Cursor and Grok live-model runs were never done
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
