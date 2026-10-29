# First real-project use: the lab repository, chair working alone

Mode of the run: Claude Code hook adapter (enforcing on PreToolUse, Stop, SubagentStop) plus the advisory MCP server, installed configuration, Claude Code 2.1.312, 2026-10-29. One chair session (`5326800e-…`), no subagent. Project: `C:\Users\rakovnik\.claude\robert`, task `robert`.

Evidence: the gate's own journal in `C:\Users\rakovnik\AppData\Local\coding-supervisor\state\C--Users-rakovnik--claude-robert\`, read only, not modified:

| File | Lines | Cited here as |
|---|---|---|
| `events.jsonl` | 104 | `#n` = line n |
| `runs.jsonl` | 26 | `rn` = line n |
| `contract.json` | | C1 `python -I jev-pilot/scripts/test_j_checks.py` (optional), C2 `python -m py_compile harness-measurement/scripts/analyse2.py` (mandatory), C3 `python -m compileall -q harness-measurement/scripts jev-pilot/scripts` (mandatory), C4 `git diff --name-only` (optional) |

The journal stores identifiers, decisions and reasons, never message text or command output (journal format v1). Where a finding depends on what a message said or what a command printed, that is stated as not shown by the journal.

Read with two scripts in `C:\temp` (`journal-summary.py`, `runs-summary.py`) that print each line's index, event, decision and reasons.

## Findings

### 1. Stop never reads the CHECK header

Code: `adapters/claude-code/src/adapter.rs`, `Session::stop` checks `claim::claims_completion` and then `try_accept`/`complete` or `observed_evidence`. `header::parse` is called from `Session::subagent_stop` only (and from `claim` to detect a `VERDICT: PASS`). A chair that works alone therefore has no path that turns its own observed runs into receipts. Its only route is MCP receipts, and those are caller-asserted and do not count at Stop.

| Line | Evidence |
|---|---|
| r23 to r26 | `agent main`, bare contract commands, exit 0: `python -I jev-pilot/scripts/test_j_checks.py`, `python -m py_compile harness-measurement/scripts/analyse2.py`, `python -m compileall -q harness-measurement/scripts jev-pilot/scripts`, `git diff --name-only` (t 1793274799 to 807) |
| #91 to #97 | `candidate_observe`, four `receipt_record` (each `recorded … (mcp-asserted)`), `accept_try` to `ready_for_review`, `task_complete` to `completed` |
| #98 | `Stop`, decision `block`: "GATE refuses completion (refusal 1 of 3, harness cap 8): state completed; missing C2; record the check with CHECK lines or report the task as not complete." Reasons: `unlinked CHECK C1..C4 … (mcp-asserted receipt; no hook observed the run)`, `missing C2`. `header: null` |
| #102, #103 | Same refusal text (refusal 1 of 3, then 2 of 3) in the next turn; `header: null` |

