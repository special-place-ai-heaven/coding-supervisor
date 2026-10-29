# Gap 1 results: receipts must cite the command the contract names

Implements gap 1 of `PROPOSAL.md` in this folder, on `main` after merge commit `7be06b2`. Gap 2 and the two smaller observations are not touched.

## What changed

| Crate or file | Change |
|---|---|
| core `src/lib.rs` | `Criterion` gains `command: Vec<String>` and `Criterion::accepts(&str)` (exact text after trimming both sides, no normalisation). `Task::record_receipt` returns the new `GateError::WrongCommand { criterion, cited, required }` when the receipt's command is not one of the criterion's commands. Checked after `UnknownCriterion`; state unchanged on refusal. Still dependency-free, `#![forbid(unsafe_code)]`, no panics. |
| adapter `adapters/claude-code/src/state.rs` | `contract_from` requires `command` per criterion: one string or a list of strings. Missing, empty, blank or non-string fails with an error naming the criterion, e.g. `criterion C2 has no command; name the command a receipt for it must cite`. The hook's existing fail-open path reports it as a `systemMessage`. |
| adapter `adapters/claude-code/src/adapter.rs` | Linkage at `SubagentStop` adds a fourth condition: the cited command is accepted by the criterion (same `Criterion::accepts` as the core). A mismatch is not linked to any run, goes to the core with `claim_supported: false`, is journaled as `unlinked CHECK <id> \| <cited> \| exit <n> (claim_supported false: ...)` and therefore appears in the journal's `receipts.unlinked`. `render` covers `WrongCommand`. |
| adapter `adapters/claude-code/src/measure.rs` | New refusal category `wrong_command`, matched on `but the contract requires`, ahead of `unlinked`. |
| MCP `adapters/mcp/src/tools.json` | `task_start` requires `command` per criterion (`string` or non-empty `array` of strings); the `receipt_record` description says the command must be one the contract names. `receipt_record` itself is unchanged and relies on the core's rule. |
| CLI `cli/src/init.rs` | Starting contract carries placeholder commands: C1 `cargo test`, C2 `cargo test`, C3 `cargo test --workspace`, C4 `git diff --name-only`, with a code comment saying to edit them. |
| Contracts | `fixtures/claude-code-enforcement/.supervisor/contract.json` and `fixtures/claude-code-install/contract.json` get `"command": "echo check-ok"` for C2. |
| Docs | `README.md` (contract description, how-it-works sentence, test counts), `CLAUDE.md` (one current-truth line). |

## Refusal wording

Core error rendered by the adapter (and by the MCP server, through the same `render`):

```
C2 cites `echo ok` but the contract requires `echo check-ok`
```

With a list, the first command is named and the others follow after `or one of:`, each in backticks.

Full `SubagentStop` refusal from the replay test:

```
GATE: handback refused (refusal 1 of 3): C2 cites `echo ok` but the contract requires `echo check-ok`; missing C2. Run that command in this session and cite it as CHECK C2 | echo check-ok | exit 0.
```

## Tests added

- `tests/scenarios.rs`: `gap1_receipt_must_cite_a_command_the_contract_names` (wrong command rejected with `WrongCommand`, state unchanged, listed command with surrounding spaces accepted); one row added to `matrix_record_receipt`, marked "specified by PROPOSAL gap 1". Existing test contracts carry `command: ["fake"]`, the string `FakeRunner` already cites.
- `adapters/claude-code/tests/replay.rs`: `replay_gap1_check_citing_another_command_is_refused` (worker runs and cites `echo ok`, C2 requires `echo check-ok`: exact refusal text, journal `receipts.unlinked == ["C2"]` with the `claim_supported false` reason, no receipt in `gate.json`, state `verifying`); `contract_without_a_command_fails_open_naming_the_criterion`. The test contract helper gives each criterion the commands those tests already cite (`cargo test c1`..`c4`, `cargo test`).
- `adapters/claude-code/tests/measure.rs`: `wrong_command_refusals_have_their_own_category`.
- `adapters/mcp/tests/replay.rs`: `gap1_receipt_must_cite_a_command_the_contract_names`; `protocol.rs` gains a `task_start` case with an empty command list. The shared test contract uses `"command": "cargo test"`.
- `cli/tests/init.rs`: asserts the four placeholder commands.

Counts after the change (`cargo test --workspace`): core 34 scenarios; CLI 1 init + 9 install; Claude Code adapter 6 measure + 8 parsers + 20 protection + 25 replay; MCP 1 journal + 6 protocol + 14 replay. Total 124, previously 119.

## Verification

