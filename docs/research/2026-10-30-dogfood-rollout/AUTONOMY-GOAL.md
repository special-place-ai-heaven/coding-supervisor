# Autonomous workflow execution

## Effective scope

Owner request, 2026-10-30: develop the installed supervisor into an install-once workflow. Ordinary tasks must not require users to manage contracts, commands or CHECK headers. Reuse existing lifecycle, hooks, MCP and skills; do not add a routing engine or another enforcing adapter. Preserve existing work, archives and sibling sessions; keep the other seven projects inactive. No commits, pushes or releases.

**Quota amendment:** the owner subsequently prohibited Claude model sessions. Live Claude acceptance is deferred. Complete implementation and independent verification using Codex MCP, deterministic hook replay, subprocess integration and local fixtures. Report Claude live checks as unverified; replay is not live enforcement proof. This amendment supersedes the live-Claude requirements in the native goal's original text. The available native goal API changes status but does not edit an active objective. The owner explicitly confirmed that the native goal may finish once implementation and the non-Claude checks pass under this amended scope.

## Completion criteria

1. Actionable requests propose a scoped contract, deterministically validated before edits; discussion stays inactive.
2. Edits, delegation and command evidence retain task/session/worktree/candidate/job identity and asynchronous TC recovery.
3. Completion derives evidence from observed executions without CHECK headers; missing/failed checks drive bounded repair; exhaustion stays unresolved.
4. Automatic archive/rollover, follow-ups, cancellation, pause/resume and scope changes preserve unfinished work and revision history. Model proposals cannot waive or weaken requirements.
5. Restart, concurrency and duplicate delivery preserve identity, archives, repair accounting and pending linkage.
6. Regression/adversarial tests reject asserted, stale, wrong-worktree, cross-task, failed and unknown evidence, plus self-weakened contracts.
7. Independent test_runner, code-reviewer and security_reviewer examine the delivered candidate. Final fmt, clippy, workspace tests and release build pass.
8. Deferred live-Claude scenarios and exact rerun instructions remain recorded. No full enforcing acceptance claim.

## Development supervision

The native goal was created without a token budget. Installed supervisor 0.1.1 remains in place. Codex MCP responded and started development task `task-18e349d168b369e0-c270-0`. Its contract requires formatting, clippy, workspace tests, release build and an external acceptance-artifact verifier. MCP is advisory and cannot enforce stopping. Candidate binaries and fixtures remain separate from the installed supervisor.

After the owner explicitly amended acceptance, supported `task next --disposition cancelled` archived that original contract and created `task-18e34a48b321a7b8-8c18-0` with the non-Claude acceptance criterion and retained deferred scenarios. TC job `job_01a18c82eada71f794aa4e9b4b49ad0f` observed exit 0; MCP confirmed the new task. No checks were waived or represented as run. The agent-managed contract is external at `C:\Target\coding-supervisor\autonomy-20261030\development-contract-amended.json`; the inherited project draft remains unchanged.

Initial source: `3c134d16d9604a88046fe1b16d04a74af5d25e51`, dirty digest `9baeab25b4e2b84e3544617b109cbb2c776da998cc2e7b6491426bfcc1da4cbc`. Inherited `.gitignore` edits and untracked `supervisor-contract.json` draft must remain intact.

## Reconciled prior intent

The 2026-10-29 autonomy ruling proposes install-once, default-on integration, inferred repository checks and headerless observed completion. Its per-session activation conflicts with the current request's inactive discussion and consecutive tasks in one conversation; the current request controls. The earlier rollout explicitly activated tasks and required CHECK citations. Its immutable archives, transactions and durable TC observation remain the implementation foundation. Plugin packaging is not required for this task's existing installer workflow.

## Dogfood journal