Three Stop refusals in two turns (#98; #102 and #103), not two. The hook had journaled all four bare runs before #98, yet the refusal names C2 missing. The refusal asks for CHECK lines that this path never reads. `header` is `null` on every Stop line by construction, because only SubagentStop fills it. The journal therefore cannot show whether the final message carried CHECK lines. That it carried four valid ones is the owner's observation of the session. The code shows that they would have been ignored either way.

### 2. `receipt_record` field names unknown to the chair

12 refusals, in three rounds of four, one field at a time: the chair skill names the tool but none of its six fields (`criterion`, `command`, `cwd`, `exit_code`, `output_ref`, `claim_supported`).

| Lines | Refusal text |
|---|---|
| #40 to #43 | "GATE (advisory): receipt_record refused: missing field criterion" |
| #44 to #47 | "… missing field output_ref" |
| #50 to #53 | "… missing field claim_supported" |

### 3. Receipts recorded before the candidate was observed

| Lines | Evidence |
|---|---|
| #54 to #57 | "receipt_record refused: candidate identity not observed"; reasons `rejected CHECK C1..C4 … candidate identity not observed (mcp-asserted)` |
| #58 | `candidate_observe` |
| #59 to #62 | the same four receipts `recorded` |

The skill lists observe before record but does not say the order is required. #37 had observed a candidate, but the hook calls on the runs at #38/#39 and #48/#49 then saved a newer candidate to `gate.json` ("candidate changed since the checkpoint"). The MCP server trusts its kept candidate (`mcp-candidate.json`) only while `gate.json` still holds it, so at #54 no candidate counted as observed. The working order is to run the commands, then call `candidate_observe`, then record, with no command in between.

### 4. `task_start` refused twice on a hooked harness

| Line | Refusal text |
|---|---|
| #6 | "GATE (advisory): task_start refused: missing field criteria" |
| #14 | "… task_start refused: the state directory already holds the contract of task robert; that task continues" |

By #1 the hooks had already created the task from `contract.json` (`task_id robert`, `gate_state verifying`). The chair skill tells every chair to call `task_start`.

### 5. The chair cannot read `contract.json`, and `task_state` does not return it

| Line | Evidence |
|---|---|
| r1 | `find . -maxdepth 4 -name contract.json …`, exit 0 (searched the project tree) |
| #4 | PreToolUse `Bash` denied: "GATE: Bash refused: the gate's own state is not writable by agents (.supervisor)" |
| #5 | `mcp:task_state`: answers state and receipts, no criteria |
| #9 | PreToolUse `PowerShell` denied with the same text |
| r3 | `C:/tools/supervisor/supervisor.exe --help …; … mcp --help …` |
| r4 | `find "$LOCALAPPDATA" -maxdepth 4 -iname 'contract.json' … \| grep -i superv` |
| #14 | `task_start` with criteria (the contract had been found) |

`guard::state_write_denial` denies any Bash or PowerShell command whose text contains `.supervisor` or the state directory, so reads are denied as well, under a refusal text that says "not writable". The denied commands are not journaled. The chair skill points at `contract.json`. The route that worked was the binary's help and a search under local app data. That the help output is what led the chair there is inferred from the order of r3, r4 and #14, because output is not journaled.

### 6. Side-tool writes move the candidate and trip the scope check

| Line | Refusal text |
|---|---|
| #63 | "GATE (advisory): accept_try refused: out of scope .claude/autoharness/session-5326800e-4536-4454-ac18-fba84199b283, .claude/autoharness/session-5326800e-4536-4454-ac18-fba84199b283.lock" |
| #80 | same text |

r18 (`git status --short --untracked-files=all …; ls -la .claude/autoharness/ …`) is the chair investigating it. The lock is written by a harness plugin, not by the task. The gate's behaviour is correct: the path is in the candidate's changed paths and outside the contract scope. Every plugin that writes into the working tree will do the same. Documented only; no design change in this work.

### 7. Compound commands mask exit codes

| Line | Command (journaled exit) |
|---|---|
| r5 | `python -I jev-pilot/scripts/test_j_checks.py; echo "EXIT=$?"` (exit 0) |
| r9 | `JEV_OFFLINE=1 python -I jev-pilot/scripts/test_j_checks.py 2>&1 \| tail -4; echo "EXIT=${PIPESTATUS[0]}"; …` (exit 0) |
| r16 | `echo "== C1"; python -I jev-pilot/scripts/test_j_checks.py 2>&1 \| grep …; echo "C1_EXIT=${PIPESTATUS[0]}"; …` (exit 0) |

A shell command's journaled exit code is the exit of its last stage (`echo`), so a failing test inside it is journaled as exit 0. That the test failed at r5 is the owner's observation, because output is not journaled. None of these runs could link to C1, and only because its text is not the contract command (gap 1). If a contract named a compound command, the masked exit would link. The chair re-ran the bare commands for its receipts (r19 to r22, r23 to r26).

### 8. Receipts bind to the candidate at recording time (gap 2, observed live)

| Lines | Evidence |
|---|---|
| r23 to r26 | runs at t 1793274799 to 807 |
| #91 | `candidate_observe` at t 812, digest `2482b9a5e2f7…` |
| #92 to #95 | receipts recorded at t 816 to 821, bound to `2482b9a5e2f7…` |
| #96 | `accept_try` to `ready_for_review` |

The gate bound each receipt to the candidate observed when the receipt was recorded, not to the candidate the run saw. In this instance the candidate did not change between the runs and the recording (#82 at t 792 already shows `2482b9a5e2f7…`, and it is unchanged through #104), so nothing stale was accepted. The order runs, then observe, then record is what gap 2 of `docs/research/2026-10-29-receipt-relevance-and-run-time-freshness/PROPOSAL.md` describes. Documented only; no design change in this work.

## What follows

Finding 1 is fixed in the Claude Code adapter (next commit: Stop reads the header). Findings 2 to 5 and 7 are addressed in the skill templates and the MCP `task_state` answer (the commit after). Findings 6 and 8 are documented only.

## Fix for finding 1: Stop reads the header

`adapters/claude-code/src/adapter.rs`. The CHECK-linking loop of `subagent_stop` moved unchanged into `Session::link_checks`, shared by both events. `stop` now parses `last_assistant_message` with `header::parse` and links each CHECK line to a journaled run by the hook's agent (`main` for the chair): same command after trimming, same exit code, not used by an earlier receipt, and the gap-1 check that the command is one the contract names. Linked receipts go to the core. Any problem (an unlinked line, a command the contract does not name, a header whose count disagrees) refuses before acceptance is tried, with the existing Stop wording: `GATE refuses completion (refusal k of 3, harness cap 8): state <s>; <problem>; record the check with CHECK lines or report the task as not complete.`, followed by `Run that command in this session and cite it as CHECK <id> | <command> | exit 0.` when the cited command is the wrong one. Without problems, acceptance runs as before.

The core takes no receipt once a task is `completed`, and the live run completed through MCP before its Stop (#97, #98). For that state `link_checks` keeps a hook-linked receipt for a known criterion in `gate.json` without the core, where `observed_evidence` reads it. An unlinked one is rejected with the core's `receipt is not linked to an observed run`. This also applies at SubagentStop. Core `src/lib.rs` is untouched.

| Replay test (`adapters/claude-code/tests/replay.rs`) | Shows |
|---|---|
| `chair_stop_links_check_lines_to_its_own_runs_and_completes` | four `main` runs, Stop citing them: no block, state `completed`, journal `receipts.recorded` C1..C4 |
| `chair_stop_citing_a_run_that_was_not_journaled_is_refused` | C1 cited, never run: block starting `… state verifying; unlinked CHECK C1 \| cargo test c1 \| exit 0 (no run by agent main with that command and exit code); record the check with CHECK lines`; a worker's run of the same command does not link either |
| `chair_stop_citing_another_command_names_the_contract_command` | `cargo test c1; echo done` cited: block names the contract command and ends with the run-instead instruction |
| `chair_stop_after_mcp_completion_counts_its_own_runs` | the live sequence: completed through MCP on an asserted receipt, then a `main` run and a Stop citing it: no block |

All through Terminal Commander, cwd `E:\project\coding-supervisor`, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`.

| Command | Job | Exit | Evidence |
|---|---|---|---|
| `cargo test -p supervisor-claude-code --test replay chair_stop` on the previous `adapter.rs` | `job_01a18795d394739ea3425b8ce921cbd4` | 101 | the four new tests FAILED, the two existing `chair_stop` matches ok |
| `cargo fmt --all -- --check` | `job_01a187963c1571bf977f0fe0091bb485` | 0 | no diff |
| `cargo clippy --workspace --all-targets -- -D warnings` | `job_01a18796439672a184b2b82e5650e3db` | 0 | no warning or error lines |
| `cargo test --workspace` | `job_01a1879654ff7491bf284b1edf5599f1` | 0 | 128 passed, 0 failed (replay 25 to 29; total 124 to 128) |

Not run: no live Claude Code session has exercised the new Stop path.

## Corrections for findings 2 to 5 and 7: skills and MCP

| File | Change | Finding |
|---|---|---|
| `skills/supervisor-chair.SKILL.template.md` | "Before you say done": read the criteria from `task_state`. Do not call `task_start` where the hooks are installed (the task exists once the contract is in the state directory); call it once on a harness without hooks. Run each command bare. With hooks, end with the CHECK header in the final message instead of MCP receipts. Without hooks, call in order, with no command in between: `candidate_observe`, `receipt_record` per check, `accept_try`, `task_complete`. Names the six `receipt_record` fields. "When you delegate": the table comes from `task_state`; with hooks the gate reads the worker's header itself. | 2, 3, 4, 5, 7 |
| `skills/supervisor-worker.SKILL.template.md` | New rule "The command runs alone": nothing appended, piped or redirected, or the run does not link. | 7 |
| `adapters/mcp/src/tools.json` | `task_start`: do not call it once hooks have created the task. `receipt_record`: call `candidate_observe` first, no command in between. `task_state`: returns the criteria. | 3, 4, 5 |
| `adapters/mcp/src/lib.rs` | `describe` adds `criteria` (`id`, `command` as a list, `mandatory`) to every answer, `task_state` included. This is code outside "templates and tools.json only": a description cannot make `task_state` return the contract. | 5 |
| `adapters/mcp/tests/replay.rs` | `task_state_returns_the_criteria_table` | 5 |
| `cli/tests/install.rs` | for all five harnesses: the chair skill names `` `output_ref` `` and says "do not call `task_start`"; both skills say "nothing appended" | 2, 4, 7 |

Hooks are written for Claude Code only, so the skills tell the two cases apart by naming that ("`supervisor install` writes them for Claude Code only") rather than by per-harness text in `cli/src/install.rs`, which is unchanged.

| Command | Job | Exit | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | `job_01a18799255b732791457494c6efd42a` | 0 | no diff |
| `cargo clippy --workspace --all-targets -- -D warnings` | `job_01a18798c8cc71549719237526230b6c` | 0 | no warning or error lines |
| `cargo test --workspace` | `job_01a18798d84c70c4995cf1362a753f7f` | 0 | 129 passed, 0 failed (MCP replay 14 to 15; total 128 to 129) |

Not run: no chair session has used the corrected skills.
