# HANDOVER — coding-supervisor, prepared 2026-10-28

For the next Claude session in `E:\project\coding-supervisor`. Self-contained: everything here is verifiable from the paths and hashes given, without EverOS. Written by the session that prepared this repository (Claude Code session id `72bf98f4-4078-4efc-8359-058cb2302a19` on this machine). Hash values are SHA-256, shown as the first 16 hex characters unless a full value is needed.

## 1. Working definition and agreed direction

> A portable coding-agent supervisor that coordinates delegation and requires verifiable, current evidence before accepting task completion.

Agreed on 2026-10-28 by the owner, this session and the owner's Codex research agent:

- The lab (research and measurement repository, section 3) is the proving ground, not the product.
- First deployable slice: deterministic completion control. Core records: task, candidate, verification, finding, checkpoint. Freshness invalidation and bounded repair. No model inside the core.
- Preserve the Duet contracts C1 to C4 and scenarios A18 to A20 (section 4).
- Delegation stays advisory initially. Reuse the existing Python Jev engine where relevant; do not install another router or SDK.
- Each adapter declares `enforcing` or `advisory/audit` on its first line.
- Claude Code enforcement is unproven. The cause of Jev's 74 percent abstention is unmeasured.

## 2. State by category

| Category | Items |
|---|---|
| Implemented | Nothing in this repository. In the harness: revised agent briefs (section 5). In the lab: measurement scripts and evidence (section 3). |
| Specified | Duet spec v1 (section 4); coding research package revision 2 with 50 acceptance scenarios (section 3); first implementation plan `docs/research/2026-10-28-completion-gate-first-slice/PLAN.md`. |
| Measured | Before-cohort of the harness (section 6); Jev epoch-1 telemetry, payload proof and delegation linkage (section 7); commerce-package recompute and coding-package verification (section 3). |
| Proposed | The completion gate (PLAN.md T1, T2); the Claude Code enforcement investigation (T3); the epoch-2 Jev roster correction (section 7). |
| Unknown | Whether Claude Code can block an unsupported acceptance; the cause of the 74 percent abstention; the resolved Jev provider model id; the true follow-through rate behind timestamp-matched linkage; the after-cohort result. |

## 3. The research and measurement repository (the lab)

Path: `C:\Users\rakovnik\.claude\robert`. Git, branch `main`, single commit `7829ebe` ("Research records: agent-loop packages, coding review lens, harness measurement, Jev pilot epoch 1"), 92 tracked files, no remote. **Do not move it. Do not copy its evidence wholesale into this repository.** Cite by path and hash.

Authoritative files inside it:

| Item | Path (relative to the lab) | Identity |
|---|---|---|
| Coding research package, revision 2 (authoritative) | `Agentic-Coding-Research-2026-10-28/` | `evidence/package-manifest.json`, 32 entries, every hash verified on 2026-10-28; report `Agentic-Coding-Research-2026-10-28.md` hash `23ada9eb380ca59a`; brief `Claude-Implementation-Brief.md` hash `a0c4321eff608e3e`; 50 acceptance scenarios AC01 to AC50; cross-variant and adjudication templates under `evidence/` |
| Stale revision-1 copies (do not cite) | `Agentic-Coding-Research-2026-10-28.md` (hash `f61e1631f198cd94`), `Claude-Implementation-Brief.md` (hash `e2d966c8d67d4462`) at the lab root | Superseded by the package above |
| Commerce-domain predecessor package | `Agent-Loop-Research-2026-10-28/` | manifest 19 entries, verified; raw run records under `evidence/*.jsonl` |
| Review lens applied to both packages | `REVIEW-LENS-CODING-2026-10-28.md` | 15 checks; recompute of the commerce runs per accepted task; the t01 correction |
| Harness measurement scripts | `harness-measurement/scripts/` | `analyse.py`, `analyse2.py`, `headline.py`, `verdicts.py`, `adjudicate.py` |
| Harness before-cohort evidence | `harness-measurement/evidence/` | `before-summary.json`, `before-analyse2-summary.json`, `before-headline-summary.json`, `adjudication-final.csv` and `.json`, `codex-agents-rules-2026-10-28.patch`, `quickstart-*` reproduction files, `typesafe-ai-SKILL-2026-09-21-backup.md` |
| Jev pilot record | `jev-pilot/experiment.json` | epoch-1 fingerprints, payload proof, open items |
| Jev pilot scripts and evidence | `jev-pilot/scripts/` (`payload_trace.py`, `test_j_checks.py`, `analyse_jev.py`), `jev-pilot/evidence/` (`jev-delegation-analysis.json`, `roster-names-epoch1.txt`) | |
| Jev routing action plan (premise corrected, see section 7) | `Claude-Jev-Routing-Action-Plan.md` | |

