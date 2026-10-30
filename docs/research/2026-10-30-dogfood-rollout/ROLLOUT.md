# Eight-project dogfood rollout

As of 2026-10-30. This is a local installation record and a proposed development backlog, not a claim of live-model enforcement on this machine.

## Outcome and activation boundary

Installed `coding-supervisor@0.1.0` globally through npm. Installed the Claude Code and Codex project integrations in all eight projects below. Claude has six hook events and the advisory MCP server; Codex has the advisory MCP server and no supervisor hooks. Both have the rendered chair and worker skills.

**No live task contracts were initialized.** This deliberately stages integrations without imposing a placeholder task on ordinary work. The current adapter is advisory when there is no contract. It still protects its state paths. Activate a real task only after its goal, exact commands and scope are defined and there is an agreed task-transition strategy. Do not interpret installed hooks as an active completion gate.

The owner intends to continue developing supervisor in another session and requested findings for that work. Preserve full access and low friction. Fix lifecycle and receipt gaps instead of adding approval prompts or weakening checks.

## Repository updates

Fetched `origin` for all eight; verified zero ahead/behind against each checkout's tracked upstream afterward. No reset, stash, rebase, push or commit was performed. AAP stays on the owner's requested `campaign/3d-cumulative-checkpoint-2026-10-18` branch; this is not a claim that it matches `main`.

| Project | Update |
|---|---|
| coding-supervisor | Already current, `8c2b77a` |
| symforge | Already current, `c3abec19` |
| RelayForge | Already current, `768a7c6` |
| aap-runner | Fast-forwarded eight commits, `52f9d22` to `79d1feb` |
| Agent_Army_Professionals | Campaign branch already current, `71af3770` |
| fathom | Fast-forwarded eighteen commits, `6dde886` to `a6b31df` |
| terminal-commander | Already current, `963d72f` |
| GW2_Build_Optimizer | Already current, `d483ab2a` |

Existing Terminal Commander documentation edits, GW2 `.gitignore` changes, and untracked local tooling directories were preserved. Installation appended `.supervisor/` to each existing `.gitignore`; those changes remain uncommitted.

## Installed configuration

Project roots are the eight named folders under the owner's `C:/AI_STUFF/PROGRAMMING` checkout directory. Use the actual paths in local configuration; do not copy this machine's configuration into another machine's checkout.

For each project:

- `.mcp.json`: named `supervisor` server, native executable, pinned `mcp --project` argument.
- `.claude/settings.json`: UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, SubagentStop and Stop; exactly one supervisor hook per event. Existing hooks/settings preserved.
- `.claude/settings.local.json`: approval for the named `supervisor` server; existing settings preserved. No blanket approval for project MCP servers.
- `.claude/skills/supervisor-{chair,worker}/SKILL.md`: Claude skills.
- `.codex/config.toml`: supervisor MCP registration; existing TOML settings preserved.
- `.agents/skills/supervisor-{chair,worker}/SKILL.md`: Codex skills.
- Exact generated config/skill paths added to local `.git/info/exclude` to avoid publishing machine-specific installation paths.

Added missing exact project trust entries for Codex in coding-supervisor, fathom and GW2; other five were already trusted. Added missing Claude project trust for coding-supervisor and RelayForge; other six were already trusted. Did not change the full-access/approval policy, model settings, Jev routing or existing safety controls.

The global native executable is in the npm-installed `coding-supervisor-windows-x64/bin` package. Its SHA-256 is `fabe4f00f3c9c0869e53ff8aaad1bfead9dd313960ea1d51304eba858771944e`. Project configuration uses the native executable rather than relying on shell launcher resolution. After a binary relocation, rerun the supported installer for each project.

Backups and a manifest are in the owner's `.codex/backups/supervisor-rollout-20261030-090110` directory. The `activation` subdirectory includes pre-change trust and local approval files. These backups can contain private configuration: do not commit or print their contents. Restore only the relevant changes after checking for intervening edits; do not replace live user configuration wholesale.

## Verification

