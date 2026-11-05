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

## Work loop (owner, 2026-11-03: "orchestrate the remaining work in the correct order until all is complete")

The remaining rows run as a self-paced `/loop` in this session, dogfooded: 0.5.5 is installed globally and
into this checkout (`supervisor install --harness claude-code --project .`; `.claude/` and `.mcp.json` stay
untracked), so the gate supervises its own development.

- **Row loop** (one row per pass): read the code the row touches; write the test and watch it fail
  (Terminal Commander, `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`); smallest change; watch
  it pass; commit with a conventional subject; set the row to `done (<commit>)`.
- **Wave loop** (one release per wave): fmt, clippy and the workspace tests once on the branch; PR; CI green;
  merge; release-please PR merges; watch `release.yml` and npm `latest`; `npm install -g` the new version
  and keep dogfooding on it.
- **Outcome (2026-11-03)**: waves 3b, 4 and 5 shipped as 0.6.0, 0.7.0 and 0.8.0. Rows still open wait on the
  owner: M7 (confirm the bug-fix default), M8 (live run on the owner's quota), X7 (state backup on upgrade),
  and gate R1 (JEZ reruns TestPilot on the new release).
- **Stop rule**: a row that needs an owner decision is set to `waiting (owner: <question>)` with a
  proposed default and the loop moves on; the loop stops when every row is done, dropped or waiting.

Order: wave 3b (K13, K14, F19, F11e, F13b, K4 with the P2/P3 rows and H2, released together as
0.6.0 because it adds features), wave 4 (0.7.0: M1, M2, K2, M3, M4), wave 5 (0.8.0: K6, M5, M6,
M7, M8).

## Wave 1: patch 0.5.2, unblock the pre-intent gate (F11)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| F11a | Stop and SubagentStop with a pending prompt release with a system message when the harness is already continuing (`stop_hook_active`), as the tamper path does; no more 9-block override. | P0 | – | done (e8132b3) |
| F11b | Before the prompt is classified, allow `ToolSearch` and a narrow set of single read-only commands (`git status/log/diff/show/rev-parse`, `git branch --show-current`, `ls`, `pwd`, `supervisor status`, `supervisor --help`; no pipes, redirects, chaining or substitution; no `--output`). Guard and policy still apply. | P0 | – | done (e8132b3) |
| F11c | An unknown `intent_apply` action is refused as unknown, with the seven valid actions, before any other check. | P1 | – | done (e8132b3) |
| F11d | The UserPromptSubmit context names the valid actions and gives the ready-to-copy next call (`continue` with the task id when a task is active; `start` with a contract, or `discussion`, when not). | P1 | – | done (e8132b3) |
| F11e | Every PreToolUse denial and Stop block is journaled to `events.jsonl` with its reason. | P2 | – | done (wave 3b, PR #27, 0.6.0) |

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
| F18 | A check kept from a cancelled task's contract (stored floor or active contract) is added to the floor only when the discovered floor has no check with that id. Before, a repo whose discovery found only `AUTO_DIFF` got two `AUTO_DIFF` entries with different check labels, so no proposal could start a task. The floor refusal now names every field that differs (command, check label, mandatory) instead of only the commands. Field report on 0.5.3, `tp-sched-01`, 2026-11-02. | P0 | – | done (a5fbc3a, 0.5.5; owner confirmed fixed in the field 2026-11-03) |
| F10b | "No checks found" becomes final: the error says `not retryable` and tells the model to stop and tell the user. It does not name the operator-contract path, because `task start` is refused once a prompt has been registered (see K13). | P1 | F11c (same error style) | done (wave 3 commit) |
| F19 | Installing the hooks into a running Claude Code session locks it: UserPromptSubmit demands `intent_apply`, but Claude Code connects a project's `.mcp.json` server only at session start, so no proposal, no write and no uninstall is possible; a person must run `supervisor level off` from outside. Found dogfooding 0.5.5 on this repository, 2026-11-03. Fix: the same proposal through the CLI (`supervisor intent apply`), allowed from the agent's shell while a prompt is pending, and named in the prompt context. | P0 | – | done (wave 3b, PR #27, 0.6.0) |
| F20 | A check whose command has a `|` inside quotes (a search pattern such as `'xdescribe|fdescribe'`) was treated as a shell pipeline whose exit code cannot be trusted: the gate saw it start and never recorded its finish, so the check never cleared. Same for a `|` inside one argv element of a program that is not a shell. Now only a `|` outside quotes counts (a line with an escaped quote or a backtick keeps the old any-`|` rule, since Bash and PowerShell escape differently), and argv counts only for a shell's script argument. Field report on 0.5.5 from JEZ, TestPilot schedule rework check C7, 2026-11-03; the run there split C7 into four checks without `|` as a workaround. | P1 | – | done (wave 3b, PR #27, 0.6.0) |
| K15 | The person-only command guard matches the words anywhere in a shell command, including inside a quoted script or commit message, so an agent's edit script that mentions the command is refused. Conservative; keep unless it blocks real work. | P3 | – | dropped (kept as is: refusing too much here only costs an agent a script file, refusing too little would let it reset its own budget) |
| K16 | `status` coverage still says `harness cap 8` (F16 removed it from refusals only). | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| K4 | `init` drafts checks from the same discovery, not a Rust draft in an npm repo. | P2 | F10a | done (wave 3b, PR #27, 0.6.0) |

## Wave 4: minor 0.6.0, modes (from PROPOSAL.md)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| M1 | Modes `off`, `audit`, `enforce`, `strict`; `lite`, `full`, `ultra` stay accepted as aliases; journals and status print the new names. | P1 | Wave 2 (Stop code settles first) | done (wave 4, PR #29, 0.7.0) |
| M2 | Requested vs effective mode: a mode the harness cannot enforce runs as `audit` and says so in status and every journal line. Absorbs F15 (MCP answers say `answer_authority`, task says its enforcement). | P1 | M1 | done (wave 4, PR #29, 0.7.0) |
| M3 | `/supervisor <mode>` and `/supervisor status` in the plugin, user-invoked only. | P1 | M1 | done (wave 4, PR #29, 0.7.0) |
| M4 | One-step plugin setup: the plugin also starts the MCP server. First find why `1ea1aad` left it out ("F1"); drop this row if the reason holds. | P2 | M3 | dropped (the reason holds: Claude Code names a plugin server's tools `mcp__plugin_coding-supervisor_supervisor__<tool>`, but the gate's pre-registration allowlist, the skills and install's permission rule expect `mcp__supervisor__<tool>`; a plugin-served server would bring back the F19 lock-out. One-step setup needs name-agnostic tool matching first) |
| F13b | `task start` warns that `forbidden_effects` are not enforced (owner's ruling: LINUS checks them in review until M5). | P2 | – | done (wave 3b, PR #27, 0.6.0) |
| K2 | MCP `accept_try` and `verdict` must not disagree: asserted-only receipts cannot reach `ready_for_review` where `verdict` would fail. | P1 | M2 | done (wave 4, PR #29, 0.7.0) |

## Wave 5: minor 0.7.0, what `strict` adds

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| K6 | A failing-before-fix receipt binds to the task's starting candidate and stays valid after the fix is committed. | P1 | – | done (wave 5, PR #30, 0.8.0) |
| M5 | `strict`: typed `forbidden_effects` (`deny_paths`, `deny_git`, `deny_argv_prefix`) compiled into PreToolUse denials for the active task (F13a). | P1 | M1, F13b | done (wave 5, PR #30, 0.8.0) |
| M6 | `strict`: optional per-criterion `min_tests`; a run satisfies it only when the parsed jest/cargo/pytest summary shows enough passed tests (F14). Owner: LINUS confirms counts until then. | P1 | M1 | done (wave 5, PR #30, 0.8.0) |
| M7 | `strict`: a bug fix needs the reported failure observed failing on the starting candidate, then passing on the current one (C1 then C2). How a task is marked as a bug fix is open. | P1 | M1, K6 | done (wave 5, PR #30, 0.8.0); waiting (owner: confirm the default that a task is a bug fix when its contract has a mandatory `baseline_reproduces_reported_failure` criterion; the alternative is a contract or proposal field such as `"kind": "bug_fix"`) |
| M8 | Live Claude Code run of every mode with only the plugin installed. | P1 | M1–M7 | waiting (owner: a live Claude Code run of every mode with only the plugin installed spends your Claude quota; it also settles whether the plugin command's `!` lines pass through the PreToolUse hooks. Proposed: run it when you say so, one short scripted task per mode) |

## Unscheduled (P2 and P3)

| ID | Item | Crit | Depends on | Status |
|---|---|---|---|---|
| X4 | Generated configs hold absolute binary paths; check whether an npm upgrade leaves hooks on the old binary. `install.rs:189` records `current_exe()`. | P2 | – (needed for gate R1) | done (no change: the recorded path `…/node_modules/coding-supervisor-windows-x64/bin/supervisor.exe` carries no version; `npm install -g coding-supervisor@0.5.5` over 0.5.2 replaced the binary in place and `status` reported 0.5.5 from that path, 2026-11-03; owner confirmed 0.5.5 behaviour in the field) |
| K13 | `supervisor task start` is refused once `intents.json` exists, so a person cannot supply an operator contract after the first prompt. Found while writing F10b. | P1 | – | done (wave 3b, PR #27, 0.6.0) |
| K14 | `intent_apply` takes the contract inline only, so the model must retype every command from a contract file; accept a contract path, read through the same validation. Field report, 2026-11-02. | P2 | – | done (wave 3b, PR #27, 0.6.0) |
| K17 | A prompt-managed task can only be cancelled by a prompt in a supervised session; `task deactivate` refuses it, so a person has no command of their own to clear a stuck task, and `amend-floor` waits on it. Field, TestPilot, 2026-11-03. | P1 | – | done (PR #36, 0.9.0: `supervisor task cancel`, a person's command the guard refuses to agents) |
| K3 | `install --dry-run` reports writes into the checkout, including a tracked file. | P2 | – | done (wave 3b, PR #27, 0.6.0) |
| K9 | State location differs with and without `SUPERVISOR_STATE_DIR` (`env` vs `home` rule), so agents see different task namespaces. | P2 | – | done (wave 3b, PR #27, 0.6.0) |
| K7 | `task start` prints `queued` while `task_state` says `preparing`. | P3 | – | dropped (not a bug: a task is `queued` until the gate first opens it, and `task_state` is what opens it, so it then reads `preparing`; the core moves Queued to Preparing on purpose) |
| K8 | `recorded_at` truncated to seconds; README promises milliseconds. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| K5 | `init` without `--output` writes into the repo root. | P3 | – | dropped (documented: the draft must be editable by the person and readable by the agent, and the state directory is closed to agents; `--output` chooses another path) |
| K11 | `supervisor --help` does not mention the MCP tools or the receipt sequence. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| K12 | README Install still says 0.1.1; also says the autonomous workflow is unreleased (shipped in 0.2.0). | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X1 | `supervisor --version`. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X2 | Alias bin `coding-supervisor` or a clearer install message. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X3 | `verdict` outside a git repo prints a git error for `no_active_task`. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X5 | Codex trust step ignores `CODEX_HOME`. | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X6 | README: criteria design advice from the report's environment note (no parallel heavy checks, cgroup over `prlimit --as`, `jest <files> -t`). | P3 | – | done (wave 3b, PR #27, 0.6.0) |
| X7 | State backup before upgrade. | P3 | – | waiting (owner: snapshot state on upgrade? npm upgrades never run our code, so it would need a postinstall script, which many setups disable; proposed default: drop, since state formats migrate in place through `task migrate`) |
| K1 | No non-Claude path yields an observed receipt. Design limit of MCP; recorded, no fix planned. | – | – | dropped (by design) |

## Housekeeping

| ID | Item | Depends on | Status |
|---|---|---|---|
| H1 | Archive PR #2 (`feat/autonomous-gate`, head `120a0ed`): push tag `archive/autonomous-gate`, close with a note pointing here. Owner approved 2026-11-02. | – | done (tag pushed, #2 closed) |
| H2 | Refresh stale lines in `CLAUDE.md` (branch `phase-a-installable`, "uncommitted" work now on main). | Wave 1 | done (wave 3b, PR #27, 0.6.0) |
| H3 | CI waste (owner, 2026-11-03: "remove any and all waste"): every PR ran CI twice (push and pull_request), docs-only and release-please changes ran it too, Windows repeated fmt and clippy, tests ran one file at a time. | – | done (PR #33: one run per change, Linux-only fmt/clippy, `cargo nextest`; Linux 3m0s to 1m55s, Windows 8m40s–10m30s to 5m27s) |
| H4 | One Stop started 19 git processes (16 the same root lookup), every tool call 8 to 10; tens of milliseconds each on Windows. | – | done (PR #34, 0.8.1: 3 per hook call, pinned by `cli/tests/git_calls.rs`; also nextest installs on the `fresh` runners without `sudo`; reusable write-up: `docs/research/2026-11-03-ci-speed/CI-SPEED-GUIDE.md`) |

## Gate friction (2026-11-05)

Two dogfood sessions (this repository and terminal-commander) hit gate dead ends with no permitted next call. Owner, 2026-11-05: "I want my coding supervisor to be usable and very robust, but not to the detriment of coding my projects." Symforge was moved to `audit` until these land.

| ID | Item | Status |
|---|---|---|
| G1 | A prompt first applied as `discussion` could never become work ("prompt event already consumed"). | done (PR #42: same-event upgrade; work cannot be downgraded) |
| G2 | A task bound to a dead session refused `start`, `continue` and `cancel`, and the prompt hook still advised `continue`; nothing named the way out. | done (PR #42: advice and refusals name the person-only cancel command) |
| G3 | `supervisor level --project C:\...` from a POSIX shell lost its backslashes, reported success and wrote a state root the hooks never read. | done (PR #42: a `--project` that is not a directory is refused) |
| G4 | A session whose prompt is registered in the main checkout could edit, ungated, in a linked worktree (`EnterWorktree` or a hook `cwd` that follows the agent): linked worktrees keep their own gate state and had no prompt for the session. | done (PR #44: such edits are denied for the standard `<main>/.git/worktrees/<name>` layout; a worktree with a separate git dir, or whose main state cannot be resolved, is not recognised and stays as before; regressions in `cli/tests/worktree.rs`) |
| G5 | Work in a linked worktree cannot complete a task registered in the main checkout ("candidate identity not observed"); start the session in the worktree instead. Binding a task to a linked worktree of the same repository needs the candidate observed there, schema-4 run records and scope resolution against the bound root. Agreed design (advisor, 2026-11-05): per-task binding inside the existing per-toplevel state namespace, not a repository-level namespace (that would make two parallel worktree sessions share one active task). An optional `worktree` path on `start`/`revise`, accepted only when its common git dir equals the session project's (normalized; bind lazily if the path does not exist yet); `GitCandidate` observes the bound root; dispatches count when `cwd` equals it, recorded as a new field (schema 4) so old records keep the old rule; edit-scope checks resolve against the bound root. Evidence counts only when the run's tree is the tree the candidate was observed in. | done (optional `worktree` on `start`/`revise`, bound per task in `autonomy.json`; a hook call from the bound linked worktree uses the main checkout's state, the candidate, edit scope and counted dispatches follow the bound root, other roots' runs do not count; shown in `task_state` and `supervisor status`; regressions in `cli/tests/worktree.rs`; unbound tasks and G4 denial unchanged) |
| G6 | Runs with an injected `CARGO_TARGET_DIR` were refused as "unapproved" with no hint; the refusal now says to set it in the harness settings `env` or cargo's `[build] target-dir`. | done (message) |
| G7 | A Terminal Commander job whose exit was seen only through `bucket_wait` stayed "no observed completion"; the refusal now names `command_status` and the job. Parsing `bucket_wait`'s free-text exit summary was rejected: it carries no structured exit state. | done (message; regression in `autonomous_completion.rs`) |
| G8 | After the agent correctly reported "not complete", Stop refused twice more per turn. Kept: the refusals now name the next step, and accepting "not complete" wording would be the text judgment Stop avoids. | dropped (by design) |
| G9 | Committing verified work changes the candidate (HEAD) and stales every receipt, so the full suite reruns after `git commit`. Agreed design (advisor, 2026-11-05): evidence transfers only when git says the tree content is identical. A content id from a scratch index (`GIT_INDEX_FILE=<state>/scratch-index git add -A` then `write-tree`; filters applied as a commit would; writes unreachable loose objects) is stored on criterion dispatch rows and computed at Stop only when the exact candidate match fails; the core `CandidateId` and receipts stay unchanged; every comparison site goes through one helper; a match by tree is journaled (`matched_by: "tree"`). Tests: commit -a accepts; a partial commit, an edit after commit, refuse; an amend that only changes the message accepts. Owner decision needed first: this changes CLAUDE.md implementation principle 2 ("a check binds to one candidate identity"). | done (owner approved; CLAUDE.md principle 2 updated; dispatch rows carry `tree` (schema 4), a tree match writes a `transfer` row and a receipt on the current candidate; regressions in `adapters/claude-code/tests/tree_transfer.rs`) |
| G10 | The prompt floor adds the repository-wide checks to every contract, including changes that touch no Rust. By design (an agent cannot drop the repository's tests). | dropped (by design) |
| G11 | Guardrail layer 3 by default (owner, 2026-11-05, after mapping the guardrail taxonomy): with a task active, a push that updates or deletes `main`, `master` or `origin/HEAD`'s branch, any force-push and `npm publish` are refused from `enforce` up, reported at `audit`, untouched at `off`. New typed kinds `deny_git_push_target:<branch>` and `deny_git_force_push`; the contract's own typed entries stay `strict`-only. Not a global guardrail: without an active task nothing is refused, and a `cd x &&` before a bare push is not followed. | done (regression `default_forbidden_effects_are_refused_from_enforce_up`) |
| G12 | `gh pr merge` into `main` is owner-only in the global rules but is not a default forbidden effect: the owner pre-approves merges per session, and refusing it would block that. Revisit with an owner-approval escape. | todo |
| G13 | A Terminal Commander job stopped with `command_stop` stays `pending` in `task_state.pending_jobs`. | done (a `stopped` row ends it and counts as no completion; regression `a_stopped_tc_job_is_no_longer_pending_and_counts_as_no_completion`) |
| G14 | After an autonomous task completed, every non-read-only tool was refused with a bare "GATE: task is completed", including the read-only `supervisor status` an unclassified prompt may run. The same read-only tools and commands now stay open (plus `supervisor level` without arguments), and the refusal names the next step. A read-only web fetch is still refused during a discussion turn: fetching a URL can carry data out. | done (regression `a_completed_task_still_allows_read_only_commands_and_names_the_next_step`) |