All through Terminal Commander, cwd `E:\project\coding-supervisor`, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`.

| Command | Job | Exit | Evidence |
|---|---|---|---|
| `cargo test --workspace` (before formatting) | `job_01a1875d0cb37366bd6ecd5f47a85e68` | 0 | every `test result: ok`, counts as above |
| `cargo fmt --all -- --check` | `job_01a1875da7c076f59433bf44d2ca8b78` | 1 | `Diff in ...adapters\claude-code\tests\replay.rs:383` (new test) |
| `cargo fmt --all` | `job_01a1875db6ce71de8db8826d3a0b1018` | 0 | changed only the new replay test |
| `cargo clippy --workspace --all-targets -- -D warnings` | `job_01a1875dd26d7335a9ccd160ba288607` | 0 | no warning or error lines |
| `cargo fmt --all -- --check` | `job_01a1875df798756eae47b744134455df` | 0 | no diff |
| `cargo tree -p coding-supervisor --edges normal,build,dev` | `job_01a1875dfe23723b87932dc3f840ce34` | 0 | one line: `coding-supervisor v0.1.0 (E:\project\coding-supervisor)` |
| `cargo test --workspace` (final tree) | `job_01a1875e63987058bb3288d1830b4a14` | 0 | 124 passed, 0 failed |

No live harness run was made; the change is verified by the replay and scenario tests only.

## Not changed, and why

- `fixtures/codex-enforcement/captures/installed/contract.json` and `fixtures/omp-enforcement/captures/installed/contract.json` are captured evidence of past installed runs and are read by nothing; editing them would misstate what those runs used. They would not load under the new format.
- The README status line already says that on every harness except Claude Code the gate is advisory and a chair can assert a receipt for a command it never ran; it was not repeated.
- The README has no JSON contract example; the starting-contract paragraph under "Quickstart" now describes `command` instead.
- The skill templates do not tell a worker which command a criterion requires; the worker learns it from the refusal.

## What gap 1 does not solve

From `PROPOSAL.md`: "A worker can still run the required command in a way that proves nothing, for example by editing the test it is supposed to pass." Gap 2 (a run bound to the candidate at handback rather than at run time) is also still open.

## Follow-up: the skills hand the worker each criterion's command

Closes the last bullet of "Not changed, and why" above: before this change a worker learned a criterion's command only from its first refusal.

| File | Change |
|---|---|
| `skills/supervisor-chair.SKILL.template.md` | "When you delegate" now tells the chair to hand the worker the contract's criteria as a table (id, command, mandatory), taken from the `task_start` contract or `contract.json` in the gate's state directory, with every command of a list-valued criterion, and says the gate refuses any other command. A three-row example table. |
| `skills/supervisor-worker.SKILL.template.md` | The `CHECK` rule says the id and command come from the chair's criteria table, cited verbatim; a different command is refused and links no run; without a table, the first refusal names the required command. |
| `cli/tests/install.rs` | `every_harness_gets_its_documented_files_and_a_second_run_changes_nothing` asserts both rendered skills contain `id, command, mandatory` for all five harnesses. |

Core `src/lib.rs`, both adapters and `cli/src/install.rs` are unchanged: install renders both templates for every harness through the same `render` (`{HARNESS}` and `{MCP_NOTE}` substitution only), so the new text needs no per-harness code.

All through Terminal Commander, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, cwd `E:\project\coding-supervisor` unless stated.

| Command | Job | Exit | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | `job_01a1876da365731b95bd67adc6c9cb6e` | 0 | no diff |
| `cargo clippy --workspace --all-targets -- -D warnings` | `job_01a1876db70c7210b2d4dcdef8aff6b3` | 0 | no warning or error lines |
| `cargo test --workspace` | `job_01a1876dd2b776188fe4e3c178bf928e` | 0 | 124 passed, 0 failed (34 + 1 + 9 + 6 + 8 + 20 + 25 + 1 + 6 + 14); unchanged count, the new assertion sits in an existing test |
| `cargo build --workspace` | `job_01a1876e57cb718dabb876808473cee2` | 0 | rebuilt `supervisor` with the new templates |
| `sh C:/temp/render-check.sh` (cwd `C:\temp`): `supervisor install --harness <h>` into a throwaway git project per harness, then `grep -c "id, command, mandatory"` on both rendered `SKILL.md` | `job_01a1876ea6ed768287c11cf6774dfced` | 0 | count 1 in `supervisor-chair` and `supervisor-worker` for `claude-code` (`.claude/skills`), `codex` (`.agents/skills`), `cursor` (`.cursor/skills`), `omp` (`.omp/skills`), `grok` (`.grok/skills`) |

Not run: no harness loaded the new skills and no chair or worker followed them in a live session; the text reaching each harness's skills directory is what is shown.