- Global npm installation: exit 0; installed launcher `--help`: exit 0.
- All sixteen project dry-runs and installations: exit 0. Repeated dry-runs produced no changes.
- Parsed installed JSON/TOML and checked that existing values/list entries survived.
- Confirmed exactly six supervisor hook events, one entry each, in every project.
- Confirmed every `.gitignore` retained its entire pre-install content and only appended `.supervisor/`.
- Independent isolated verification: init preserves an existing contract; repeat installations are byte-identical; MCP initializes, lists eight tools and answers task_state; a synthetic Stop with missing evidence refuses; synthetic observed-command receipts plus CHECK lines reach completed.
- Isolated test receipts: Terminal Commander jobs `job_01a18bd23637731daff8045aedf1f4fc` and `job_01a18bd278b272299cfb251f5833e9e7`, exit 0, observed. These are replay assertions, not proof that the synthetic cargo commands executed.
- Actual installed-project checks: all eight `claude mcp get supervisor` calls exited 0 and reported Connected. All eight `codex mcp get supervisor --json` calls exited 0 with enabled=true and the correct native executable/project argument. Direct MCP checks against both registrations in each project initialized, listed eight tools and returned the expected no-contract refusal from task_state. Exact project trust and named server approvals were verified. Terminal Commander job `job_01a18bd739c872d097cfde1391c38eb3` exited 0, observed, with eight projects passed and zero failed.
- Codex's get command proves configuration loading; direct MCP proves the registered process works. Neither proves that an already-running Codex model session has loaded the newly installed server. Reopen the project session to load it. No paid model session or full product regression suite is part of this configuration-only rollout.

## Recommended next development work

### 1. Give tasks an explicit lifecycle

Evidence: `adapters/mcp/src/lib.rs::task_start` rejects any existing contract. `cli/src/init.rs::run` never replaces it. `adapters/claude-code/src/adapter.rs` denies Write/Edit/Agent after completed; a new prompt only resets continuation accounting. There is no supported next-task/reset CLI.

Implement an explicit next-task transition with unique task IDs, immutable archived evidence and a project-to-active-task pointer. Require an explicit disposition for unfinished work. Start the next task without deleting protected state, silently editing its contract, or weakening tamper detection. Separate project installation from task activation.

Acceptance: two real tasks complete sequentially in the same checkout, a resumed task retains its history, and late receipts from task one cannot satisfy task two. Implement a safe explicit recovery path for blocked/budget-exhausted tasks.

### 2. Correlate Terminal Commander jobs through completion

Evidence: the adapter recognizes command/argv/script but not `shell_line`. A finished run_and_watch can provide a receipt; an asynchronous start has no exit code and a later command_status has no original command. The current adapter cannot join those events. See `adapters/claude-code/src/adapter.rs::observe_command` and related parsing code.

Persist job ID, resolved cwd, command identity and start-candidate identity at dispatch. Join completion from the same job with its authoritative exit status and outcome trust. Support run_and_watch, command_start_combed, command_status and shell_exec without accepting a model's assertion as a command observation. Do not require rerunning long tests through bare Bash to satisfy the gate.

Acceptance: a job that outlives the first tool response is accepted only after an observed successful completion; wrong job, missing exit, degraded/lost execution, wrong cwd and changed candidate remain rejected. Preserve structured argv instead of ambiguous space-joined command strings.

### 3. Make effective enforcement visible

Installation, registration loading, contract activation and observed evidence are distinct states. Add a `doctor` or `status` report showing harness, binary/version, project identity, active task, hook coverage and whether receipts are observed or asserted. Make a missing contract explicitly say `installed; inactive/advisory`, not `protected`.

Acceptance: fresh install, trusted/loaded install, active Claude task, inactive task and advisory Codex session each produce distinct truthful status. Keep ordinary conversation free from mandatory test gates.

### 4. Protect concurrent sessions and project identity

Current project state is shared by both harnesses, and the state-directory key replaces punctuation with hyphens (`adapters/claude-code/src/location.rs`). Source documentation explicitly lists concurrent hooks as unverified/uncovered.

Add serialized transactional state updates and unique event IDs before broad parallel dogfooding. Use a canonical project/worktree identity with a collision-resistant suffix, with an explicit migration for existing state. Decide whether sessions cooperate on one task or have separate task namespaces.

Acceptance: racing hook/MCP events neither lose receipts nor falsely report tampering; duplicate events are idempotent; two paths that normalize to the same slug remain separate; worktree behavior is explicit.

### 5. Keep completion control distinct from action authorization

The repository declares `forbidden_effects` but does not enforce them. C4 scope is optional in the starter contract. An advisory MCP receipt is caller-asserted; Codex has no enforcing adapter here. Installation cannot complete the whole safety loop by itself.

Specify the action policy separately from completion evidence. Prioritize protected-path deletion, protected-branch history changes, secret publication and unauthorized external effects across all actual execution routes. Keep routine work silent. Maintain a coverage matrix with observed fixtures, including bypass paths, hook failure, non-header completion claims, cap exhaustion and stale evidence. Do not call same-user writable hooks an unbypassable security boundary.

## Next session

Read this record and current source before changing behavior. Implement and verify task lifecycle and asynchronous receipt linkage first, then activate one real task in coding-supervisor and observe it end to end before enabling task contracts in the other seven. Retain the installed integrations; no need to repeat installation unless the binary or templates change.