| Event | Expected / actual | Identity and reproduction | Assessment |
|---|---|---|---|
| Bootstrap | No unfinished goal overwritten; actual native goal absent, then created | Initial `get_goal`; no source edits | Correct |
| MCP connection | Installed server available; `task_state` reported no contract | Codex loaded `mcp__supervisor__task_state`; followed by explicit agent-managed start | Correct inactive behavior, manual operation to automate |
| Status | Distinguish configured hooks and loading; status reported partial matcher coverage and unknown model loading | Installed 0.1.1 `supervisor status --json`, TC `job_01a18c7ac068758baabd820ef9219235` | Accurate bounded report; investigate candidate registration coverage |
| Baseline | Run workspace suite before implementation | TC `job_01a18c7b86987397be16cb355836e1f4`, initial candidate above | Observed exit 0; 33.6 seconds; process cleanup complete |
| Resource correction | No Claude sessions should consume quota | Owner amendment before any model session launch | Live scenarios deferred; no calls launched |
| Scope correction | Preserve original acceptance history and apply owner amendment | Archive/next job `job_01a18c82eada71f794aa4e9b4b49ad0f`; successor above | Correct explicit rollover; manual agent work to automate |
| Hook registration | Pending intent must see MCP tool mutations; existing matcher excluded them | Installer regression `job_01a18c7fe67070868aa246b0fc70f6b6` observed exit 101; repaired all-tool registration `job_01a18c8135177063ac108fcf65205893` observed exit 0 | Local subprocess proof only; installed config untouched |
| Headerless completion | Observed runs should suffice without model headers; initial chair/worker regressions failed | Red `job_01a18c7dc3c877aab2940790d1166bbe`; focused green `job_01a18c80628572888f4f92ce367a3e37` | Replay proof; later integration checks supersede |
| MCP authority | Assertions must not accept hook-owned autonomous work; accept_try initially returned ready_for_review | Red `job_01a18c8ab25f7055a289f26bfab9a8a7`; green `job_01a18c8b3ad67735b2c9f0884150c36d` | Security finding repaired with narrow authority refusal; rollover separately checks observed provenance |
| Pause visibility | Status should show preserved paused task; field initially absent | Red `job_01a18c86b9b9762494879ce852a3c8f0`; green `job_01a18c8b64e176a58c11288af2a51eb9` | Subprocess status regression; first attempt had an invalid empty-criteria fixture and is not behavioral proof |
| Custom check floor | Git-only discovery should retain previously approved custom checks; it initially dropped them | Red `job_01a18c8d560f72fb8b18942187304889`; green `job_01a18c8da2ab742b857c9b855acc3391` | Preserve approved criteria without inventing a new floor |
| Recovery usability | New session must explicitly resume and discover pending jobs; review found both paths missing | Independent CODE-REVIEW-REPRO.json and test_runner source trace under external evidence directory | In-scope repair; pending results never imply successful evidence |
| Integration latency | Full adapter suite after compatibility repairs passed 146 tests in 44.7 seconds | `job_01a18c8c430675e1af7e860750f6981b` observed exit 0 | Predates subsequent recovery changes; final checks must rerun |

Known-good supervision entered repair round 1 for independent security findings
on candidate digest `fa71cb7d5adb3dcbfc848531ef88d6c96934666df93f4f6c6866ff03b15c4952`.
The counter is retained by the installed supervisor. Preliminary independent
reports are not final passes; acceptance requires refreshed reports and checks
for the delivered source and binary.

Round 2 addresses independently reproduced cross-session resume and pending-job
discovery gaps (candidate `cc54f20fd5ea4cb48e68d21864d09c4c1f72e29c8543d0f17fdec0ae0abb080e`).
The focused recovery suite passed 60 cases, observed TC job
`job_01a18c94bc2675d1bc72e790aee81cca`; this predates the next security repairs.
Round 3 addresses silent npm verification-entry-point replacement and the
ownership-check/transaction race identified in `SECURITY-REVIEW-2.md` (starting
candidate `9cdeb44667a6bccbdd1eddcb2c3d65b500bd09a021da92bc8ac1d620124766d4`).
Entry-point hashes constrain captured commands, not arbitrary test semantics.
Neither preliminary review nor an intermediate focused suite is final acceptance.

