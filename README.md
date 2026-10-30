# coding-supervisor

**A portable coding-agent supervisor that coordinates delegation and requires verifiable, current evidence before accepting task completion.**

It sits beside an existing coding harness. The harness keeps the model, the tools and the execution environment. The supervisor keeps one small, deterministic rule: a task is complete only when every mandatory check has a current receipt, bound to the exact candidate that was checked, produced by a command the harness actually observed. A worker saying "tests passed" is not evidence. A worker saying it would run the tests is not evidence. A receipt for yesterday's version of the file is not evidence.

> [!IMPORTANT]
> **Status, as of 2026-10-29: first slice complete, installable on five harnesses, enforcing on one, and has supervised two real tasks on Claude Code, the second with the enforcing path closed end to end.** The core and the Claude Code hook adapter exist, are tested, and were proven live in an isolated fixture on Claude Code 2.1.312. The command-line installer and the MCP server install into Claude Code, Codex CLI, Cursor, OMP and Grok Build; each harness has a results file with a measured mode line under `docs/research/2026-10-29-installable/`. **On every harness except Claude Code the gate is advisory: receipts arrive through MCP and are caller-asserted, so a chair can assert a receipt for a command it never ran.** On Claude Code it has supervised two real tasks on the owner's lab repository. In the second, a worker's handback was refused at `SubagentStop` until it reran the contract's commands bare, and the chair's own completion was accepted at `Stop` on runs the hooks observed, with no MCP receipts (`docs/research/2026-10-29-first-real-project-use/RESULTS.md`). No claim is made that it improves verified work per token: the after-cohort is 11 dispatches, too few for that claim, and no PASS in either cohort is backed by a receipt.

## What exists

| Part | Where | What it does | Verified by |
|---|---|---|---|
| Core gate | `src/lib.rs` | Records `TaskContract`, `Candidate`, `VerificationReceipt`, `Finding`, `Checkpoint`; a ten-state task machine; freshness invalidation when the candidate changes; a bounded repair budget; typed errors, no panics, no `unsafe`, **no dependencies** | 34 scenario tests in `tests/scenarios.rs`, including a declared state-by-event transition matrix |
| Claude Code adapter | `adapters/claude-code/` | The `supervisor-claude-code` binary, run by Claude Code hooks. Parses the verdict header of a worker (at `SubagentStop`) and of the chair (at `Stop`), links cited checks to commands the harness observed, drives the core, refuses unsupported completion claims, denies writes to its own state, detects tampering with a hash chain, journals every decision | 8 parser, 20 protection, 29 replay and 6 measure tests (as_of 2026-10-29, `docs/research/2026-10-29-first-real-project-use/RESULTS.md`); three live fixture runs documented in `docs/research/2026-10-28-completion-gate-first-slice/T3-RESULTS.md` and `T4-T5-RESULTS.md` |
| Measurement reader | `supervisor-claude-code measure` | Turns the adapter's journals into gate-derived numbers: dispatches, supported and unsupported passes, linked receipts, refusals by reason, accepted tasks, worker tokens per accepted task | `T6-RESULTS.md` |
| Fixture | `fixtures/claude-code-enforcement/` | A throwaway project with its own hook settings, a contract, controlled fake workers and captured hook events, used for every live test | the results files above |
| MCP server (advisory) | `adapters/mcp/` | The `supervisor-mcp` binary: eight tools over the same gate state as the hooks (`task_start`, `candidate_observe`, `receipt_record`, `finding_raise`, `repair_request`, `accept_try`, `task_complete`, `task_state`), newline-delimited JSON-RPC on stdin and stdout. Answers; never blocks. Receipts are caller-asserted | 22 tests (6 protocol, 15 replay, 1 journal; as_of 2026-10-29); one live Claude Code session in `A-RESULTS.md` (S3b) |
| Command-line installer | `cli/` | The `supervisor` binary: `init`, `install --harness`, `hook`, `mcp`. Writes the two skills and the gate's registration into a project for one of five harnesses | 17 tests per platform (15 install, of which one runs only on Windows and one only on Unix, and 2 init; as_of 2026-10-29); Claude Code installed configuration run live in `A-RESULTS.md` |
| Skill templates | `skills/` | `supervisor-chair` and `supervisor-worker`, rendered per harness by `install` | rendered text shown in the install runs below |
| Install fixture | `fixtures/claude-code-install/` | The runner, contract and captured hook and tool journals from the live sessions through the installed configuration | `docs/research/2026-10-29-installable/A-RESULTS.md` |
| CI and release workflows | `.github/workflows/ci.yml`, `release-please.yml`, `release.yml` | CI on every push and pull request: format, lint, tests, and a check that the core has no dependencies. Releases are automatic from main: release-please opens and merges the release PR and tags it. Release on that `v*` tag: native release builds for two targets (Windows and Linux x86_64), archives with checksums, a GitHub release | CI green on every push to main. Release: two targets, published as `v0.1.0-rc2` (release run 40876811539, 2026-10-29): both archives published with checksums; `sha256sum -c SHA256SUMS` passed on the downloaded files and the Windows `supervisor.exe` started; the Linux binaries were not executed (as_of 2026-10-29) |

