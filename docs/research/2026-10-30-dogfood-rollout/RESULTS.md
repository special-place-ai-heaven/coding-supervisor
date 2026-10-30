# Sequential dogfood execution results

Historical first-delivery record, 2026-10-30. The subsequent independent audit rejected the A2/A3/A5 acceptance claims and found A4/A6 gaps. Passing compiler/test receipts and two live tasks did not prove all requirements. The original audit remains intact at `C:\Target\coding-supervisor\audit-20261030\AUDIT.md`; current repairs and separate re-audit evidence are in [REPAIR-RESULTS.md](REPAIR-RESULTS.md). The original `dogfood-evidence\verification-final.json` remains evidence for its own candidate only. No commit, push or release was performed.

## Authority and baseline

The owner explicitly authorized local implementation and scoped dogfooding in this checkout, with no commits, pushes, releases or remote mutations. The native harness goal is active and references GOAL.md, including A1–A6; no token or time budget was set. Product task activation is deferred until lifecycle, observed-job and concurrency checks pass. The other seven projects remain inactive and outside the mutation scope.

Baseline: main at `8c2b77a779f3f597f3cf834e4572ba8c20f5fb20`; one worktree, no staged changes. Existing `.gitignore` appends `.supervisor/`; GOAL.md and ROLLOUT.md were untracked. These files are preserved. `git fetch origin` completed with observed exit 0, TC job `job_01a18bf38e117066853c243c21b669cb`; HEAD and origin/main remain equal.

Baseline `cargo test --workspace`: TC job `job_01a18bf3e96175b5b0d039375bb8d047`, cwd `C:\AI_STUFF\PROGRAMMING\coding-supervisor`, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, observed exit 0. This is baseline evidence, not acceptance evidence for later edits. Installed compiler: rustc 1.99.1, observed by job `job_01a18bf5887c75feb00eff9c14af1594`. This host has no `C:\Target\README.md`; the mandated external target directory works.

## Agreed implementation contracts

- One shared active task per canonical physical worktree. Canonical path plus SHA-256 distinguishes legacy punctuation collisions; Windows case normalization and explicit override semantics are tested.
- Generated task identity is separate from the caller's contract identity. An active pointer selects a task directory. Rollover freezes old evidence bytes; disposition and recovery linkage are separate metadata. Unfinished rollover requires an explicit disposition. Deactivation restores inactive conversation.
- OS-backed file locking serializes hook and MCP transactions; a durable redo record supports interrupted writes. Duplicate event identities reuse recorded responses. Legacy state requires explicit migration when identity cannot be established; ambiguity never selects a first match.
- Hook sessions bind to task identities. Rebinding is explicit; late dispatch/completion events cannot supply evidence to a successor.
- TC observations retain dispatch-time task, candidate, cwd and structured command identity. Only authoritative completion of the matching job can produce evidence. Nested output text is never an exit receipt. Reconstructed status alone is insufficient; original trusted completion and linkage must be recoverable.
- Status distinguishes registration, observed connection and model-session evidence. Codex remains advisory. Action policy is separate from completion, bounded to explicitly configured supported cases, with visible unsupported routes.

## Work and verification ledger

| Check | Current state | Owner / next evidence |
|---|---|---|
| A1 | Live verified | Two actual Claude validation tasks, supported rollover/deactivation, immutable archive comparison below |
| A2 | Original claim rejected by audit | Wrong-checkout acceptance and missing reconstructed-completion recovery; see repair evidence |
| A3 | Original claim rejected by audit | Archived task-start replay and implicit fallback migration defects; see repair evidence |
| A4 | Original claim partial | Human status omitted effective gate state and named coverage |
| A5 | Original coverage claim rejected | Persistent-shell fixture used the wrong input field |
| A6 | Original completion premature | Checks passed but missed confirmed defects; stale activation guidance remained |

Every repair hypothesis is limited to two attempts before new evidence and a changed hypothesis. Full builds are serialized by the chair. Acceptance receipts must name command, cwd, candidate commit and dirty digest, observed exit/trust and artifact. Changes invalidate affected evidence. Scripted replays and actual live Claude traces will be reported separately.

## Pending jobs and recovery

No live model or baseline jobs remain running. Both actual project tasks are archived and the active pointer is inactive. On resume, inspect the final manifest and actual diff; subsequent changes invalidate affected verification. All work remains uncommitted. Recovery and supported commands are in OPERATIONS.md.

