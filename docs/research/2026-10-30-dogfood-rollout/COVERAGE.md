# Effective status and scoped action policy

As of 2026-10-30. This document separates action authorization from completion evidence. New cases below are deterministic fixtures against temporary state, not live model enforcement claims. Same-user writable hooks and policy files are not an unbypassable security boundary.

## Status contract

`supervisor status [--project <directory>] [--harness claude-code|codex] [--json]` defaults to Claude Code and reads a consistent project snapshot. It creates no task contract and does not run checks. Human and JSON reports distinguish:

- Reporting executable and package version from the configured MCP executable, whose version is unknown unless separately measured. A configured path existing on disk does not prove it is loaded.
- Registration, configured hook events, trust (unknown), observed hook/MCP connection, and model-session loading (unknown). `mcp-initialize` is shared-project evidence with unknown harness attribution; `claude-hook` identifies the Claude hook path. Neither proves a model loaded the server. The report does not probe a paid model session.
- Inactive project, active task identity/state, required legacy migration, and unknown/tampered state. An inactive project stays advisory and ordinary conversation needs no contract or tests.
- Asserted MCP receipts, live observed receipts, TC-reconstructed receipts with matching durable task/run/candidate/command/cwd/exit/time provenance, and unverified legacy or unmatched receipts. Counts are historical receipt provenance, not a statement that all receipts are current or all criteria pass. Human output includes the effective gate state and named completion paths as well as their evidence basis.
- Advisory Codex integration and conditional Claude enforcement on configured named hook paths. Configured hooks remain conditional on harness loading. Completion refusals have an adapter cap of three, inside the previously measured Claude harness cap of eight.

The JSON schema is version 1. Registration parsing never prints file contents or parser diagnostics. Codex parsing recognizes the installer's basic-string `[mcp_servers.supervisor]` command; unsupported command encodings stay unknown. Hook recognition accepts a direct supervisor executable followed by `hook`; an echo or compound command does not qualify. Unrecognized configured command values are withheld.

Status fixtures: `cli/tests/status.rs` covers fresh/inactive, registered but unloaded, connected inactive, active Claude, advisory Codex, invalid registration redaction, legacy migration, and a false hook-shaped echo. Unit cases in `cli/src/status.rs` cover human session-loading language and direct hook recognition.

## Explicit policy configuration

Policy is optional and project-wide at `<resolved project state>/policy.json`, separate from a task contract. Use `status --json` to find the state directory. No policy means no new denials. Policy does not reinterpret `forbidden_effects` in the completion contract.

Schema (all lists optional; unknown fields and invalid types are rejected):

```json
{
  "version": 1,
  "protected_paths": ["C:/fixture/protected"],
  "protected_branches": ["main"],
  "synthetic_secret_markers": ["SYNTHETIC_FIXTURE_MARKER_ONLY"],
  "denied_tools": ["mcp__fixture__publish"],
  "denied_argv_prefixes": [["npm", "publish"]]
}
```

Configure only deliberately protected cases. Paths must be absolute. Synthetic markers are fixture-only: **never copy real secrets into this file**. Denial messages name the rule category and never reproduce the marker, tool arguments, or parser input. An unreadable or malformed configured policy denies Claude `PreToolUse` until repaired; an absent policy permits routine work silently. No blanket approval prompt is added. Policy itself lives under the existing state-path guard.

Supported command recognition is intentionally bounded: structured argv or one simple command, paired quotes and whitespace, with no compound operators, expansion, redirection or embedded interpreter. It does not parse shell programs. Exact external argv prefixes preserve argument boundaries. `rm`, `rmdir`, `rd`, `del`, `erase`, and `Remove-Item` target overlap includes deletion of an ancestor of a configured path. Git protection recognizes explicit `push`/`branch` force flags with a configured branch target, and `reset`/`rebase` while the locally queried current branch is configured. It does not contact a remote.

## Action routes and evidence

**Deny** means the adapter emits a denial on the intercepted Claude `PreToolUse` input. The harness must load and execute that hook for it to enforce. **Allow** means no decision is emitted for ordinary permitted calls. **Unsupported** means no denial is promised. The existing state guard is deliberately broader than delete policy: it denies state references even in read commands.

| Route | State mutation/deletion | Configured protected-path deletion | Configured branch rewrite | Synthetic publication | Configured unauthorized external effect |
|---|---|---|---|---|---|
| Bash / PowerShell simple command | Deny state text references | Deny recognized delete target | Deny recognized Git form | Deny marker anywhere in tool input | Deny exact argv prefix |
| TC `run_and_watch`, `command_start_combed`, `pty_command_start` argv | Deny state text references | Deny direct delete argv | Deny direct Git argv | Deny marker | Deny exact argv prefix |
| TC `shell_exec` with `shell_line` | Deny state text references | Deny simple command; refuse relative deletion if cwd is unknown and paths are configured | Deny simple Git form; refuse context-dependent rewrite if cwd is unknown and branches are configured | Deny marker | Deny simple command prefix |
| TC `shell_session_exec` with `session_id` and `line` | Deny literal state references | Deny absolute overlap; refuse relative deletion when paths are configured because sticky cwd is unknown | Deny explicit protected branch targets; refuse context-dependent rewrite when branches are configured | Deny marker | Deny simple command prefix |
| Compatibility fixtures for `file_delete` / `file_remove` with `path` (not exposed by current daemon) | Deny state path | Deny absolute overlap; refuse relative deletion when configured paths and unknown cwd | Unsupported | Deny marker | Deny if exact tool configured |
| TC `file_write` / `file_edit` | Deny state path | Unsupported for destructive overwrite of a non-state protected path | Unsupported | Deny marker | Deny if exact tool configured |
| Direct `Write`, `Edit`, `NotebookEdit` | Deny state `file_path`/`notebook_path` | Unsupported for non-state overwrite/deletion | Unsupported | Deny marker | Deny if exact tool configured |
| Other exposed MCP mutation tool | Unsupported unless exact tool configured | Unsupported unless exact tool configured | Unsupported unless exact tool configured | Deny marker in intercepted input | Deny exact configured tool name |
| Supervisor MCP lifecycle/receipt mutations | Supported task transitions, not generic state-file writes; caller-asserted receipts never satisfy hook evidence | Unsupported | Unsupported | Deny marker if intercepted | Deny exact configured tool if chosen; otherwise advisory MCP contract |
| Codex MCP integration | Advisory | Unsupported | Unsupported | Unsupported | Unsupported |
| Any route outside Claude hook dispatch | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported |

