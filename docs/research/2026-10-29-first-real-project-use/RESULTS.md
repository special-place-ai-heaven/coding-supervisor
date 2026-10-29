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