Verify access from a fresh session: `git -C C:\Users\rakovnik\.claude\robert log --oneline -1` must print `7829ebe`, and the manifest check in section 9 must report no differences.

## 4. Inherited contracts (not in git; copy before relying on them)

Both live only in `C:\Users\rakovnik\Downloads\`, which is not durable. Copy them into the lab under `docs/` or into this repository's `docs/archive/` before the next session depends on them.

| Document | Path | Hash | What to preserve |
|---|---|---|---|
| Duet on Hermes Ultra harness spec v1 (2026-10-23) | `C:\Users\rakovnik\Downloads\Duet-Hermes-Ultra-Harness-Implementation-Spec-v1.md` | `5152b33988bc6041` | Section 9 Prepare: task contract with checks C1 baseline reproduces the failure, C2 candidate passes the reported test, C3 configured regression suite passes, C4 diff within scope; `max_repair_rounds`; states `queued, preparing, verifying, ready_for_review, repairing, blocked, cancelled, budget_exhausted, failed`. Section 13: A18 self-reported pass cannot mark ready; A19 candidate change makes a check stale; A20 resume keeps contract, budget, pending effects, candidate identity. |
| Hermes harness research ledger (2026-10-23) | `C:\Users\rakovnik\Downloads\Hermes-Harness-Research-Ledger-2026-10-23.md` | `f068c894caf2a0ea` | H01 to H19 method cues; evaluation plan T01 to T16; per-accepted-task cost rule |
| Build Your Own Duet (source PDF) | `C:\Users\rakovnik\Downloads\build-your-own-duet.pdf` | `a57db5d75700b3d1` | Background only |

No Rust or TypeScript product repository for "Hermes Ultra" exists on this machine. The only local Hermes material is the Nous Hermes Agent adapter under `C:\Users\rakovnik\.agents\`. The target of the research packages is therefore unnamed and unpinned; this repository is the product that fills that gap.

## 5. Harness changes already made (live; do not revert, do not extend during the cohort)

Applied 2026-10-28 before 12:08:18Z. Model and effort unchanged everywhere so the before and after cohorts isolate the brief change.

| File | Change | Hash after |
|---|---|---|
| `C:\Users\rakovnik\.claude\plugins\marketplaces\fable-orchestration\agents\fresh-verifier.md` and `...\plugins\cache\fable-orchestration\fable-orchestration\1.0.0\agents\fresh-verifier.md` | Terminal Commander tools added; every check must run and be cited by exit code; "would run" counts as not run; no runnable check means `VERDICT: UNVERIFIED`; fixed first two lines `VERDICT:` and `CHECKS RUN:` | `826287f59e3fb26820b685bfeeff0f5e70108a107ea2b357273bce932bc81db7` (full, both copies identical) |
| `C:\Users\rakovnik\.claude\agents\*.md` (17 of 26 changed) | Command rule plus Terminal Commander tools in 15 command-running agents; verdict header in 6 judges (code-reviewer, security-reviewer, test-runner, browser-tester, impeccable-finish-reviewer, unlazy-master) | code-reviewer `aee456c6b59690ab`, test-runner `ff7b016a538800bf`, security-reviewer `3010651dd78f4a88`, browser-tester `9a3ed2104c3e9808` |
| `C:\Users\rakovnik\.codex\agents\*.toml` (13 of 26 changed) | Same two rules in the instruction body; all 26 files parse | diff in the lab at `harness-measurement/evidence/codex-agents-rules-2026-10-28.patch` |
| `C:\Users\rakovnik\.claude\skills\typesafe-ai\SKILL.md` (junction to `.agents\skills\typesafe-ai`) | Updated to upstream typesafe-ai/skills release v0.6.2, 288 lines, LF | `d1513cb2e0f72f58`; previous copy in the lab evidence folder |

Known pre-existing issues, left as found: four agent files have frontmatter that a strict YAML parser rejects but Claude Code loads (performance-engineer, tech-researcher, ui-designer, vite-pro).

## 6. Harness measurement: before-cohort and the after-cohort procedure

Before-cohort (newest 200 session transcripts, all pre-change, computed 2026-10-28; sources in the lab `harness-measurement/evidence/`):

| Measure | Value |
|---|---|
| Judge dispatches (fresh-verifier 140, code-reviewer 54, browser-tester 28, security-reviewer 22, test-runner 2) | 246 |
| Completed with compliant header / without / interrupted / worker file missing | 0 / 237 / 6 / 3 |
| Linked command receipts / unlinked commands | 14 / 5,581 |
| Median judge wall time | 369 s |
| Judge share of session tokens | 3.4 percent |
| Fresh-verifier headline verdicts (n 140): accept / reject / none | 81 / 22 / 37 |
| Of 22 rejections (provisional, excerpt-based): valid acted / valid deferred / false positive / undetermined | 13 / 3 / 0 / 6; receipt-backed 2 |
| Accepts with no command run (the risk cell) | 19 |

Readings agreed: the verifier is not the fold target, its rejections were right where determinable; the gap is unsupported accepts and uncited commands. Follow-through and linkage numbers are indicative because matching is by timestamp.

After-cohort rules: sessions started after `2026-10-28T12:08:18Z` on a restarted Claude Code whose loaded `fresh-verifier.md` hashes to `826287f5…`. Exclude session `72bf98f4-4078-4efc-8359-058cb2302a19` (resumed across the cutoff) or filter by dispatch timestamp. Jev policy must stay unchanged during the cohort. Success: compliant headers dominate, zero passes without a linked receipt, UNVERIFIED reasons listed, interruptions reported separately, verifier token share reported with absolute tokens, latency and retries.

Exact commands (run from the lab root; Python 3.14; outputs must be outside the corpus):

```powershell
cd C:\Users\rakovnik\.claude\robert
python harness-measurement\scripts\analyse2.py "C:\Users\rakovnik\.claude\projects" "C:\temp\after-cohort\out" --since 2026-10-28T12:08:18Z --newest 200
# positional: corpus, out; options: --since <ISO UTC ending Z>, --newest N, --types a,b,c
# headline verdict pass over an existing verdicts.csv:
python harness-measurement\scripts\headline.py <source verdicts.csv> <output.csv> <output.json>
# earlier pass (per-dispatch census; --count N):
python harness-measurement\scripts\analyse.py "C:\Users\rakovnik\.claude\projects" "C:\temp\after-cohort\out1" --count 200
```

`C:\temp` is cleared by the owner; copy results into the lab's `harness-measurement/evidence/` with an `after-` prefix.

## 7. Jev routing: epoch 1 findings, fingerprints, limitations, epoch 2

The action plan `Claude-Jev-Routing-Action-Plan.md` assumed Jev was not installed and proposed piloting BillionsBobby/JevRouter. That premise was wrong and is corrected: a home-grown TypeSafe Jev adapter exists at `C:\Users\rakovnik\.agents\jev` (not a git repository; transport `jev_core` 1.1.0, README stale at 1.0.1), wired as the Claude Code `UserPromptSubmit` hook (`hooks\prompt_hook.py --harness claude`, 10 s timeout) and into Codex, Cursor, Grok and OMP via `policies\<harness>.yaml`. No second router is to be installed.

Epoch 1 (current production configuration; fingerprints in the lab `jev-pilot/experiment.json`):

| Fact | Value |
|---|---|
| Mode | `policies\claude.yaml`: `mode: advisory`, `allow_live: true`; key env present in the Claude process; about 741 ms per prompt |
| Payload | state contains only `request`; code blocks stripped; cut to 600 **characters** (611 bytes for the fixture) at `jev_router.py:448`, before serialisation (`jev_router.py:778` precedes `jev_core\transport.py:309`); receipts log `input_len` raw and `sent_len` sent. Proven with a stubbed transport (`jev-pilot/scripts/payload_trace.py`, exit 0). Not a live transmission test. |
| Gate | usable requires confidence ≥ 0.65 and selected-option probability ≥ 0.35 (`jev_router.py:255`; policy lines 52 to 53). Behaves as an abstention rule, consistent with TypeSafe skill v0.6.2 lines 191 to 196. **Unchanged and not validated.** |
| Telemetry, persona route, all time (n 4,375) | abstained at gate 3,228 (74 percent); explicit none 604; recommended 258; table-resolved 282; provider failures 3 |
| Follow-through on 258 recommendations | matched 143: followed 10, diverged 63, not followed 70; unmatched 115 (unobservable) |
| Roster | persona registry loads `~/.claude/agents` only (policy line 24 to 29, six exclusions). The workers actually dispatched after a "none" (opus-builder 146, fresh-verifier 107, sonnet-worker 92, general-purpose 76) are plugin or built-in agents outside that roster. |
| Receipts | `C:\Users\rakovnik\.agents\jev\telemetry\claude\shadow.jsonl`, identifier-only, no model id recorded |
| Fingerprints (epoch 1) | `jev_router.py` `ed03314d22863ebd`; `policies\claude.yaml` `b6702727e87dd56d`; `policies\_common.yaml` `8a1198bb2d8a37ff`; `jev_core\transport.py` `332581fb3c0197f2`; roster of 26 agent names `ed0014491386c782`; resolved provider model: unknown |
| Open engine defect | `jev_router.py:658`: an unusable skill rerank falls to the default and reports `usable=true`, so abstention is indistinguishable from an affirmative none |
| Known failing test | `test_harnesses.py:349` (run via `test_jev.py`): pins the `impeccable` skill description, changed 2026-10-22; unrelated to this project. **The full engine suite has not been run** (pytest absent; one test executed directly). |
| Manifest mismatch | `check-cases` reports a policy SHA-256 mismatch between the live `policies\claude.yaml` and `eval\claude\MANIFEST.json`: the frozen v3-e1 evaluation describes an earlier policy. Original manifest preserved. |

Epoch 2 (deferred): extend the persona registry so the candidate set includes the workers this host can actually call, nothing else; mandatory verification stays outside optional delegation; new fingerprint and a new eval manifest; receipts labelled separately. Conditions: only after the brief-after cohort is measured; compared with epoch 1 as a changed-policy experiment, never as equivalent cohorts. The 74 percent abstention is measured; its cause (missing candidates versus the 600-character request-only state) is to be tested separately, not assumed.

## 8. Decisions already made (do not reopen)

1. Product definition and build order as in section 1; completion control first, delegation advisory.
2. No second router, no SDK, no replacement adapter; the Python Jev engine is reused if routing is needed.
3. Adapters declare their mode on the first line; enforcement claims are scoped to proven paths.
4. The lab stays where it is; this repository references it by path and hash.
5. The revised agent briefs stay as applied; model and effort unchanged until the after-cohort is read.
6. The dual gate in Jev stays unchanged; alternatives are evaluated only on the pre-registered dev split after the roster fix.
7. Per-accepted-task cost, failed attempts in the numerator, four repetitions or a cross-variant table before attributing a flip; these are the measurement rules for every comparison.
8. Terminal Commander for every long command; the verdict header for every judge.
9. Naming and GitHub location are decided: `coding-supervisor` at `https://github.com/special-place-ai-heaven/coding-supervisor`, local `E:\project\coding-supervisor`.