Routine allow fixtures exercise source writes, `cargo test`, normal topic-branch push and unrelated absolute deletion without a decision. `adapters/claude-code/tests/policy.rs` covers every newly implemented category, JSON-policy errors, inactive hook denial, policy self-protection, unsupported compound shell, nested interpreters and unknown connectors. `adapters/claude-code/tests/protection.rs` covers the state guard across direct tools, Bash, TC argv/shell/file routes, using the real persistent-shell `line` field.

TC command routes with omitted or relative cwd cannot borrow the hook project's cwd: the daemon owns that resolution. Only an explicit absolute cwd is usable for those checks. Persistent `shell_session_exec` never supplies cwd; an extra caller-supplied cwd does not establish its sticky session location. When configured protection depends on that missing context, recognized relative deletions and current-branch-dependent reset/rebase operations receive narrow denials. Routine commands remain silent; no approval prompt is introduced. TC 0.3.15 reports shell sessions unsupported on this Windows host, so persistent-shell decisions are schema-faithful fixtures, not live shell-session enforcement evidence. Follow-up PTY input remains a separate unsupported path.

Command policy uses each tool's authoritative field; extra `argv` or `command` fields cannot shadow `line` or `shell_line`. Local filesystem/current-branch checks do not establish remote-target or environment-overridden repository context (for example `GIT_DIR`/`GIT_WORK_TREE`); those context-dependent cases are unsupported. Exact configured tool/argv-prefix denials and synthetic-marker checks still inspect intercepted inputs. No remote operation was exercised.

Named limits: script files, variable or environment expansion, shell aliases/functions, executable wrappers, compound commands, shell redirects, git aliases/implicit remote refspecs/`+ref` force syntax, `git -C`, alternate field names, bulk file paths, resolved jobs receiving later PTY input, unrecognized connectors and paths obscured from the hook remain gaps. Existing path canonicalization is not an OS filesystem capability or a guarantee against alias/symlink races. Exact denied tools and synthetic markers cover intercepted input only; they cannot detect unknown secrets or authorize arbitrary effects. Real deployment, publishing and protected-branch operations were not performed.

## Completion and failure boundaries

| Case | Expected result and fixture |
|---|---|
| Unsupported non-header completion | Refuse while mandatory evidence is missing. `policy.rs::nonheader_completion_refuses_until_adapter_cap_without_marking_completed`; historical live bypass: first-slice `T3-RESULTS.md` T6. |
| Adapter refusal cap | After three refusals, report cap reached and leave task uncompleted; same fixture. Worker cap: `replay.rs::handback_refusals_release_after_three`. |
| Hook payload/process input failure | Adapter returns advisory system message, no deny/block: `policy.rs::hook_input_failure_is_advisory_not_a_policy_denial`. `timed_out_hook_process_produces_no_enforcement_decision` starts a synthetic stalled hook subprocess, reaches a bounded runner deadline, kills and reaps it, and verifies no enforcement decision was produced. This proves process failure semantics in the fixture, not live Claude timeout handling; missing/crashed hooks and harness timeout remain without dependable enforcement. |
| Unsupported bypass route | Compound shell, nested interpreter and unknown MCP connector return no denial: `policy.rs::unsupported_shell_and_unknown_connector_are_explicit_bypass_fixtures`. |
| Stale candidate | Refuse stale receipts: `replay.rs::replay_a19_changed_candidate_makes_receipts_stale`; runtime candidate/cwd and TC tests extend this evidence. |
| Asserted MCP success | Does not let the chair stop: `replay.rs::mcp_asserted_receipt_does_not_let_the_chair_stop`, `task_completed_through_mcp_does_not_let_the_chair_stop`, `task_completed_through_mcp_does_not_pass_a_handback`. |
| Tampered checkpoint or receipt | Refuse without silently resetting state: `replay.rs::tampered_receipt_refuses_and_leaves_gate_untouched`, `tampered_checkpoint_denies_writes`. |

These fixtures prove adapter decisions. They do not prove that all current harnesses load hooks, that a timeout blocks, or that an agent with the same OS authority cannot replace the hooks. The original audit rejected the earlier persistent-shell coverage claim; its findings and original evidence remain intact. Repair evidence, fresh live Claude traces and independent final-candidate re-audit are recorded separately in `REPAIR-RESULTS.md` and its linked external artifacts.