Round 4 addresses independent review's cached-approval ownership race and loss
of the approved custom-check floor after cancellation. The seeded-cache and
cancel/start regressions both failed in TC job
`job_01a18ca250b570059be9000f42534cf9` (19 passed, 2 failed). The earlier race
test did not seed the cache, so its pass did not close this case. Both reviewers
withdrew final acceptance pending these repairs and refreshed evidence.

The first final workspace attempt, `job_01a18c9cbb1e72d08bb21ab05da79927`,
correctly failed on two installer expectations: missing worker wording that a
command runs alone, and the obsolete restricted PostToolUse matcher. The worker
discipline was restored; the matcher assertion now expects omitted matching,
consistent with the separate all-tool registration regression. No suite failure
was waived. SymForge withheld the existing installer test because its secret
detector matched; only the diagnostic-identified assertion was edited without
emitting file contents or changing the detector.

The external acceptance verifier received its own negative tests. Its first
test exposed validation-order failure (`job_01a18ca5200b7434bd43d4031ae3cdf2`);
the repaired verifier rejects candidate mismatch, missing scenarios or actual
failed execution, and missing/mismatched recovered job linkage
(`job_01a18ca571cd76eca57021280e4083d0`, exit 0). This validates the verifier's
rejections; it does not substitute for running the candidate's scenarios.

Round 4's repeated cancel/start probe additionally caught duplicate AUTO_DIFF
requirements (`job_01a18ca50b2d70deb0fdb218f3c46fc5`, exit 101). Equivalent parsed
criteria now merge; conflicting commands and weakened mandatory checks still
refuse. All 21 autonomy tests passed afterward
(`job_01a18ca5810672e2aeaffeddc0f5af61`). The installer rerun then exposed one more
missing worker instruction, the criteria table; that wording was restored too.

Round 5 addresses the remaining latest-intent race reported in
`SECURITY-REVIEW-4.md`: ownership can remain unchanged while a newer prompt
becomes pending or discussion between early inspection and the locked callback.
Both cached and uncached paths require current-intent validation. The known-good
gate correctly refused another repair request while already repairing and then
refused replaying that operation after state changed; observing the candidate
and submitting a new operation identity advanced the retained counter to five.

Workspace job `job_01a18ca7b13d718aa2a44f6a95e3e61a` reached a stale MCP test
expectation: caller registration in a hook-owned session is now refused before
intent application. The regression now asserts that earlier refusal and keeps
the separate cross-session attempt to control the trusted task. This is a
correct denial, not a reason to relax the authority check. Final fmt/clippy had
passed for that superseded source; the workspace failure remains recorded.

Independent review of the external subprocess harness found wrong assumptions
before execution: fixture state requires the physical-project namespace, and
discussion must be followed by a new actionable prompt before editing. It also
found a result-key typo and inconsistent fixture-versus-repository build checks.
These are acceptance-harness defects, not product passes. Corrected external
scripts and their actual run evidence must be independently checked before
the acceptance verifier can pass.

Round 5's pre-fix matrix failed all 12 pending/discussion interleavings
(`job_01a18cac469473bc8cc60c17c74ed84a`, exit 101). The repair also binds older
events against concurrent actionable revision, exempts usable read-only hooks
from incidental task-state changes, and preserves in-flight job observations.
The final focused run passed all 23 autonomy tests, including the 18-case race
matrix (`job_01a18cb08e89756180c33b6c860dd78c`); all seven MCP autonomy tests
passed (`job_01a18cb0cfdb727e9859f06a79dc0db2`). These are focused regression
results; the external final-candidate record remains the acceptance authority.

## Deferred live proof

When the owner authorizes Claude use after quota returns, run two ordinary actionable requests in one isolated conversation and prove activation/rollover without user commands or headers. Exercise missing/failed check refusal, actual repair and successful recheck. Include discussion, follow-up, new task, cancellation and scope change turns. Retain model transcripts, hook journals, candidate/binary identities and archive hashes separately from replay evidence. Exact candidate-specific commands will be recorded with the implemented fixture runner before delivery.