## Reproduction and integration findings

- Installed 0.1.0 baseline, isolated temporary state: `python C:\Target\coding-supervisor\dogfood-evidence\baseline_tc.py`, TC `job_01a18bf92f207666af408bd181198b01`, observed exit 0. A nested output string containing an exit code falsely reached completed; an asynchronous start followed by successful status stayed unlinked. These are synthetic hook replays; the named cargo command did not actually execute.
- Current TC `command_status` omits `degraded`; successful authoritative status carries complete process-observation and cleanup metadata instead. The new parser checks that shape explicitly. It never interprets stdout/tail/signals as completion metadata.
- Existing tamper regressions caught a cached-response bypass after modifying gate/contract files. Lifecycle repair validates task integrity before cached responses and binds the cache to candidate identity. This remains subject to independent rerun.
- Policy/status initial integration compilation failed on missing ErrorKind import and render visibility; the owners repaired the integration. Failures are not acceptance passes.
- Claude Code 2.1.314 is installed and authenticated through first-party auth. The live results below supersede the initial preflight. Read-only checks found no legacy or local contracts for the seven other rollout projects.

Original preserved-file SHA-256 values: `.gitignore` `cfd113baaf9a69dc6dc88332687b97bd6be81a39d012586151a5b88a9212bb44`; GOAL.md `5317a17f978629e35b739315beb3997e7d962e74c20f69773378adf5293d08e8`; ROLLOUT.md `ab0bfc26ad2273c3bf85873db528559d767c2677ce06ecbbcbb3d309b9ca9d04`.

## Independent review and repairs

The named code-reviewer and security_reviewer performed read-only reviews independently of implementation. Their findings caused these repairs before live activation:

- Legacy receipts remain historical after migration and cannot authorize hook completion without matching durable dispatch/completion provenance.
- Candidate observation uses the canonical physical worktree root, including edits to siblings of a nested session directory.
- Unapproved command text and CHECK input do not enter command journals or refusal text. Native background shell launches are unsupported completion evidence; TC is the supported asynchronous path.
- A candidate mismatch observed at completion permanently invalidates its dispatch. Reverting the checkout cannot revive that run.
- Restricted, asynchronous, non-command or disabled hook registrations no longer produce unrestricted coverage claims.
- MCP rejected receipt commands receive the same validation before diagnostics. The synthetic-marker regression passed, and independent security rereview closed the remaining disclosure finding.

Code-review rereview found no blocking issue in observation, lifecycle, transaction and status on its inspected hashes. Security rereview closed legacy provenance, nested-cwd freshness and hook command disclosure in source; final independent test evidence remains required. Neither reviewer claimed that source inspection proved live behavior.

The selected live tasks are concrete validation tasks in the actual coding-supervisor checkout: (1) lifecycle/archive/concurrency validation through the CLI and adapter lifecycle suites; (2) asynchronous observation and effective-status validation through observation and status suites. Their exact contracts live outside the checkout under `C:\Target\coding-supervisor\dogfood-evidence\live-task-{one,two}.json`. They were activated only after prerequisites passed and are now archived/inactive. Their full-root scope acknowledges the preserved dirty checkout; neither task authorized commits or remote effects.

## Live verification, kept separate from replays

The first actual model session (`live-one.jsonl`, TC `job_01a18c0a6ec473e1bde072af028da3e3`) ran both tests successfully but the gate refused completion: no jobs linked. The model correctly reported NOT COMPLETE. Rollover without a disposition was refused; that task was preserved.

Two causes were investigated, not waived: Claude supplies MCP results as a bare one-text-block array, unlike the enclosing object used in fixtures; and another installed harness writes `.claude/autoharness` locks/queues on each event. The parser gained strict array support after a failing reproduction (`job_01a18c0d0aa8710a9f1f6003e859e2f5`, exit 101), then 45 affected tests passed (`job_01a18c0d3079749ba3c0b9101521d332`). Independent affected tests and release rebuild passed. The runtime directory was excluded locally in `.git/info/exclude`; its files and hooks were preserved, and no guard was disabled. No source directory was excluded.

