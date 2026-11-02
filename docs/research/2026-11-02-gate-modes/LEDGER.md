# Ledger: 0.5.1 field issues and gate modes (2026-11-02)

One ledger for the field report (`C:\Users\rakovnik\Downloads\CSUP-ISSUES-REPORT-2026-11-02.md`, from the
TestPilot runs on 0.5.1) and the agreed modes (`PROPOSAL.md` beside this file). Owner's rulings of
2026-11-02 are folded in. Root causes F10, F11, F12 and F16 were checked against `cb86835` (= v0.5.1);
the others are the report's reading of the source until their row is worked.

Criticality:
- **P0**: blocks a supervised task outright, or burns it with no recovery. Both TestPilot runs hit these.
- **P1**: the gate gives a wrong answer, or real use needs an operator workaround.
- **P2**: misleading output or a missing guard rail; a person can work around it.
- **P3**: polish, docs.

Status: `todo`, `doing`, `done (<commit>)`, `dropped (<why>)`.

## Wave 1: patch 0.5.2, unblock the pre-intent gate (F11)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| F11a | Stop and SubagentStop with a pending prompt release with a system message when the harness is already continuing (`stop_hook_active`), as the tamper path does; no more 9-block override. | P0 | – | done (e8132b3) |
| F11b | Before the prompt is classified, allow `ToolSearch` and a narrow set of single read-only commands (`git status/log/diff/show/rev-parse`, `git branch --show-current`, `ls`, `pwd`, `supervisor status`, `supervisor --help`; no pipes, redirects, chaining or substitution; no `--output`). Guard and policy still apply. | P0 | – | done (e8132b3) |
| F11c | An unknown `intent_apply` action is refused as unknown, with the seven valid actions, before any other check. | P1 | – | done (e8132b3) |
| F11d | The UserPromptSubmit context names the valid actions and gives the ready-to-copy next call (`continue` with the task id when a task is active; `start` with a contract, or `discussion`, when not). | P1 | – | done (e8132b3) |
| F11e | Every PreToolUse denial and Stop block is journaled to `events.jsonl` with its reason. | P2 | – | todo |