The core has no dependencies; its dependency tree is one line. The only dependency in the workspace is `serde_json`, used by the Claude Code adapter (to read hook payloads from stdin), by the MCP server and by the command-line installer.

## Install and use

The local sequential-dogfood changes (2026-10-30) add explicit task lifecycle,
dispatch-bound asynchronous receipts and `supervisor status`. Two actual Claude
tasks completed consecutively through observed TC jobs and Stop acceptance.
Verification, candidate identities and the repaired first live failure are
tracked in [RESULTS.md](docs/research/2026-10-30-dogfood-rollout/RESULTS.md).
The independent audit rejected the first delivery's full acceptance claim;
[repair and re-audit evidence](docs/research/2026-10-30-dogfood-rollout/REPAIR-RESULTS.md)
tracks the corrected command provenance, durable recovery, replay, migration,
policy and status behavior. Earlier test success alone is not acceptance evidence.
Use the [current operation and recovery guide](docs/research/2026-10-30-dogfood-rollout/OPERATIONS.md)
for these commands. The captured installer output below describes the earlier
release; it is historical evidence, not current activation guidance. Installation
and ordinary conversation do not require an active task.

Status, as of 2026-10-29: Claude Code is the only harness run live through the installed configuration (`docs/research/2026-10-29-installable/A-RESULTS.md`, throwaway project). For Codex CLI, Cursor, OMP and Grok, install writes files whose locations come from each harness's documentation (`docs/research/2026-10-29-installable/HARNESS-LOCATIONS.md`); none of those harnesses has been recorded loading them yet. Phase B is that check.

### The three binaries

| Binary | Commands | What it does |
|---|---|---|
| `supervisor` | `init [--project <dir>]`, `install --harness <name> [--project <dir>] [--home <dir>] [--binary <path>] [--dry-run]`, `hook`, `mcp [--project <dir>]` | The single entry point. `hook` behaves exactly as `supervisor-claude-code hook`; `mcp` serves the gate as an MCP server |
| `supervisor-mcp` | `[--project <dir>]` | The MCP server on its own; same as `supervisor mcp` |
| `supervisor-claude-code` | `hook`, `measure [--json] <path>...` | The Claude Code hook adapter and the journal reader. `measure` exists only here |

Harness names for `--harness`: `claude-code`, `codex`, `cursor`, `omp`, `grok`.

### Get the binaries

```text
npm install -g coding-supervisor
supervisor init --project <dir>
supervisor install --harness claude-code --project <dir>
```

The npm package `coding-supervisor` is a small launcher. npm installs alongside it the platform package for your system, `coding-supervisor-windows-x64` or `coding-supervisor-linux-x64`, which holds the three binaries; `supervisor` on your PATH runs the native `supervisor` from it. Windows and Linux x86_64 only. Releases publish under the npm dist-tag `latest`; the earlier release candidates are under `next`. Releases are automatic from main: on every push, `.github/workflows/release-please.yml` checks the commit subjects (conventional: `feat:`, `fix:`, `docs:`, `ci:` and the other types release-please knows), and release-please opens a release PR with the version bump and `CHANGELOG.md`, which the workflow merges at once; release-please then tags the merge commit, and the tag runs `release.yml`. Nobody tags by hand. The first automatic release is `v0.1.0`, under the npm dist-tag `latest`; until it exists this is unverified (as_of 2026-10-29). Evidence: release run 40881138321 (`v0.1.0-rc3`, 2026-10-29) published all three packages at `0.1.0-rc3`; npm set both `next` and `latest` to it on each package's first publish, and `npm install -g coding-supervisor@0.1.0-rc3` from the registry into a temp prefix ran `supervisor --help` with exit 0 on Windows. The Linux package has not been installed or run (as_of 2026-10-29). Each package also lists a version `0.0.0-stage`: npm's placeholder from a package's first publish, not a release.

