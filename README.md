# coding-supervisor

coding-supervisor is a completion gate for coding agents. It records a task contract, candidate and verification evidence, and evaluates whether the required checks support completion. It runs alongside a coding harness; the harness supplies the agent and tools.

The core is deterministic. A receipt must match a criterion's command and the candidate it checked. A changed candidate makes affected evidence stale. Repair rounds are bounded and unresolved work has explicit outcomes. The Claude Code hook adapter can refuse unsupported completion on its covered paths; the MCP interface is advisory and its receipts are caller assertions.

## Install

The npm launcher and native packages support Windows and Linux x86_64:

```sh
npm install -g coding-supervisor
supervisor --help
```

Alternatively, download the archive for your platform and its checksums from the [Releases page](https://github.com/special-place-ai-heaven/coding-supervisor/releases), or build from source below. Keep the executable at a stable path: the installer records its path in the harness configuration. With npm that path carries no version, so `npm install -g coding-supervisor@<version>` upgrades the binary the hooks already run; `supervisor status --json` shows the running version in `reporting_binary.version`. Releases and their notes are on the [Releases page](https://github.com/special-place-ai-heaven/coding-supervisor/releases) and in `CHANGELOG.md`.

From the project you want to supervise, register your harness:

```sh
supervisor install --harness claude-code --project <project> --dry-run
supervisor install --harness claude-code --project <project>
supervisor status --project <project> --harness claude-code
```

Replace `claude-code` with `codex`, `cursor`, `omp` or `grok` for installation. Follow the manual trust or approval steps printed by the installer. Claude Code connects a project's MCP server only when a session starts: install before starting the session, or restart it. A session that has the hooks but not the server classifies its prompt from the Bash tool with `supervisor intent apply --proposal '<json>'`. Installation registers the MCP server and two skills; only Claude Code receives supervisor hooks. Its PreToolUse hook carries `"onFailure": "block"`, as the plugin's does, so a `supervisor` that crashes or does not answer in time refuses the tool call instead of letting it through. It does not activate a task. The installer does not stop other processes; if a file is held open, close the harness session and retry. A malformed configuration is not rewritten.

### Claude Code plugin

Instead of hook entries in `.claude/settings.json`, Claude Code can take the gate's hooks from this repository's plugin. The plugin still needs the npm package, because its hook runs `supervisor` from `PATH`, and it declares no MCP server, so `supervisor install` stays the MCP registrar:

```sh
npm install -g coding-supervisor
claude plugin marketplace add special-place-ai-heaven/coding-supervisor
claude plugin install coding-supervisor@coding-supervisor
supervisor install --harness claude-code --project <project>
```

When `enabledPlugins` has `coding-supervisor@coding-supervisor: true` in the most specific settings file that names it (the project's `.claude/settings.local.json`, then `.claude/settings.json`, then `~/.claude/settings.json`), install prints `the coding-supervisor@coding-supervisor plugin is enabled and runs the hooks: no hook entry is written, and any written earlier is removed`. It then writes no hook entry and removes the gate's earlier ones, so no event runs the gate twice. It still writes the MCP server, the `mcp__supervisor` allow rule and the skills. `supervisor status` reads hooks from the project's settings files only, so it reports plugin hooks as `not_configured`.

The plugin adds the hooks and one command, `/supervisor`, with no MCP server, agent, skill or executable. `/supervisor` shows the [mode](#levels) and the status; `/supervisor <off|audit|enforce|strict>` sets the project's mode first. The command is marked `disable-model-invocation`, so only a person can run it, and it runs `supervisor level` and `supervisor status` before the model sees anything. The plugin carries no MCP server because Claude Code names a plugin server's tools `mcp__plugin_coding-supervisor_supervisor__<tool>`, while the gate, its skills and the permission rule install writes expect `mcp__supervisor__<tool>`; `supervisor install` stays the MCP registrar. It registers `node hooks/supervisor-hook.js` on the same six events as install, with no matcher and a 30-second timeout. The script passes the payload to `supervisor hook` and its stdout back unchanged.

The script looks `supervisor` up in the absolute `PATH` entries only, trying each `PATHEXT` extension in order on Windows. Empty and relative entries, which mean the current directory, are skipped, so a `supervisor` or `supervisor.cmd` written in the project is never run. A `.cmd` or `.bat` (npm's shim on Windows) is started as `cmd.exe /d /s /c` with its quoted absolute path; anything else is started with no shell. The child gets `NoDefaultCurrentDirectoryInExePath=1`, so cmd.exe, including the shim's own `node`, does not run programs from the current directory either.

When the gate cannot answer, the script does the following:

| Failure | PreToolUse, Stop, SubagentStop, or a payload with no `hook_event_name` | Stop or SubagentStop with `"stop_hook_active": true` | UserPromptSubmit, PostToolUse, PostToolUseFailure |
|---|---|---|---|
| `supervisor` cannot be started from `PATH`, exits non-zero, is killed, gives no answer within 20 seconds, or prints anything other than one JSON object, or the script itself fails | Exit 2, stdout empty, stderr `coding-supervisor: <event> refused, the gate fails closed: <reason>.` Claude Code refuses the tool call or keeps the turn going. | Exit 0, stdout `{"systemMessage":"coding-supervisor: WARNING, this <event> went ahead NOT certified: <reason>. …"}`. Claude Code is already continuing from a refused stop, so this stop ends the turn instead of looping. The task has no receipt. | Exit 0, stdout `{"systemMessage":"coding-supervisor: <event> went ahead unjudged: <reason>."}`. The prompt or tool result goes ahead and the user sees the message. |

PreToolUse also sets `"onFailure": "block"` (Claude Code 2.1.295 or later), so a hook that does not start or exceeds its 30-second timeout still refuses the tool call. Claude Code ignores `onFailure` on Stop and SubagentStop, so a Stop hook that exceeds its own timeout records nothing and does not block. The resulting missing record leaves no receipt.

The hook tests need Node.js 18 or later; CI does not run them yet:

```sh
node --test hooks/supervisor-hook.test.js
```

One test runs on Windows only (cmd.exe never takes the shim's `node` from the current directory). To run it alone:

```sh
node --test --test-name-pattern="Windows shim" hooks/supervisor-hook.test.js
```

## Ordinary tasks

The autonomous workflow ships in every release since 0.2.0. After installing
and starting a new harness session, give ordinary tasks. The agent interprets the request and proposes a scoped contract;
the supervisor validates it before covered edits. Discussion stays inactive.
Users do not prepare contracts, choose supervisor commands or write CHECK headers.

Root and subdirectory entry points discover the same physical worktree task.
Check evidence retains its actual execution context: current criteria require
root execution, and unapproved per-command environment changes cannot certify a
check. Normalizing an old path does not upgrade old evidence. See the
[fresh-audit repair record](docs/research/2026-10-30-dogfood-rollout/FRESH-AUDIT-REPAIRS.md)
for verification results and compatibility boundaries.

Claude's UserPromptSubmit hook supplies prompt identity. Codex uses the same
proposal workflow through caller-asserted MCP intent and remains advisory.
Repository checks are captured before edits, then retained as mandatory criteria.
Cargo projects use fmt, clippy and workspace tests; declared npm test, lint and
typecheck scripts are also detected. Captured npm verification entry points
cannot be silently replaced while retaining the same command name. Unsupported verification setups require
existing approved criteria; the model cannot invent a trivial verification floor.
These checks constrain proposals; they do not prove a proposed contract fully
captures the meaning of a free-text request.

Follow-ups retain task identity. A new task after completion archives its
predecessor. Unfinished work is retained until continued or explicitly cancelled.
Pause/resume and monotonic contract revisions preserve history and repair counts;
revision invalidates old evidence. Ambiguous control requests require clarification.
A new session can explicitly resume the task; obsolete sessions cannot mutate
its state. `task_state` exposes pending job identifiers for recovery while
retaining their original execution provenance.

Live Claude model acceptance for this candidate is **deferred due to quota**.
Deterministic replay and subprocess results do not prove live hook enforcement.
See the [amended goal and evidence](docs/research/2026-10-30-dogfood-rollout/AUTONOMY-GOAL.md).

## Explicit recovery and older installations

`init` writes an inactive editable `supervisor-contract.json` in the project, or at the path supplied with `--output`. Edit the draft for the actual goal, scope, criteria, exact accepted commands and repair budget. Each criterion needs a command or list of commands; matching trims whitespace but otherwise requires an exact match. The generated examples are placeholders.

```sh
supervisor task start --project <project> --contract <project>/supervisor-contract.json
supervisor status --project <project> --harness claude-code --json
supervisor task resume --project <project> --task <generated-task-id>
```

Keep the generated task ID; it is distinct from the contract's ID. Resume preserves the task's candidate, receipts and history. Status distinguishes inactive and active state, registration and observed connections, and receipt provenance. A configured executable or connection does not prove a model session loaded it; the reporting binary may differ from the configured one.

For a successor after completion:

```sh
supervisor task next --project <project> --task <old-id> --contract <next.json>
supervisor task rebind --project <project> --task <active-id> --session <session-id>
supervisor task deactivate --project <project> --task <active-id>
```

For unfinished work, `next` or `deactivate` requires `--disposition cancelled`, `failed`, `blocked`, or `budget_exhausted`. The old task is archived; its evidence is retained. Prefer a fresh harness session for a successor; use `rebind` only to deliberately reuse an existing session. Rebinding does not transfer pending checks.

Three commands are for a person only; the gate refuses them from an agent's shell, as it refuses `supervisor level`:

```sh
supervisor task reset-rounds --project <project> --task <active-id> --reason <why>
supervisor task amend-floor --project <project> --criterion <id> --command <command> --reason <why>
supervisor task cancel --project <project> --task <active-id> --reason <why>
```

`reset-rounds` moves a `budget_exhausted` task back to `repairing` with no rounds spent and journals the reason. `cancel` ends the active task, including a prompt-managed one that `deactivate` refuses and that otherwise only a cancel prompt in a supervised session can end; it is archived as `cancelled` with the reason, and its checks are kept as the project floor, as a cancel prompt keeps them. `amend-floor` replaces one command of the requirement floor while no task is active, and records the old and new command with the reason: a discovered check (for example `AUTO_NPM_TEST_BACKEND`, to add `--maxWorkers=2`), or a check of the stored floor kept when a prompt-managed task is cancelled in a repository whose files yield none. Discovery reads `Cargo.toml` and the `test`, `lint` and `typecheck` scripts of the root `package.json`, or, when the root has none, of each direct subfolder's `package.json` as `npm --prefix <dir> run <script>`. A Stop refusal spends a repair round only when a required check was observed failing, a check's npm entrypoint was redefined, a finding is open, or the diff leaves the scope, and at most once per version of the code; a check that never ran, or another refusal before the code changes, spends nothing. A required check started in the background is refused: run it in the foreground.

Legacy state requires explicit selection after checking its ownership:

```sh
supervisor task migrate --project <project> --legacy <old-state-directory>
```

Migration preserves the source. Each physical worktree has its own state namespace; keep `SUPERVISOR_STATE_DIR` consistent across cooperating processes. For recovery and state locations, see the [operations guide](docs/research/2026-10-30-dogfood-rollout/OPERATIONS.md). Retain evidence when state is corrupt or tampered; rollover is not a tamper reset.

## Designing checks

Each required check runs alone, in the foreground, on the code as it is. Checks that run
slowly or use a lot of memory decide whether a task can finish at all, so design them for the
machine they run on:

- Do not require two heavy checks that the agent will run at the same time; a test suite and a
  browser build together can exhaust memory where either alone passes.
- Cap a test runner's workers in the command itself (`jest --maxWorkers=2`,
  `cargo test -- --test-threads=2`); a person can amend a discovered command with `supervisor task amend-floor`.
- To limit memory, prefer a cgroup limit (`systemd-run --user --scope -p MemoryMax=…`) over
  `prlimit --as`: an address-space limit is inherited by every child, and Node and Chrome
  reserve far more address space than they use, so they fail long before memory runs out.
- A focused check (`jest <files> -t <name>`) is a fine per-task criterion; keep the full suite
  for the repository floor.

## Evidence and completion

The development adapter derives chair and worker receipts directly from observed
runs, without CHECK headers. Evidence remains bound to task, session, candidate,
working directory and job. A later failed or unresolved attempt cannot borrow an
earlier success. A self-reported check or MCP receipt does not satisfy hook evidence.
The MCP server adds `intent_record` and `intent_apply` to the existing eight tools:
`task_start`, `candidate_observe`, `receipt_record`, `finding_raise`,
`repair_request`, `accept_try`, `task_complete` and `task_state`. MCP receipts and
intent registration remain caller-asserted; MCP responses do not block the harness.

For Terminal Commander jobs, use an explicit absolute working directory, preserve argument boundaries and retrieve the final status of the same job. Dispatch binds task, candidate and cwd; changes during execution can make evidence stale. After a daemon restart, reconstructed status can support recovery only with persisted matching dispatch and durable process-completion evidence. It is labeled `tc-reconstructed`, separately from live observations. Missing linkage, masked pipelines and old records without current provenance require investigation or a fresh check. See [recovery details](docs/research/2026-10-30-dogfood-rollout/OPERATIONS.md#durable-jobs-and-crash-recovery) and [repair evidence](docs/research/2026-10-30-dogfood-rollout/REPAIR-RESULTS.md).

### Verdict

`supervisor verdict [--project <dir>] [--task <active-id>] [--require observed|asserted] [--json]`
recomputes the active task's result from its receipts, the hook run journal and the
live git candidate; it never reads the stored gate state. `--task` must name the active task.

- **Provenance.** A receipt is `hook` or `tc` when its `observed:` or
  `tc-reconstructed:` reference names a current dispatch and a run in the hook journal
  matches its command, cwd, candidate, exit code and times; `mcp` when MCP
  `receipt_record` wrote it; otherwise `unverified`.
- **Current.** HEAD is read once, before any receipt. A receipt is current only when its
  sha and dirty digest byte-equal that snapshot. If HEAD or the worktree moves after the
  snapshot, the verdict is still a correct statement about the reported `head.sha` and
  `head.dirty_digest`; it is not an error and is not retried. If it moved before the
  snapshot, older receipts read as `stale`. Callers must compare `head.sha` with the HEAD
  they act on and must not apply a verdict to any other sha.
- **State.** Among a criterion's current receipts the latest in the first matching tier
  decides: `hook`/`tc` gives `observed` (exit 0) or `failed`; else `mcp` gives
  `asserted_not_observed` or `failed`; else `unverified`. With receipts but none current
  it is `stale`; with none, `missing`.
- **Require.** `observed` (the default) passes only when every mandatory criterion is
  `observed`. `asserted` also accepts `asserted_not_observed`; it relaxes nothing else.
  Missing or asserted-only receipts are flagged before M6 and hard-blocked at the gate
  from M6; the verdict encodes no milestone, and from M6 gates use `observed`.
  Non-mandatory criteria are listed but never affect the result, and a task with no
  mandatory criterion fails.

`--json` prints one object on one line. Every key is always present, with `null` for
none:

```json
{"schema_version":1,"verdict":"fail","reason":"mandatory_unmet","exit_code":3,"require":"observed",
 "effective_level":"enforce","level_source":"default",
 "task":{"task_id":"task-…","contract_id":"probe","contract_digest":"<64 hex>"},
 "head":{"sha":"<40 hex>","dirty_digest":"<64 hex>","reason":null},
 "criteria":[{"id":"C1","mandatory":true,"state":"asserted_not_observed","reason":null,"provenance":"mcp",
   "receipt":{"head_sha":"<40 hex>","dirty_digest":"<64 hex>","command":"false","exit_code":0,
              "recorded_at":"2026-11-01T08:55:42.000Z","output_ref":"mcp-asserted:…"},
   "satisfies":false}],
 "unmet":["C1"],
 "flags":[{"criterion":"C1","code":"asserted_not_observed","mandatory":true}]}
```

| field | values |
|---|---|
| `reason` | `null` on pass; else the first of `no_active_task`, `head_unknown`, `no_mandatory_criteria`, `mandatory_unmet` |
| `task` | `null` with no active task; `contract_digest` is the sha256 of contract.json |
| `head.sha`, `head.dirty_digest` | 40 and 64 lower-case hex, or both `null` with `head.reason` one of `git_unavailable`, `git_failed`, `worktree_unreadable`, `head_unparseable` (the cause goes to stderr) |
| `criteria[]` | contract order; `state` as above, `provenance` `hook`, `tc`, `mcp`, `unverified` or `null` when missing |
| `criteria[].reason` | `failed`: `exit_nonzero`, `no_exit_code` or `claim_unsupported`; `unverified`: `no_observation_record`; `stale`: `receipt_at_other_head`, `worktree_changed` or `head_unknown`; otherwise `null` |
| `criteria[].receipt` | the deciding receipt, `null` when missing; `recorded_at` is RFC 3339 UTC; the gate records whole seconds, so the millisecond field is always `.000` |
| `unmet`, `flags` | mandatory criteria not satisfied, and every `asserted_not_observed` criterion, in contract order |
| `effective_level`, `level_source` | the hook's level and its source, read when the verdict runs; see [Levels](#levels) |

Without `--json` the same result is printed as one `<id>: ` line per criterion and a
final `verdict: pass (` or `verdict: fail (` line; only those prefixes are stable.

| exit | meaning | stdout |
|---|---|---|
| 0 | pass | the verdict |
| 3 | answered fail | the verdict |
| 1 | could not answer: unreadable, corrupt or tampered state (including runs.jsonl and autonomy.json), a pending transaction, or a `--task` mismatch | empty; the error on stderr |
| 2 | usage | empty; usage on stderr |

The verdict writes nothing: it takes a shared hold on an existing state lock, creates no
state directory or lock, leaves a pending transaction for `supervisor status` to
recover, and runs git without optional locks.

`observed` proves consistency, not authenticity: it means a matching hook-format row
exists in runs.jsonl for a current dispatch. Any process running as the same user can
append a perfectly matching row and so forge `observed`; the same holds for a
caller-supplied MCP candidate. This is an accepted risk under the same-user limitation
(tamper detected, not prevented), not something the verdict defends against. Do not
describe `observed` as tamper-proof.

## Harness support

| Harness | Supervisor integration and evidence |
|---|---|
| Claude Code | Hooks can deny covered tool calls and refuse unsupported chair or worker completion when loaded. Two sequential real tasks on the repaired code completed with observed receipts; see [results](docs/research/2026-10-30-dogfood-rollout/REPAIR-RESULTS.md). Completion refusals have an adapter cap of three; the historical measured harness cap was eight. |
| Codex CLI | Installed MCP integration is advisory; no enforcing supervisor hook adapter. Installed connection was tested with a scripted model. See [Codex results](docs/research/2026-10-29-installable/codex-T3-RESULTS.md). |
| OMP | Installed MCP integration is advisory; no enforcing supervisor adapter. Its hook capabilities were investigated separately, and the installed MCP integration was exercised with a live model. See [OMP results](docs/research/2026-10-29-installable/omp-T3-RESULTS.md). |
| Grok Build | Installed MCP integration is advisory; no enforcing supervisor adapter. The installed connection and hook investigation used a scripted model. See [Grok results](docs/research/2026-10-29-installable/grok-T3-RESULTS.md). |
| Cursor | Installed MCP integration is advisory; no enforcing supervisor adapter. The registration listed tools after approval; model-driven verification was blocked by quota. See [Cursor results](docs/research/2026-10-29-installable/cursor-T3-RESULTS.md). |

Harness hook capabilities and a supervisor adapter are separate questions. A configured hook only enforces when the harness loads and executes it. Hook failures and timeouts may yield no denial; same-user access to hooks and state is not an unbypassable security boundary. See the [coverage matrix](docs/research/2026-10-30-dogfood-rollout/COVERAGE.md).

## Levels

`supervisor level [off|audit|enforce|strict] [--project <dir>] [--global] [--failure open|closed]` shows the effective level (`level: <name> (project|global|default)`, then the failure mode, see below) or writes one (`level <name> written (project|global): <path>`). Exit 0 done, 1 error (for example an unreadable `level` file), 2 usage.

Precedence: **project** (a `level` file in the project's state directory) over **global** (a `level` file in `SUPERVISOR_CONFIG_DIR`, else `%APPDATA%\coding-supervisor` on Windows, else `$XDG_CONFIG_HOME/coding-supervisor`, else `~/.config/coding-supervisor`) over **default** (`enforce`). The names used before 0.7, `lite`, `full` and `ultra`, are still accepted for `audit`, `enforce` and `strict`, and level files written with them still read. The Claude Code hook reads it on every call:

- `off`: nothing is judged or written; PreToolUse still denies writes to the gate's state and configured action-policy violations.
- `audit`: judged as `enforce`, but every refusal is reported as a system message instead of blocking, and stop refusals do not count toward the refusal caps.
- `enforce`: the gate as described above. While a task is active, PreToolUse also refuses three effects by default, whatever the contract says: a `git push` that updates or deletes `main`, `master` or the branch `origin/HEAD` names (including `--all`, `--mirror`, and a bare push or `HEAD` while that branch is checked out), any force-push (`--force`, `-f`, `--force-with-lease`, `--force-if-includes` or a `+refspec`), and `npm publish`. At `audit` they are reported instead; at `off` nothing is checked. Without an active task nothing is refused. A person who needs one of them lowers the level for that step with `supervisor level audit`.
- `strict`: `enforce`, plus PreToolUse denies a `Write`, `Edit`, `MultiEdit` or `NotebookEdit` outside the active task's contract scope, under the same rule completion applies to changed paths. This scope check covers those edit tools only: a shell command (`Bash`, `PowerShell`, Terminal Commander) or another MCP tool that writes outside the scope is not checked before it runs. `strict` adds three more rules, all in the Claude Code hooks:
  - **Forbidden effects.** Typed `forbidden_effects` entries are refused before they happen: `deny_path:<dir>` (an edit tool under that project path), `deny_git:<subcommand>` (for example `deny_git:push`) and `deny_argv_prefix:<words>` (a command, or a part of one between `;`, `&`, `|` or a newline, that starts with those words, for example `deny_argv_prefix:npm version`), `deny_git_push_target:<branch>` (a push that updates or deletes that branch) and `deny_git_force_push`. Untyped entries stay recorded for a reviewer only.
  - **Bug fixes.** A contract with a mandatory criterion whose `check` is `baseline_reproduces_reported_failure` is a bug fix. Its baseline must be a run that *failed* on the task's starting candidate (run it before changing the code); that receipt stays current after the fix changes the code. At `enforce` such a criterion is an ordinary check that must pass.
  - **Minimum tests.** A criterion may set `"min_tests": <n>`; its run then counts only when its test summary (cargo, jest, vitest or pytest) showed at least `n` passing tests. Only the count is kept, never the output, and a run with no summary in its output (a Terminal Commander job, for one) does not meet the minimum.

An unreadable `level` file runs the hook at `enforce` and journals why.

**Failing closed.** At `enforce` and `strict` (an unreadable `level` included), an error inside the hook adapter itself, such as an unreadable or unwritable gate file, unreadable prompt or task state, a contract that does not load or a corrupt cached answer, refuses instead of letting the call through (pending legacy migration is not an adapter error: it refuses every PreToolUse, read-only calls and `failure open` included, until `supervisor task migrate`): PreToolUse denies with `GATE: the gate fails closed: <error>`, and Stop and SubagentStop are refused. A Stop or SubagentStop that Claude Code already continued once (`stop_hook_active: true`) goes ahead instead, with `GATE: WARNING, this <event> went ahead NOT certified: <error>`, so a broken gate cannot loop the session (the plugin script follows the same rule). A prompt whose intent cannot be recorded is refused at UserPromptSubmit. As a defensive measure a fail-closed text never contains "no such file", "not found" or "missing"; they are reworded. Claude Code reads such text as non-blocking only on the plugin wrapper's exit-2 stderr path (`hooks/supervisor-hook.js`); the binary's JSON `decision: block` blocks either way, and ordinary refusals such as "missing C1" still block. A lost events.jsonl line never turns a refusal into a pass, and a call it would have allowed is refused with nothing it staged committed. The same holds after a tool has run: at PostToolUse a lost line discards that run's staged observation, so its check shows as pending at Stop and has to be run again. Payload text that is not JSON is still refused at PreToolUse, Stop and SubagentStop (its event name is read from the raw text; no level can be read, so it counts as governed); a lone UTF-16 surrogate escape such as `\ud800` in tool input is replaced before parsing, so such a call is judged like any other. Read-only calls (such as `Read`, `Grep` or `git status`; deliberately fewer than the gate lets through before a prompt is classified, since `intent_apply` changes gate state), PostToolUse, PostToolUseFailure and other UserPromptSubmit errors keep the advisory `GATE adapter error: <error>` message, so a broken build stays diagnosable; so do `off` and `audit`, a JSON payload with no `cwd` (no project is known), and unreadable stdin. A person who needs work to go on while the adapter is broken runs `supervisor level <level> --failure open --project <dir>`: this writes a `failure` file (`open`) beside the project's `level` file, every adapter error above is then answered with the advisory message, and the journal line records `failure open (advisory)`. `--failure closed` (also the default when the file is absent) restores failing closed. `supervisor level`, every project `supervisor level <level>` write and `supervisor status` print `failure: closed` or `failure: open (advisory)`; `status --json` carries `failure_mode`. `supervisor level off --project <dir>` from a plain terminal also works while the hooks fail closed, as long as the project's state directory resolves; a state directory whose `project.json` names another worktree does not, and is a known limit. The hook denies an agent's `supervisor level <level> ...` with or without `--failure`, as above.

The mode you ask for is the *requested* mode; the one a call actually runs at is the *effective* mode. Only Claude Code with the gate's PreToolUse, Stop and SubagentStop hooks can refuse, so anywhere else (the MCP server, another harness, Claude Code without those hooks) a requested `enforce` or `strict` runs as `audit`. `supervisor status --json` reports `requested_level`, `effective_level` and, when they differ, a `level_note` saying why; every journal line carries `requested_level` beside `effective_level`. MCP answers carry `answer_authority: "advisory"` for the answer itself; that is not the task's enforcement, which `status` reports (`mode` in MCP answers is kept for one release and means the same as `answer_authority`).

Every events.jsonl line, from the hook and from the MCP server, carries `effective_level` (the level the call ran at) and `level_source` (`project`, `global`, `default` or `unreadable`). `supervisor verdict --json` and `supervisor status --json` carry the same two keys, and status prints a `level: <name> (<source>)` line in its text output. When a `level` file is unreadable, the level is `enforce`, never `off` or `audit`, and the source is `unreadable`. At `off` the hook writes nothing: no journal line, no run record and no receipt, so a check run or claimed at `off` never counts as observed. The verdict and status are where `off` shows. These keys are additive; `schema_version` and the journal's `v` stay 1, and they are not part of the gate.json hash chain. The hook denies agent tool calls that write either `level` file or run `supervisor level <level>`; this is the same text-matching guard as for the gate's state, with the same same-user limits. `off` and `audit` are user-chosen bypasses of enforcement.

## Optional action policy

An optional project-wide `policy.json` in the resolved state directory can configure protected paths and branches, synthetic markers, exact denied tools and argv prefixes. No policy adds no such denials; malformed configured policy denies Claude `PreToolUse` until repaired. This is separate from the contract's `forbidden_effects`, which the core does not enforce.

The policy inspects named Claude hook routes, including simple shell commands and supported Terminal Commander argv and shell fields. It does not parse arbitrary shell programs. Compound commands, scripts, expansion, unknown connectors, later PTY input, remote or environment-overridden Git context and several path forms remain gaps. Persistent-shell policy is schema/replay tested on the Windows host, where TC shell sessions were unavailable, not live verified. See the [policy schema, interception table and limits](docs/research/2026-10-30-dogfood-rollout/COVERAGE.md#explicit-policy-configuration).

## Build and test

Install the Rust toolchain specified in [rust-toolchain.toml](rust-toolchain.toml). Set a build directory outside the checkout (replace the example path):

```sh
export CARGO_TARGET_DIR=/path/outside/checkout/coding-supervisor-target
```

In PowerShell, use `$env:CARGO_TARGET_DIR = 'C:\path\outside\checkout\coding-supervisor-target'`. Then run:

```sh
cargo build --release --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace contains the dependency-free core, Claude Code adapter, MCP server and CLI. The release build produces `supervisor`, `supervisor-mcp` and `supervisor-claude-code`. The last also provides `measure` for adapter journals. See [CI](.github/workflows/ci.yml) for the checks.

## Further work

The current integration does not enforce completion through supervisor hooks on the other harnesses. Dependency-aware receipt reuse, broader effect observation and additional harness adapters remain separate work; see the [first-slice plan](docs/research/2026-10-28-completion-gate-first-slice/PLAN.md) and the coverage documents for the boundaries of current evidence.

## License

[MIT](LICENSE).