## Wave 2: patch 0.5.3, repair rounds and background runs (F12, F16)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| F12a | A Stop refusal spends a repair round only when the candidate was judged wrong: a required check's latest observed run on it failed, a check's npm entrypoint was redefined, a finding is open, or the diff is out of scope. A missing or stale receipt refuses without spending one and says so. | P0 | – | done (wave 2 commit) |
| F12b | A contract criterion's command launched in the background is denied at PreToolUse with "run it in the foreground; background runs are never counted". | P0 | – | done (wave 2 commit) |
| F12c | `continue` on an operator-started task says it converted it (`converted_to_autonomous` plus a note naming what changes). Chosen over keeping it operator-owned, which would need a second session-binding path. | P1 | F12a | done (wave 2 commit) |
| F12d | `supervisor task reset-rounds --task <id> --reason <text>`: a person's command, journaled and hash-chained, moves `budget_exhausted` to `repairing`. The PreToolUse guard denies it from an agent's shell, as it denies `supervisor level` (owner's ruling). | P1 | F12a | done (wave 2 commit) |
| F17a | The "immutable repository requirement floor" refusal names the criterion and shows the floor's command beside the proposed one, and points at `amend-floor` for a stored floor. | P2 | – | done (wave 2 commit) |
| F17b | `supervisor task amend-floor --criterion <id> --command <new> --reason <text>`: a person's command, recorded in the stored floor's `amendments` with old and new command, guard-denied to agents like F12d. Works only while no task is active and only on the stored floor (a repository whose files yield checks has none to amend). Chosen over a list of flags that cannot weaken a run, which would be per-runner and fragile. Field case: adding `--workerIdleMemoryLimit=1536M` to a jest command was refused at `start` after a cancel. | P1 | F12d (same guard and journal) | done (wave 2 commit) |
| F16 | Refusal counter never shows `n of 3` above 3; the cap is per turn and the message says so; no hardcoded "harness cap 8". The fourth stop in a turn is released whether or not the harness is already continuing. | P2 | F12a (same Stop code) | done (wave 2 commit) |

## Wave 2b: patch 0.5.4, one repair round per version of the code

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| F12e | A repair round is charged at most once per candidate: after a round is charged on a version of the code, further refusals on that same version spend nothing until the code changes. Owner, 2026-11-02: "we cant have normal work hitting budgets like this". Field case: a TestPilot run on 0.5.1 hit its budget before recording its last two checks. | P0 | F12a | done (wave 2b commit) |

**Gate R1 (owner):** after 0.5.2 and 0.5.3 publish, JEZ installs them and reruns the faces fix and Part 1 of
TestPilot on fresh tasks with no workarounds in the prompts. Before the rerun, `supervisor status --json`
must show the new version in `reporting_binary.version` and in the configured hook path (the report says
configs hold a versioned binary path; unverified here, see X4).

## Wave 3: patch, monorepo discovery (F10)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| F10a | Discover npm checks in direct subfolders (skip `node_modules` and hidden folders) when the root has no package.json, as `npm --prefix <dir> run <script>` with ids like `AUTO_NPM_TEST_BACKEND`; `npm_definitions` (`<dir>/<script>`) and entry points learn the `--prefix` form. Owner, 2026-11-02: discovered commands are amendable by a person through `task amend-floor` (stored as `floor_overrides`), so a heavy suite can carry `--maxWorkers=2`. Workspace globs and deeper folders are not read yet. | P1 | – | done (wave 3 commit) |
| F10b | "No checks found" becomes final: the error says `not retryable` and tells the model to stop and tell the user. It does not name the operator-contract path, because `task start` is refused once a prompt has been registered (see K13). | P1 | F11c (same error style) | done (wave 3 commit) |
| K4 | `init` drafts checks from the same discovery, not a Rust draft in an npm repo. | P2 | F10a | todo |

## Wave 4: minor 0.6.0, modes (from PROPOSAL.md)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| M1 | Modes `off`, `audit`, `enforce`, `strict`; `lite`, `full`, `ultra` stay accepted as aliases; journals and status print the new names. | P1 | Wave 2 (Stop code settles first) | todo |
| M2 | Requested vs effective mode: a mode the harness cannot enforce runs as `audit` and says so in status and every journal line. Absorbs F15 (MCP answers say `answer_authority`, task says its enforcement). | P1 | M1 | todo |
| M3 | `/supervisor <mode>` and `/supervisor status` in the plugin, user-invoked only. | P1 | M1 | todo |
| M4 | One-step plugin setup: the plugin also starts the MCP server. First find why `1ea1aad` left it out ("F1"); drop this row if the reason holds. | P2 | M3 | todo |
| F13b | `task start` warns that `forbidden_effects` are not enforced (owner's ruling: LINUS checks them in review until M5). | P2 | – | todo |
| K2 | MCP `accept_try` and `verdict` must not disagree: asserted-only receipts cannot reach `ready_for_review` where `verdict` would fail. | P1 | M2 | todo |

## Wave 5: minor 0.7.0, what `strict` adds

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| K6 | A failing-before-fix receipt binds to the task's starting candidate and stays valid after the fix is committed. | P1 | – | todo |
| M5 | `strict`: typed `forbidden_effects` (`deny_paths`, `deny_git`, `deny_argv_prefix`) compiled into PreToolUse denials for the active task (F13a). | P1 | M1, F13b | todo |
| M6 | `strict`: optional per-criterion `min_tests`; a run satisfies it only when the parsed jest/cargo/pytest summary shows enough passed tests (F14). Owner: LINUS confirms counts until then. | P1 | M1 | todo |
| M7 | `strict`: a bug fix needs the reported failure observed failing on the starting candidate, then passing on the current one (C1 then C2). How a task is marked as a bug fix is open. | P1 | M1, K6 | todo |
| M8 | Live Claude Code run of every mode with only the plugin installed. | P1 | M1–M7 | todo |

## Unscheduled (P2 and P3)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| X4 | Generated configs hold absolute binary paths; check whether an npm upgrade leaves hooks on the old binary. `install.rs:189` records `current_exe()`. | P2 | – (needed for gate R1) | todo |
| K13 | `supervisor task start` is refused once `intents.json` exists, so a person cannot supply an operator contract after the first prompt. Found while writing F10b. | P1 | – | todo |
| K3 | `install --dry-run` reports writes into the checkout, including a tracked file. | P2 | – | todo |
| K9 | State location differs with and without `SUPERVISOR_STATE_DIR` (`env` vs `home` rule), so agents see different task namespaces. | P2 | – | todo |
| K7 | `task start` prints `queued` while `task_state` says `preparing`. | P3 | – | todo |
| K8 | `recorded_at` truncated to seconds; README promises milliseconds. | P3 | – | todo |
| K5 | `init` without `--output` writes into the repo root. | P3 | – | todo |
| K11 | `supervisor --help` does not mention the MCP tools or the receipt sequence. | P3 | – | todo |
| K12 | README Install still says 0.1.1; also says the autonomous workflow is unreleased (shipped in 0.2.0). | P3 | – | todo |
| X1 | `supervisor --version`. | P3 | – | todo |
| X2 | Alias bin `coding-supervisor` or a clearer install message. | P3 | – | todo |
| X3 | `verdict` outside a git repo prints a git error for `no_active_task`. | P3 | – | todo |
| X5 | Codex trust step ignores `CODEX_HOME`. | P3 | – | todo |
| X6 | README: criteria design advice from the report's environment note (no parallel heavy checks, cgroup over `prlimit --as`, `jest <files> -t`). | P3 | – | todo |
| X7 | State backup before upgrade. | P3 | – | todo |
| K1 | No non-Claude path yields an observed receipt. Design limit of MCP; recorded, no fix planned. | – | – | dropped (by design) |

## Housekeeping

| ID | Item | Depends on | Status |
|---|---|---|---|
| H1 | Archive PR #2 (`feat/autonomous-gate`, head `120a0ed`): push tag `archive/autonomous-gate`, close with a note pointing here. Owner approved 2026-11-02. | – | done (tag pushed, #2 closed) |
| H2 | Refresh stale lines in `CLAUDE.md` (branch `phase-a-installable`, "uncommitted" work now on main). | Wave 1 | todo |
