# coding-supervisor

coding-supervisor is a completion gate for coding agents. It records a task contract, candidate and verification evidence, and evaluates whether the required checks support completion. It runs alongside a coding harness; the harness supplies the agent and tools.

The core is deterministic. A receipt must match a criterion's command and the candidate it checked. A changed candidate makes affected evidence stale. Repair rounds are bounded and unresolved work has explicit outcomes. The Claude Code hook adapter can refuse unsupported completion on its covered paths; the MCP interface is advisory and its receipts are caller assertions.

## Install

The npm launcher and native packages support Windows and Linux x86_64:

```sh
npm install -g coding-supervisor
supervisor --help
```

Alternatively, download the archive for your platform and its checksums from the [Releases page](https://github.com/special-place-ai-heaven/coding-supervisor/releases), or build from source below. Keep the executable at a stable path: the installer records its path in the harness configuration. Version 0.1.1 is published; see the [release workflow](https://github.com/special-place-ai-heaven/coding-supervisor/actions/runs/41041057353). The later [main CI run](https://github.com/special-place-ai-heaven/coding-supervisor/actions/runs/41041987970) passed on Linux and Windows; it includes a test-fixture fix after the release.

From the project you want to supervise, register your harness:

```sh
supervisor install --harness claude-code --project <project> --dry-run
supervisor install --harness claude-code --project <project>
supervisor status --project <project> --harness claude-code
```

Replace `claude-code` with `codex`, `cursor`, `omp` or `grok` for installation. Follow the manual trust or approval steps printed by the installer. Installation registers the MCP server and two skills; only Claude Code receives supervisor hooks. It does not activate a task. The installer does not stop other processes; if a file is held open, close the harness session and retry. A malformed configuration is not rewritten.

## Ordinary tasks (development candidate)

The autonomous workflow in this checkout is not yet a published npm release.
After installing the candidate integration and reloading the harness, give
ordinary tasks. The agent interprets the request and proposes a scoped contract;
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

Legacy state requires explicit selection after checking its ownership:

```sh
supervisor task migrate --project <project> --legacy <old-state-directory>
```

Migration preserves the source. Each physical worktree has its own state namespace; keep `SUPERVISOR_STATE_DIR` consistent across cooperating processes. For recovery and state locations, see the [operations guide](docs/research/2026-10-30-dogfood-rollout/OPERATIONS.md). Retain evidence when state is corrupt or tampered; rollover is not a tamper reset.

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

## Harness support

| Harness | Supervisor integration and evidence |
|---|---|
| Claude Code | Hooks can deny covered tool calls and refuse unsupported chair or worker completion when loaded. Two sequential real tasks on the repaired code completed with observed receipts; see [results](docs/research/2026-10-30-dogfood-rollout/REPAIR-RESULTS.md). Completion refusals have an adapter cap of three; the historical measured harness cap was eight. |
| Codex CLI | Installed MCP integration is advisory; no enforcing supervisor hook adapter. Installed connection was tested with a scripted model. See [Codex results](docs/research/2026-10-29-installable/codex-T3-RESULTS.md). |
| OMP | Installed MCP integration is advisory; no enforcing supervisor adapter. Its hook capabilities were investigated separately, and the installed MCP integration was exercised with a live model. See [OMP results](docs/research/2026-10-29-installable/omp-T3-RESULTS.md). |
| Grok Build | Installed MCP integration is advisory; no enforcing supervisor adapter. The installed connection and hook investigation used a scripted model. See [Grok results](docs/research/2026-10-29-installable/grok-T3-RESULTS.md). |
| Cursor | Installed MCP integration is advisory; no enforcing supervisor adapter. The registration listed tools after approval; model-driven verification was blocked by quota. See [Cursor results](docs/research/2026-10-29-installable/cursor-T3-RESULTS.md). |

Harness hook capabilities and a supervisor adapter are separate questions. A configured hook only enforces when the harness loads and executes it. Hook failures and timeouts may yield no denial; same-user access to hooks and state is not an unbypassable security boundary. See the [coverage matrix](docs/research/2026-10-30-dogfood-rollout/COVERAGE.md).

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