Both successful tasks ran on base `8c2b77a779f3f597f3cf834e4572ba8c20f5fb20`, dirty digest `fcdb8a7d44f2abbd176f3c270300a829924673601e4aa88f3bab1b209f2e5222`, cwd `C:\AI_STUFF\PROGRAMMING\coding-supervisor`, external Cargo target. Each ended with **two observed receipts, zero asserted receipts and one accepted Stop**. No MCP receipt/accept/complete mutation was used.

| Task | Actual Claude session / outer TC job | Observed command jobs (all exit 0) |
|---|---|---|
| `task-18e3431a2f9f274c-6a50-0` | `5ff1e9b6-898e-4014-9d69-2d339c74a780`; `job_01a18c0ed8057258a44273a473033b45` | `cargo test -p supervisor --test lifecycle`: `job_01a18c0f132e706db480b9cc8f983410`; `cargo test -p supervisor-claude-code --test lifecycle`: `job_01a18c0f38b874e6a3c11e9c77cb1736` |
| `task-18e3436ac19088a8-3160-0` | `6b77d396-793f-4f8a-afd4-5a3bcbbdc71e`; `job_01a18c0fea0271dea0c6d8f68038cdac` | `cargo test -p supervisor-claude-code observation::tests`: `job_01a18c10304b7650997df0ab2218ff5c`; `cargo test -p supervisor --test status`: `job_01a18c1050a5767f835bd2d1d6bcb111` |

Supported `task next` returned the second identity (`job_01a18c0fb6157694b0c9972167f3fbdd`, observed exit 0). `inspect_task.py` verified first completion, second completion and unchanged hashes for every first-task file after rollover (`job_01a18c0fb52172f18bce64d740743e83`, `job_01a18c11a85377cc896f693b25452b92`, `job_01a18c11a91b72989e048c5d10d78529`, all observed exit 0). `task deactivate` archived the second task and returned `inactive:true` (`job_01a18c11a9f5751083a93e70d212de84`, observed exit 0).

The separate actual asynchronous fixture ran `python C:\Target\coding-supervisor\dogfood-evidence\verify_async.py` in an isolated local checkout: TC `job_01a18c097c9d71298a49f9b965be36f3`, duration 3039 ms, observed exit 0. Its first response carried no exit status; each Pre/start-result/status-result/Stop delivery used a new supervisor process. Status confirmed completed with one observed receipt and no assertion. This proves actual command execution and process-restart linkage with scripted event delivery; it is not labeled a live model trace. Reconstructed-status recovery and rejection are separately covered by deterministic fixtures; the TC daemon was not restarted.

## Delivered evidence and retained boundaries

The executable SHA-256 used live is `08149cafd60eebeb38f5cc16a577eda2ed65e08ea82c6c9be1e37b711d6d10e3`. `live-source-manifest.json` binds 48 source/config/template files and this executable. Final documentation changes produce a different whole-checkout candidate: final checks record that new digest and compare the delivered code/binary with this live manifest. The live receipts are never relabeled as having run on the later prose digest.

Artifacts under `C:\Target\coding-supervisor\dogfood-evidence\`: `verification.json`/`verification-end.json` (initial independent checks), `verification-receipt-fix.json` (independent live-parser repair check), `verification-final.json` (final independent candidate), `live-one-retry.jsonl`, `live-two.jsonl`, their summaries, `live-one-completed.json`, `live-one-after.json`, `live-two-completed.json`, `live-source-manifest.json`, `async-*.json`, and `codex-advisory.json`. Final evidence lives outside the candidate to avoid a receipt changing its own fingerprint. Original integration files were backed up under `pre-install/` before supported local reinstallation.

Codex project registration was read and its actual MCP process initialized, listed all eight tools and returned inactive state; status reports advisory and model loading unknown (`job_01a18c127b3a76f5b7b1b94219732126`, observed exit 0). This is direct-process integration evidence, not a Codex model enforcement claim. An initial evidence-script assertion incorrectly expected a ninth `task_next` tool and was corrected to the actual eight-tool schema; rollover is CLI-supported.

Claude enforcement proved here: the installed hooks refused unsupported completion and accepted actual observed test runs at Stop. SubagentStop, policy denials, failure/timeout, cap and bypass routes retain the fixture/previous-trial coverage declared in COVERAGE.md; they were not all rerun in live models. Same-user writable hooks are not a security boundary. No npm launcher code changed, so npm tests were not required. No mandatory failure was waived.