## 9. Verification commands for the next session

```powershell
# this repository
git -C E:\project\coding-supervisor log --oneline -3
# the lab is reachable and unchanged
git -C C:\Users\rakovnik\.claude\robert log --oneline -1          # expect 7829ebe
git -C C:\Users\rakovnik\.claude\robert status --short            # expect empty
# the authoritative coding package matches its manifest
cd C:\Users\rakovnik\.claude\robert\Agentic-Coding-Research-2026-10-28
python -c "import json,hashlib,os; m=json.load(open('evidence/package-manifest.json')); bad=[e['path'] for e in m if hashlib.sha256(open(e['path'],'rb').read()).hexdigest()!=e['sha256']]; print('manifest entries',len(m),'mismatches',bad)"
# the live verifier brief is the epoch-1 one
Get-FileHash C:\Users\rakovnik\.claude\plugins\cache\fable-orchestration\fable-orchestration\1.0.0\agents\fresh-verifier.md -Algorithm SHA256   # expect 826287F5...
# Jev engine fingerprint
Get-FileHash C:\Users\rakovnik\.agents\jev\policies\claude.yaml -Algorithm SHA256   # expect B6702727...
```

## 10. What the next session should do first

Read `CLAUDE.md`, then `docs/research/2026-10-28-completion-gate-first-slice/PLAN.md`, run section 9, then start PLAN T1 and T2 (core records, state machine, scenario tests with fake providers). Do not touch the live harness, the Jev policy, the agent briefs or the running cohort. Delegate execution to Codex workers in background mode with Terminal Commander for any long command; keep the chair for decisions and verification.
