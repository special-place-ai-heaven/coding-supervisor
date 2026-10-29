# T6 results: the gate's own records as the measurement source

Date: 2026-10-29. Status: journal format v1 and the `measure` reader are built and tested; the tables below are gate-derived from the fixture runs. No core change; the adapter's hook decisions are unchanged, only the content of its journal line grew. Local commits, no push.

## 1. The record: journal format v1

One line per hook call, appended to `events.jsonl` in the gate's state directory (outside the project tree when the environment allows, see T4-T5-RESULTS.md). Fields:

| Field | Content |
|---|---|
| `v` | 1 |
| `t` | unix seconds |
| `session_id`, `prompt_id` | from the hook payload |
| `task_id` | the contract id, or null when the project has no contract (advisory mode) |
| `state_location` | which rule chose the state directory: `env`, `local-app-data`, `xdg`, `home`, `cwd-fallback` |
| `candidate` | `{base_commit, dirty_digest}` of the candidate observed on this call, or null |
| `event`, `tool_name`, `agent_id`, `agent_type` | from the payload (`agent_id` is `main` for the chair) |
| `decision` | `{kind: none \| block \| deny \| context \| system, text}` exactly as emitted on stdout |
| `reason` | the primary reason: the reason part of a refusal, else the first journaled reason |
| `reasons` | the full list the adapter journaled for the call |
| `receipts` | `{recorded: [criterion ids], rejected: [ids], unlinked: [ids]}` for a SubagentStop; empty lists otherwise |
| `header` | `{verdict, checks_run, lines, consistent}` parsed from the worker's handback on SubagentStop, else null |
| `gate_state` | the gate state after the call |
| `accepted` | true only on the Stop call that moved the task to `completed` |
| `tampered` | true when the chain check failed on this call |
| `usage` | on a foreground `PostToolUse(Agent)`: `{total_tokens, input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens, duration_ms, model}` from the tool response; else null |

What is deliberately absent: prompt text, the worker's message, tool inputs other than the CHECK lines already quoted in reasons, any credential. Hook payloads carry no USD cost, so the journal has none.

## 2. The reader: `supervisor-claude-code measure <file or directory>...`

Reads every `events.jsonl`, `*.adapter-events.jsonl` and `*.events.jsonl` under the given paths. Three line shapes are recognised and reported as separate columns, never summed:

| Source | Lines | Where it comes from |
|---|---|---|
| `gate-v1` | `"v": 1` | the adapter from commit `e17f922` on |
| `gate-legacy` | `event`, `decision`, `reasons`, no `v` | adapter journals from runs 1 to 4 (facts derived from the reason strings) |
| `t3-capture` | `payload` and `event` | the T3 python capture hook (no receipts, no gate state) |

Measures, each with its denominator in the cell and every row prefixed `gate-derived`, mapped to what the lab's `analyse2.py` derives from transcripts:

| Measure | Gate definition | Transcript counterpart in `analyse2.py` |
|---|---|---|
| sessions | distinct `session_id` (legacy and T3: one per file) | selected sessions |
| judge dispatches, by agent type | `SubagentStop` events | Agent tool dispatches of the judge types |
| compliant headers | SubagentStop whose last message parses as `VERDICT:` then `CHECKS RUN:` | completed judge files with the compliant two-line header |
| PASS supported / unsupported / FAIL / UNVERIFIED / no header | PASS is supported when every cited line was recorded, nothing was rejected or unlinked, and no refusal or UNVERIFIED release followed | fresh-verifier headline verdicts, and the accept-with-no-command-run risk cell |
| linked receipts | recorded / (recorded + rejected while linked + unlinked) | linked command receipts versus unlinked commands |
| refusals by reason | block and deny decisions classified by the fixed refusal sentence first (tampered, state path, no header, terminal state, unobserved), then by the evidence problem in the gate's order (inconsistent, unlinked, out of scope, failing, stale, missing), else other | rejection categories from the adjudication sheet |
| tamper refusals | block or deny whose text contains `tampered` | none |
| accepted tasks | `accepted` true (v1) or reason `completed` (legacy) over sessions | accept verdicts |
| worker tokens per accepted task | sum of `usage.total_tokens` / accepted tasks, with the number of lines carrying usage; the figure is the worker's final request only | per-accepted-task cost from transcript usage |