- **Download.** Fetch the archive for your platform from the [Releases page](https://github.com/special-place-ai-heaven/coding-supervisor/releases): `coding-supervisor-<tag>-x86_64-pc-windows-msvc.zip` for Windows or `coding-supervisor-<tag>-x86_64-unknown-linux-gnu.tar.gz` for Linux (x86_64 only; no other platform is built), together with `SHA256SUMS`. Verify the archive against `SHA256SUMS` before unpacking: on Linux `sha256sum -c SHA256SUMS --ignore-missing`; on Windows compare `(Get-FileHash <archive> -Algorithm SHA256).Hash` in PowerShell with the archive's line in `SHA256SUMS`. Unpack it into a permanent folder; each archive holds `supervisor`, `supervisor-mcp` and `supervisor-claude-code`. Then run `supervisor init` and `supervisor install` as described below. The first published release is the pre-release `v0.1.0-rc2` (release run 40876811539, 2026-10-29): both archives published with checksums; `sha256sum -c SHA256SUMS` passed on the downloaded files and the Windows `supervisor.exe` started; the Linux binaries were not executed.
- **From source.** With the toolchain pinned in `rust-toolchain.toml` (Rust 1.99.1): `cargo build --release --workspace`. The binaries are in `target/release/` under your `CARGO_TARGET_DIR`, or under `target/` if it is unset.

Put the binaries where they will stay. `install` writes the absolute path of the running `supervisor` into the hook and MCP files (override with `--binary <path>`), so moving the binary later breaks the installed configuration.

### Quickstart, any harness

1. `supervisor init --project <project>` writes a starting contract into the gate's state directory (outside the project) and never overwrites one. The runs below used `SUPERVISOR_STATE_DIR=C:\temp\sup-throwaway\state`, a throwaway project at `C:\temp\sup-throwaway\proj`, and the debug build; output as printed on 2026-10-29, exit 0:

   ```text
   project: C:\temp\sup-throwaway\proj
   state directory: C:\temp\sup-throwaway\state\C--temp-sup-throwaway-proj (rule: env)
   contract exists, left unchanged: C:\temp\sup-throwaway\state\C--temp-sup-throwaway-proj\contract.json
   Edit the contract's goal, criteria and scope there, then register the gate with one of:
     supervisor install --harness claude-code --project C:\temp\sup-throwaway\proj
     …
     supervisor install --harness grok --project C:\temp\sup-throwaway\proj
   ```

   On a fresh state directory it prints `contract written:` instead. The starting contract (from `cli/src/init.rs`) has criteria C1 to C4 with only C2 and C3 mandatory, scope `src/` and `tests/`, and two repair rounds. Each criterion carries a `command`: one string, or a list of accepted strings, such as `"command": "cargo test --workspace"`. A receipt for that criterion must cite exactly one of them (compared after trimming), otherwise the gate refuses it; a contract without a command for every criterion does not load. The starting commands are placeholders. Edit its goal, criteria with their commands, and scope.
2. `supervisor install --harness <name> --project <project>`. Add `--dry-run` to print what would change and write nothing; the dry run also prints the state of all five harnesses in the project (`not-installed`, `absent`, `present-current`, `present-stale` or `malformed`). Running it again is safe: when the registration already runs this binary (the same file, through symlinks or a `.cmd` wrapper) and every file is up to date, it prints `state: present-current` and `unchanged` and writes nothing. The gate's own hooks from an earlier binary path are replaced, one per event. A config file that does not parse is never written, and its text and parse error are not printed. Files are written to a temporary file beside them and renamed over; a file another process holds open makes install stop with its path and the advice to close the harness session (install never stops processes).
3. Do the manual step install prints at the end. Install never performs it.

Install writes, per harness, two skills (`supervisor-worker`, `supervisor-chair`) into the project's skills directory and the registration of the MCP server `supervisor` (run as `<binary> mcp --project <project>`) into the project's MCP file. Only Claude Code also gets hooks.

### Claude Code

Writes `.claude/skills/<skill>/SKILL.md` (two files), `.mcp.json`, and `.claude/settings.json` with the six hook events (`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStop`, `Stop`) and a `permissions.allow` entry `mcp__supervisor`. If a `.gitignore` already exists and does not list it, it also adds `.supervisor/`.

Dry run into an empty directory (the `<<skill text>>` markers replace the printed file bodies; hook entries trimmed to one event with `…`):

```text
install Claude Code into C:\temp\sup-throwaway\fresh (dry run)
--- /dev/null
+++ C:\temp\sup-throwaway\fresh\.claude\skills\supervisor-worker\SKILL.md
<<skill text>>
--- /dev/null
+++ C:\temp\sup-throwaway\fresh\.claude\skills\supervisor-chair\SKILL.md
<<skill text>>
--- /dev/null
+++ C:\temp\sup-throwaway\fresh\.mcp.json
+{
+  "mcpServers": {
+    "supervisor": {
+      "args": [
+        "mcp",
+        "--project",
+        "C:/temp/sup-throwaway/fresh"
+      ],
+      "command": "C:/Target/coding-supervisor/main/target/debug/supervisor.exe",
+      "type": "stdio"
+    }
+  }
+}
--- /dev/null
+++ C:\temp\sup-throwaway\fresh\.claude\settings.json
+{
+  "hooks": {
+    …
+    "Stop": [
+      {
+        "hooks": [
+          {
+            "command": "C:/Target/coding-supervisor/main/target/debug/supervisor.exe hook",
+            "type": "command"
+          }
+        ]
+      }
+    ],
+    …
+  },
+  "permissions": {
+    "allow": [
+      "mcp__supervisor"
+    ]
+  }
+}
dry run: nothing written
note: JSON files are rewritten with two-space indentation; keys are written in sorted order.
```

The same command against the already installed throwaway project (exit 0; printed by the build of 2026-10-29 before the state scan existed, which now prints `state: present-current`, the five harness states and `unchanged` instead of `no changes`):

```text
install Claude Code into C:\temp\sup-throwaway\proj (dry run)
no changes: every file is already up to date
dry run: nothing written
```

Manual step, as printed:

```text
manual step (not performed): Trust the project folder once: start Claude Code interactively here and accept the trust dialog. Until then Claude Code ignores the `permissions.allow` entry for `mcp__supervisor` that install wrote, and every gate tool call is refused (seen live, A-RESULTS.md session S3c). Interactive sessions also ask once to approve the project MCP server `supervisor` from .mcp.json: approve it. `claude -p` loads it without asking (section 1, row 3).
```

### Codex CLI

Writes `.agents/skills/<skill>/SKILL.md` (two files) and `.codex/config.toml`. No hooks. Dry run into an empty directory with `--binary supervisor` (skill bodies omitted):

```text
install Codex CLI into C:\temp\sup-throwaway\fresh (dry run)
…
--- /dev/null
+++ C:\temp\sup-throwaway\fresh\.codex\config.toml
+[mcp_servers.supervisor]
+command = "supervisor"
+args = ["mcp", "--project", "C:/temp/sup-throwaway/fresh"]
dry run: nothing written
```

Against the throwaway project, which had been installed before the chair skill changed, install shows the update as a diff (exit 0):

```text
install Codex CLI into C:\temp\sup-throwaway\proj (dry run)
--- C:\temp\sup-throwaway\proj\.agents\skills\supervisor-chair\SKILL.md
+++ C:\temp\sup-throwaway\proj\.agents\skills\supervisor-chair\SKILL.md
-1. Observe the candidate: `candidate_observe` with the current commit and a
-   digest of the uncommitted changes, plus the changed paths. If the files
-   changed since the last receipt, the gate marks those receipts stale.
+1. Observe the candidate: call `candidate_observe` with no identity fields.
+   The gate reads git itself (the current commit, a digest of the uncommitted
+   changes and the changed paths), the same way its hooks do. Pass
+   `base_commit`, `dirty_digest` and `changed_paths` only on a harness
+   without git. If the files changed since the last receipt, the gate marks
+   those receipts stale.
dry run: nothing written
```

Manual step, as printed (the home directory was `--home C:\temp\sup-throwaway\home`):

```text
manual step (not performed): Trust the project so Codex loads .codex/config.toml: add these lines to C:/temp/sup-throwaway/home/.codex/config.toml
    [projects.'C:\temp\sup-throwaway\proj']
    trust_level = "trusted"
  (section 2, row 3; the Windows spelling of that key follows this machine's config, not a documented rule: gap G3).
```

### Cursor

Writes `.cursor/skills/<skill>/SKILL.md` (two files) and `.cursor/mcp.json` (the same JSON shape as `.mcp.json` above). No hooks. Against the throwaway project, exit 0: the same chair-skill hunk as for Codex, with the path `C:\temp\sup-throwaway\proj\.cursor\skills\supervisor-chair\SKILL.md`, then `dry run: nothing written`. Manual step, as printed:

```text
manual step (not performed): Approve the server once: run `cursor-agent mcp enable supervisor` in the project, or pass `--approve-mcps` in scripted runs (section 3, row 3; whether this covers project servers only is gap G4a). On this machine the bare `agent` command is not Cursor; use `cursor-agent`.
```

### OMP

Writes `.omp/skills/<skill>/SKILL.md` (two files) and `.omp/mcp.json`. No hooks: OMP hooks are code modules, and install does not write one. Against the throwaway project, exit 0: the same chair-skill hunk for `…\.omp\skills\supervisor-chair\SKILL.md`, then `dry run: nothing written`. Manual step, as printed:

```text
manual step (not performed): None: OMP loads project MCP servers without approval (section 4, row 3).
```

### Grok Build

Writes `.grok/skills/<skill>/SKILL.md` (two files) and `.grok/config.toml` (same TOML table as Codex). No hooks: the documentation says Grok's hooks cannot refuse completion. Against the throwaway project, exit 0: the same chair-skill hunk for `…\.grok\skills\supervisor-chair\SKILL.md`, then `dry run: nothing written`. Manual step, as printed:

```text
manual step (not performed): Trust the folder once so Grok loads project MCP servers: start `grok --trust` in the project or run `/hooks-trust` (section 5, row 3; `--trust` is missing from the CLI reference: gap G5).
```

### Limits to know before relying on it

- **Receipts through the MCP tools are caller-asserted.** The first line of `adapters/mcp/src/lib.rs` declares the mode: `MODE: advisory via MCP on every harness; the server answers and never blocks; receipts are caller-asserted (the server does not observe commands). On Claude Code, the hook adapter ignores these receipts when it decides whether a completion may stop.` The server stores such a receipt with an `output_ref` starting `mcp-asserted:`. In the one live run that exercised this (`A-RESULTS.md`, S3b), the MCP path reached `completed` and the Claude Code `Stop` hook still refused the completion claim for a missing receipt. On the other four harnesses no hook is installed, so nothing there blocks.
- **Claude Code must trust the folder before the allow rule works.** In the run through the installed configuration alone (`A-RESULTS.md`, S3c) Claude Code printed that it was ignoring the `permissions.allow` entry because the workspace was not trusted, and denied the gate's tool call, while the hooks in the same file still ran. The run that reached the tools (S3b) added `--allowedTools mcp__supervisor` on the command line, which is not part of the installed configuration. A run of the installed configuration alone in a trusted folder has not been done.
- **JSON files are rewritten with sorted keys.** Install reads an existing `.mcp.json`, `.claude/settings.json` or `.cursor/mcp.json`, keeps what is there, adds its entries, and writes the whole file back with two-space indentation and keys in sorted order. Expect a reordered diff on a hand-written file. The TOML files are edited in place and only a plain `[mcp_servers.supervisor]` header line is recognised.
- **Nothing is written outside the project and home.** Install writes only under the project directory and refuses any path outside the project and the home directory (`--home`, default the user profile). The user-level harness configuration is never touched: user-level skills directories and trust settings appear only as printed notes and manual steps. The gate's own state lives in the state directory chosen by the rule described under "Build and test".

## How it works

```mermaid
flowchart LR
    subgraph harness [Claude Code]
        W[Worker agent] -->|last message| SS[SubagentStop hook]
        C[Chair] -->|final message| ST[Stop hook]
        C -->|Write / Edit / Bash / connector call| PT[PreToolUse hook]
    end
    SS --> A[supervisor-claude-code hook]
    ST --> A
    PT --> A
    A -->|resume, replay receipts, derive candidate from git| G[Core gate]
    G -->|accept / refuse with typed reason| A
    A -->|checkpoint + hash chain| S[(state dir outside the project)]
    A -->|one line per decision| J[(journal v1)]
    J --> M[supervisor-claude-code measure]
```

A worker ends its turn with a fixed header:

```
VERDICT: PASS | FAIL | UNVERIFIED
CHECKS RUN: <n>
<command>  exit <code>
...
```

The adapter turns each cited command into a receipt only if the command is one the contract names for that criterion and the harness observed that command run under the same agent. A `PASS` with no linked receipt is refused at `SubagentStop`. A chair that claims completion while the gate is not in `completed` is refused at `Stop`. A write while the task is terminal, or any attempt to touch the gate's own state through any known tool, is denied at `PreToolUse`.

## What the adapter can and cannot enforce

The first line of `adapters/claude-code/src/lib.rs` declares the mode, measured rather than assumed:

- **Enforcing** at `SubagentStop`, `Stop` and `PreToolUse`. The two stop hooks are bounded by the harness's cap of eight consecutive continuations; the adapter refuses, then releases visibly rather than looping.
- **Advisory** at `PostToolUse`, which can annotate a foreground worker's result but cannot stop the chair, and which never fires for a background dispatch.
- **No event exists** at the moment the chair accepts a worker's claim. That moment is implicit in the model's reasoning.

> [!WARNING]
> These measures raise the bar and detect tampering. They do **not** make the adapter enforcing against an adversarial agent that holds a shell as the same OS user. Such an agent can spell the state path differently, delete the state, edit the hook settings, or replace the binary. The guard matches on tool name and known input fields only; a tool outside its list, or a field it does not read, is uncovered. Treat the gate as protection against sloppy or self-deceiving agents, not hostile ones.

## What is deliberately not done

| Item | State |
|---|---|
| `forbidden_effects` on the contract | Declared, **not enforced**. Nothing observes effects yet. Enforcement needs adapter-supplied observations and an effect history that survives candidate changes and resume. |
| Blocked as a resumable paused state | The core treats `blocked` as terminal. The inherited specification lists it under "terminal/paused". An explicit unblock operation is a declared gap awaiting a decision. |
| Repair rounds and findings through the adapter | The core models them; the adapter emits no repair or finding events yet. |
| Candidate identity from uncommitted content | The core compares identities; the adapter derives one from git. Detecting real content changes is the adapter's job and is tested only with synthetic identities in the core. |
| Delegation (which agent should do the work) | Advisory only, and not part of this slice. An existing routing engine on the author's machine is reused where relevant; nothing here routes. |
| Other harnesses | Only Claude Code has a hook adapter. For Codex CLI, Cursor, OMP and Grok Build, install writes the two skills and an MCP registration (advisory, caller-asserted receipts) and no hooks. Each was investigated the same way as Claude Code, with a results file and a mode line: OMP and Claude Code measured with a live model; Codex CLI and Grok Build with scripted models (quota and sign-in pending); Cursor advisory only, with the CLI confirmed to load the registration and list the eight tools but no hook observed firing. A hook adapter for any of them comes only with its own measured mode line. |

## What still needs to happen

1. **Use it on a real project.** Delete any gate state written before the hash chain existed, run `supervisor init` and `supervisor install --harness claude-code` for the project, trust the folder in Claude Code, edit the contract, and let it supervise real work. This is the first time the gate meets real workers.
2. **Measure.** Compare sessions with and without the gate on matched tasks: accepted tasks, defects escaped, unsupported passes, retries, tokens and wall time, with failed attempts counted. Gate-derived numbers from the journal stay separate from transcript-derived numbers.
3. **Decide blocked-state semantics** and implement unblock if paused is chosen, with the rule that unblocking never grants acceptance, resets the repair budget, or restores stale evidence.
4. **Effects.** Design the observation contract before implementing enforcement of `forbidden_effects`.
5. **Close two receipt gaps found by review** (`docs/research/2026-10-29-receipt-relevance-and-run-time-freshness/PROPOSAL.md`). Gap 1, in progress on main: a receipt today proves a command ran, not that the contract's command ran, so each criterion gains a required `command` and a mismatched citation is refused by name. Gap 2, sequenced after the first real-project use: bind each observed run to the candidate at run time, so a check run before an edit cannot be cited after it.
6. **Finish the release.** Releases are automatic from main: on every push, `.github/workflows/release-please.yml` checks the commit subjects (conventional: `feat:`, `fix:`, `docs:`, `ci:` and the other types release-please knows), and release-please opens a release PR with the version bump and `CHANGELOG.md`, which the workflow merges at once; release-please then tags the merge commit, and the tag runs `release.yml`. Nobody tags by hand. The first automatic release is `v0.1.0`, under the npm dist-tag `latest`; until it exists this is unverified (as_of 2026-10-29). CI is proven on main (Linux on the owner's self-hosted runners, Windows on a hosted runner). The release workflow published the pre-release `v0.1.0-rc2` (release run 40876811539, 2026-10-29): both archives published with checksums; `sha256sum -c SHA256SUMS` passed on the downloaded files and the Windows `supervisor.exe` started; the Linux binaries were not executed. The npm publish ran with `v0.1.0-rc3`: release run 40881138321 (`v0.1.0-rc3`, 2026-10-29) published all three packages at `0.1.0-rc3`; npm set both `next` and `latest` to it on each package's first publish, and `npm install -g coding-supervisor@0.1.0-rc3` from the registry into a temp prefix ran `supervisor --help` with exit 0 on Windows. The Linux package has not been installed or run. Still open: running the Linux binaries, and the first automatic release `v0.1.0`.
7. **Live re-runs where quotas or sign-in blocked them.** Codex CLI with a live model after 2026-11-04, Cursor after its weekly reset, Grok Build after a sign-in. Until then their mode lines say "scripted model" or "advisory".

## Build and test

Requires the pinned toolchain in `rust-toolchain.toml` (Rust 1.99.1). Build artifacts are kept outside the checkout on the author's machine; set `CARGO_TARGET_DIR` as you prefer. The workspace has four crates: the core, `adapters/claude-code`, `adapters/mcp` and `cli`.

```bash
cargo build --release --workspace   # supervisor, supervisor-mcp, supervisor-claude-code
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo tree -p coding-supervisor -e normal,build,dev --target all   # one line: the core has no dependencies
```

These are the commands the CI workflow runs (it checks the one-line tree with `--edges normal,build,dev`). The CI workflow has not been observed running.

Run the adapter as a hook (one payload on stdin, at most one decision on stdout); `supervisor hook` does the same:

```bash
supervisor-claude-code hook
```

Serve the gate over MCP (the project defaults to the current directory):

```bash
supervisor mcp --project <dir>
```

Read gate-derived numbers from one or more journals:

```bash
supervisor-claude-code measure [--json] <path>...
```

State lives, in order of preference, under `SUPERVISOR_STATE_DIR`, then the platform's local app data or XDG state directory under `coding-supervisor/state/<project key>/`, and only as a last resort under `.supervisor/` in the working directory. The rule chosen is journaled on every event.

## Repository map

| Path | Content |
|---|---|
| `CLAUDE.md` | Live truth for this repository: definition, scope, principles, current state |
| `HANDOVER.md` | Self-contained state for the next working session, with the paths and hashes of the research this builds on |
| `docs/research/2026-10-28-completion-gate-first-slice/` | The plan and the results of each task, with commands and exit codes |
| `docs/archive/inherited-specs/` | The two specifications this inherits from, preserved byte-identical |
| `docs/research/2026-10-29-installable/` | Phase A results (`A-RESULTS.md`), the harness location survey (`HARNESS-LOCATIONS.md`), and in Phase B one results file per harness |
| `docs/research/2026-10-29-harness-capability-survey/` | The earlier capability survey for Codex CLI, Cursor, OMP and Grok Build |
| `src/`, `tests/` | The core and its scenarios |
| `adapters/claude-code/` | The Claude Code adapter crate |
| `adapters/mcp/` | The MCP server crate (`supervisor-mcp`), with its tool definitions in `src/tools.json` |
| `cli/` | The `supervisor` crate: `init`, `install`, `task`, `status`, `hook`, `mcp` |
| `skills/` | The chair and worker skill templates that `install` renders |
| `fixtures/claude-code-enforcement/` | The live-test fixture and captured hook events |
| `fixtures/claude-code-install/` | The runner, contract and captures for the live sessions through the installed configuration |
| `.github/workflows/` | `ci.yml` (every push and pull request), `release-please.yml` (every push to main: commit subjects, release PR, tag) and `release.yml` (on `v*` tags); `release-please.yml` has not run yet |

## Provenance

This repository grew out of a study of when to fold specialist agents into one agent loop, applied to coding. The conclusion that shaped it: in the author's own session corpus, the independent verifier caught real defects and was worth keeping, but one accept in seven was issued with no command run, and almost no judge cited the command it ran against the claim it supported. The gate exists to make that impossible by construction rather than by instruction. The research and measurement that led here live in a separate repository on the author's machine and are referenced from `HANDOVER.md` by path and hash.

License: MIT.

## What works

✅ Works today    🚧 Being wired up    💭 Strong opinions, pending code

| Status | Capability | Where |
|---|---|---|
| ✅ | Deterministic completion gate: contract, candidate, receipts, typed refusals, freshness, bounded repair; no model, no dependencies | core, `src/lib.rs` |
| ✅ | Claude Code refusal at Stop, SubagentStop and supported PreToolUse paths; hash-chain tamper detection; same-user bypasses remain possible | `adapters/claude-code` |
| ✅ | A receipt must cite the exact command the contract names for its criterion | gap 1 |
| ✅ | A chair working alone is judged by its own CHECK header at Stop | Stop path |
| ✅ | Install for Claude Code, Codex CLI, Cursor, OMP and Grok; one entry per hook event, scan before write, atomic writes | `supervisor install` |
| ✅ | MCP server with the gate's tools on all five harnesses (advisory: receipts are caller-asserted) | `adapters/mcp` |
| ✅ | `npm install -g coding-supervisor` on Windows and Linux; releases cut automatically from main | release-please, npm |
| ✅ | Local dogfood: dispatch-time candidate/cwd/task identity and observed asynchronous TC completion, including restart and stale-result refusal | `observation.rs`; rollout RESULTS.md |
| 🚧 | No header needed: the gate picks the checks from the agent's own runs, and the latest run on the current files decides | derived receipts |
| ✅ | Local dogfood: explicit sequential tasks, immutable archives, recovery, inactive conversation and truthful `supervisor status` | CLI; rollout OPERATIONS.md |
| 🚧 | Levels off, lite, full, ultra | separate autonomous-gate roadmap |
| 🚧 | No setup per project: contract inferred from the repo's declared checks, a new task per session, required checks shown at session start | autonomous gate, phase 4 |
| 🚧 | Claude Code plugin with `/supervisor <level>` and `/supervisor status` | autonomous gate, phase 5 |
| 💭 | Enforcement on Codex, Cursor, OMP and Grok; today they have no hook the gate can block from | per-harness adapters |
| 💭 | Changes outside the project folder, for example an installed tool the fix edits, observed as part of the candidate | extra observed roots |
| 💭 | Compound commands such as `cmd; echo $?` recorded with the real stage's exit code, not the last one | run records |
| 💭 | The rule that keeps hook-linked receipts after an MCP completion moves into the core, or is deleted | adapter rule from `fa80d3c` |
| 💭 | Evidence that the gate improves verified work per token; two real tasks so far, too few to claim it | measurement |

The remaining roadmap rows are outside this local rollout; no release is implied.