`--json` prints the same numbers as an object. Unreadable files are listed on stderr and skipped; exit 1 only when nothing could be read.

## 3. Gate-derived tables from the fixture journals

Sources: the T3 python captures (`fixtures/claude-code-enforcement/captures/*.events.jsonl`), the adapter journals of T4 run 3 (`captures/run3`), the tamper run (`captures/run4`), and run 5 (`captures/run5`), which is run 3's scenario set replayed on the v1 journal. Each column is one source; nothing is combined across columns, and nothing here is a transcript number.

Command: `supervisor-claude-code measure captures/<the eight T3 capture files> captures/run3 captures/run4 captures/run5`. Files read 25, unrecognized lines 0, duplicate lines dropped 21 (the L2b snapshots repeat their L2 session's journal; the reader counts byte-identical lines once per source and says so). The `gate-legacy` column is run 3 (seven files) plus the tamper run 4 (three files) together; `gate-v1` is run 5 (seven files, six sessions because L2 and L2b share one session id); `t3-capture` is the eight T3 sessions.

| measure | gate-legacy (runs 3 and 4) | gate-v1 (run 5) | t3-capture (T3) |
|---|---|---|---|
| gate-derived lines | 71 | 51 | 53 |
| gate-derived sessions | 10 | 6 | 8 |
| gate-derived judge dispatches (SubagentStop) | 13 | 10 | 7 |
| gate-derived dispatches, agent_type cited-worker | 0/13 | 4/10 | 0/7 |
| gate-derived dispatches, agent_type fake-worker | 0/13 | 4/10 | 7/7 |
| gate-derived dispatches, agent_type lying-worker | 0/13 | 2/10 | 0/7 |
| gate-derived dispatches, agent_type unknown | 13/13 | 0/10 | 0/7 |
| gate-derived compliant headers | 12/13 | 9/10 | 7/7 |
| gate-derived PASS supported | 5/13 | 4/10 | 0/7 |
| gate-derived PASS unsupported | 6/13 | 5/10 | 0/7 |
| gate-derived PASS, support not observable (no receipts in source) | 0/13 | 0/10 | 7/7 |
| gate-derived FAIL | 0/13 | 0/10 | 0/7 |
| gate-derived UNVERIFIED | 0/13 | 0/10 | 0/7 |
| gate-derived not PASS, verdict not journaled (legacy) | 1/13 | 0/10 | 0/7 |
| gate-derived no header | 1/13 | 1/10 | 0/7 |
| gate-derived linked receipts | 5/7 | 4/5 | 0/0 |
| gate-derived refusals (block or deny) | 13 | 7 | 10 |
| gate-derived refusals: missing | 4/13 | 4/7 | 0/10 |
| gate-derived refusals: stale | 1/13 | 1/7 | 1/10 |
| gate-derived refusals: unlinked | 2/13 | 1/7 | 0/10 |
| gate-derived refusals: tampered | 1/13 | 0/7 | 0/10 |
| gate-derived refusals: state_path | 4/13 | 0/7 | 0/10 |
| gate-derived refusals: terminal_state | 1/13 | 1/7 | 0/10 |
| gate-derived refusals: failing, out_of_scope, no_header, inconsistent, unobserved | 0/13 each | 0/7 each | 0/10 each |
| gate-derived refusals: other | 0/13 | 0/7 | 9/10 |
| gate-derived tamper refusals | 1/13 | 0/7 | 0/10 |
| gate-derived accepted tasks | 3/10 | 3/6 | 0/8 |
| gate-derived worker tokens (final request only) | n/a (0 usage lines) | 47350/3 = 15783 (5 usage lines) | n/a (5 usage lines) |

Reading the columns:

- Run 5 is run 3's scenario set replayed on the v1 journal, and the two agree: three accepted tasks (the cited worker's positive path, the stale-candidate recovery, the background dispatch), four refusals for a missing receipt, one for a stale one, one for an unlinked citation, one write denied after completion. Run 5 also knows the worker type per dispatch and the worker's final-request tokens, which the legacy lines did not carry.
- The legacy column adds the tamper run: four state-path denials and one tamper refusal, zero accepted tasks in those three sessions.
- The T3 column shows what a capture without a gate can and cannot say: seven compliant headers, seven passes whose support is not observable because no receipt existed, nine refusals in a wording the categories do not recognise (the throwaway handler's), five usage lines but no accepted task to divide by.
- Each "unknown" agent type in the legacy column is the same fake, cited or lying worker; the legacy line simply did not record the type.
- The token figure is the worker's final request only, summed over the source and divided by accepted tasks there; it is a gate-derived lower bound per task, not a session cost and not USD.

Run 5 sessions: L1 `9dc3cdad…`, L2 and L2b `667cff1f…`, L3 `deb9f38c…`, L4 `bb9d2fb0…`, L5 `0631f42e…`, L6 `e91f1fb9…`; all exit 0; state location `local-app-data` on every line; list cost of the seven sessions 0.48 USD (from the `-p` result JSON, which is a transcript-side number and is not in the table).

## 4. What the gate can measure that transcripts cannot, and the reverse

Gate only:

- Whether a cited check is **linked** to a command the harness observed in the same worker, by agent id, command and exit code. The transcript heuristics match by timestamp and reported 14 linked receipts against 5,581 unlinked commands in the before-cohort; the gate records the link or its absence per cited line.
- The **candidate identity** a receipt was bound to and whether it was **stale** at the moment of a completion claim.
- The **decision** the gate made and the **reason**, in a closed vocabulary, including tamper refusals and state-path denials that leave no trace a transcript parser would classify.
- Background-dispatch handbacks, which never appear as a tool result in the main transcript; the gate sees them at SubagentStop.
- Whether a completion claim was **accepted** by rule, rather than inferred from the chair's wording.

Transcript only:

- Anything about sessions that run **without** the adapter: the whole before-cohort and any after-cohort session where the hooks were not configured.
- The chair's **reasoning and wording** around a result, and the user's prompts; the gate journals neither by design.
- **USD cost** and the full per-request token series; hook payloads expose only the worker's final request, so the gate's token figure is a lower bound for one request, not a session cost.
- Judge dispatches by name for agents the gate never saw stop (interrupted sessions, worker files missing), which the transcript census counts as dispatched.
- Wall time of a judge across its whole run, which transcripts carry per message.

Numbers from the two sources therefore answer different questions and are kept in separate tables. A comparison between them is a cross-variant table with its own denominators, never a merge.

## 5. Housekeeping

- The after-cohort measurement must exclude every transcript under the fixture project path `C:\Users\rakovnik\.claude\projects\E--project-coding-supervisor-fixtures-claude-code-enforcement\`; the lab's `analyse2.py` selects transcripts with a one-level glob under the corpus root and has no exclusion option, so copy the corpus without that directory or drop those rows by transcript path afterwards, and record which. Written into `HANDOVER.md` section 6 (the lab repository itself has no handover document; its measurement procedure lives there).
- Gate state files written before the hash chain existed (any `gate.json` without `checkpoint_hash`) read as tampered and must be deleted by hand before the adapter is used on a real project. Written into `HANDOVER.md` section 6.

## 6. Verification (chair, Terminal Commander, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, rustc 1.99.1)

| Command | Exit | Result |
|---|---|---|
| `cargo test --workspace` | 0 | core unit 0; `scenarios.rs` 33; `measure.rs` 4 (three synthetic journals, the committed run3 journals with the deduplicated numbers, a duplicate-line case); `parsers.rs` 8; `protection.rs` 20; `replay.rs` 19 |
| `cargo fmt --all --check` | 0 | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | clean |
| `cargo tree -p coding-supervisor -e normal,build,dev --target all` | 0 | one line, the core only |
| `cargo tree -p supervisor-claude-code -e normal,build,dev --target all --depth 1` | 0 | the core and `serde_json` |
| `supervisor-claude-code measure …` over the four sources | 0 | the table above |

Worker: the same Opus 5.5 lane that built the adapter, two turns (journal and reader, then the duplicate-line correction); the chair re-ran every check, ran the live sessions, read the journals and wrote this file.

Known limits of the reader: refusal categories are substring matches on the gate's own wording, so a changed sentence moves a refusal to `other`; a legacy session is a file, so a resumed session counts twice there; two genuinely distinct events that are byte-identical would be counted once; on lines without a refusal the primary `reason` falls back to the first journaled note, which is the replay summary.
