# Harness capability survey: hooks, workers, MCP, files, logs

Date: 2026-10-29. Produced by a read-only research worker from official documentation fetched that day, each harness's latest release, and read-only checks of the versions installed on the author's machine. **This is a reading of documentation, not a measurement.** Every "mode the docs would permit" below must be confirmed by a fixture investigation (the T3 method) before an adapter declares it. Cells marked unknown stay unknown until a hook that logs its own stdin has been run.

Purpose: starting facts for Phase B (one adapter investigation per harness). The Claude Code column is omitted because it is already measured (see `../2026-10-28-completion-gate-first-slice/T3-RESULTS.md`).

## Summary table

| Row | Codex CLI | Cursor (agent + CLI) | OMP (oh-my-pi) | Grok Build |
|---|---|---|---|---|
| deny tool call | yes [1] | yes [9] | yes [17] | yes [25] |
| see worker result | yes [2] | yes, but unreliable [10] | partial [18] | partial [26] |
| refuse completion | yes [3] | partial [11] | yes [19] | no [25] |
| alter final message | no [4] | no [12] | yes [20] | no [25] |
| background workers | partial [5] | yes [13] | yes [21] | yes [27] |
| MCP client | yes [6] | yes [14] | yes [22] | yes [28] |
| instruction file | yes, AGENTS.md [7] | yes, AGENTS.md and .cursor/rules/*.mdc [15] | yes, AGENTS.md and RULES.md [23] | yes, AGENTS.md and CLAUDE.md [29] |
| skills dir | yes, .agents/skills [7] | yes [15] | yes [23] | yes [29] |
| logs on disk | yes [8] | partial [16] | yes [24] | partial [30] |
| version | 0.165.0, released 2026-10-27 [8] | app 3.28.5; CLI docs v2026.10.22 [16] | 18.13.3, released 2026-10-28 [24] | 0.1.210; docs show no version [30] |

## Footnotes

### Codex CLI

Hooks page: https://developers.openai.com/codex/hooks.md (same text at learn.chatgpt.com/docs/hooks.md). The page gives no version number and says to "use this page as the release behavior reference."

1. Hooks are configured in `~/.codex/hooks.json`, `~/.codex/config.toml`, `<repo>/.codex/hooks.json` or `<repo>/.codex/config.toml`. Each hook gets JSON on stdin with `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `model`, `turn_id` and `permission_mode`.
   - Events: SessionStart, SessionEnd, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SubagentStart, SubagentStop, Stop, Interrupt.
   - A PreToolUse hook denies with `hookSpecificOutput.permissionDecision: "deny"` or with exit code 2 plus a reason on stderr. It can rewrite the call with `updatedInput`.
   - Limits: hosted tools such as WebSearch get no PreToolUse or PostToolUse. The page says "Treat tool hooks as a useful guardrail, not a complete enforcement boundary."
   - Unsupported outputs (`ask`, `continue:false`) are "marked as failed ... and continues the tool call", so they fail open.
   - Project hooks run only after the user trusts them in `/hooks`.
2. SubagentStop's input includes `agent_id`, `agent_type`, `agent_transcript_path`, `stop_hook_active` and `last_assistant_message` ("Latest subagent assistant message"). A hook can return `decision:"block"` to make the subagent continue.
3. Stop's input includes `last_assistant_message` and `stop_hook_active`. On `decision:"block"`, Codex continues and "creates a new continuation prompt ... using your `reason`". Same semantics as Claude Code's Stop. No limit on continuations is documented; the hook must check `stop_hook_active` itself.
4. Neither Stop nor PostToolUse documents a field that replaces the final assistant text. PostToolUse `continue:false` replaces a tool result, not the final message.
5. Subagents page: https://learn.chatgpt.com/docs/agent-configuration/subagents.md. Subagents run in parallel, but "Codex waits until all requested results are available, then returns a consolidated response." A "background-agent UI" exists in some clients. Custom agents are TOML files in `~/.codex/agents/` or `.codex/agents/`.
6. MCP page: https://learn.chatgpt.com/docs/extend/mcp.md. Codex supports "STDIO servers" and "Streamable HTTP servers". No documented limit on tool count found; unknown.
7. AGENTS.md is read from `~/.codex` (AGENTS.override.md first) and then from each directory from the repo root down to the working directory: https://learn.chatgpt.com/docs/agent-configuration/agents-md.md. Skills are folders with a `SKILL.md` (required `name` and `description`) in `$CWD/.agents/skills` up to the repo root, `$HOME/.agents/skills` and `/etc/codex/skills`: https://learn.chatgpt.com/docs/build-skills.md. **Gap:** the author's local harness notes say Codex skills live in `~/.codex/skills`, but the current docs only list `.agents/skills`; whether `~/.codex/skills` is still read is unknown.
8. "Codex saves local session transcripts under CODEX_HOME". The hooks page warns that the transcript format "isn't a stable interface". On the author's machine `codex --version` reports 0.165.0; a rollout file under `~/.codex/sessions/2026/10/24/` contains `function_call`, `function_call_output`, `custom_tool_call` and `token_count` records with `input_tokens`, `output_tokens` and `total_token_usage`. Latest release per the GitHub API: rust-v0.165.0, published 2026-10-27.

### Cursor

Hooks page: https://cursor.com/docs/hooks.md. CLI changelog: https://cursor.com/docs/cli/changelog.md (latest entry v2026.10.22). Installed desktop app (package.json) is 3.28.5.

9. Hooks live in `~/.cursor/hooks.json`, `<project>/.cursor/hooks.json`, the enterprise system path, team-distributed config and plugins. Format `{"version":1,"hooks":{...}}` with per-hook `matcher`, `timeout`, `failClosed` and `loop_limit`.
   - Events: sessionStart, sessionEnd, preToolUse, postToolUse, postToolUseFailure, subagentStart, subagentStop, beforeShellExecution, afterShellExecution, beforeMCPExecution, afterMCPExecution, beforeReadFile, afterFileEdit, beforeSubmitPrompt, preCompact, stop, afterAgentResponse, afterAgentThought, beforeTabFileRead, afterTabFileEdit, workspaceOpen.
   - preToolUse returns `permission: "deny"` and can set `updated_input`. Exit code 2 also blocks. Invalid JSON blocks the permission hooks.
   - Any other failure is "fail-open by default" unless `failClosed: true` is set.
   - On Windows, hook commands run through PowerShell.
   - The CLI runs hooks too: "afterAgentThought/afterAgentResponse events emit in the CLI".
10. subagentStop's input includes `status`, `task`, `summary` ("Output summary from the subagent"), `modified_files`, `tool_call_count` and `agent_transcript_path`. Its output is `followup_message`. Forum reports (forum.cursor.com) say subagentStart and subagentStop stopped firing around 2026-08-14 to 2026-08-18; Cursor staff could not reproduce on demand. Current status unknown.
11. stop's input is `status` and `loop_count`; it does not include the final text. Its output `followup_message` "will automatically submit it as the next user message". That re-prompts the agent rather than refusing the stop. Re-prompts are capped by `loop_limit`: default 5, `null` removes the cap.
12. afterAgentResponse receives `text` ("assistant final text") but documents no output fields; observe only.
13. Subagents page: https://cursor.com/docs/subagents.md. "`is_background` ... runs in the background without blocking the parent." Agent definitions are Markdown with YAML frontmatter in `.cursor/agents/` or `~/.cursor/agents/`.
14. MCP page: https://cursor.com/docs/mcp.md. Servers in `mcp.json` over stdio, SSE or Streamable HTTP. No tool-count limit stated; unknown.
15. Rules page: https://cursor.com/docs/rules.md. Project rules are `.cursor/rules/*.mdc`; "A plain `.md` file in `.cursor/rules` is ignored". AGENTS.md is supported. Skills page: https://cursor.com/docs/skills.md. Skill folders in `.agents/skills/`, `.cursor/skills/`, `~/.agents/skills/` and `~/.cursor/skills/`.
16. Every hook gets `transcript_path`. On the author's machine transcripts are `~/.cursor/projects/<ws>/agent-transcripts/<id>/<id>.jsonl` with `tool_use` records but no token-usage fields. The CLI changelog mentions "Per-turn token usage" in hook payloads, but the hooks page names only `context_tokens` on preCompact.

### OMP (oh-my-pi)

Docs in the repo: https://github.com/can1357/oh-my-pi/tree/main/docs. Installed binary reports omp/18.13.3; npm and GitHub show v18.13.3, published 2026-10-28.

17. OMP hooks are TS/JS modules, not shell commands. They live in `.omp/hooks/pre|post/*.ts` or `~/.omp/agent/hooks/pre|post/` and are loaded as extensions that call `pi.on(...)` (docs/hooks.md, docs/extensions.md). `tool_call` "may block, revise the tool's execution `input`". A handler error makes the wrapper "fail closed".
18. `before_subagent_spawn` can block a spawn or change its model. No dedicated event carries a subagent's result. Inference, not stated: the `task` tool's result passes through `tool_result`, and handlers are "rebound to every subagent session" (`ctx.agent.kind === "sub"`).
19. `session_stop`: "Explicit `{ decision: "block", reason }` refusals ... remain blocking until the hook allows completion or the operator interrupts." A real refusal, but it "never fires for task/subagent sessions."
20. `assistant_message`: "Return `{ content }` to replace text for history, persistence, `message_end` consumers". Text already streamed to the screen is not taken back.
21. docs/task-agent-discovery.md: "`blocking: true` makes the parent wait ... even when async task execution is enabled". Agents are Markdown files in `~/.omp/agent/agents/*.md` or `.omp/agents/*.md`.
22. docs/mcp-config.md: servers in `.omp/mcp.json` or `~/.omp/agent/mcp.json`. OMP also imports Claude, Cursor and VS Code MCP files. Limits unknown.
23. docs/context-files.md: `~/.omp/agent/AGENTS.md`, `.omp/AGENTS.md` and `RULES.md`. Task subagents "do not inherit it". docs/skills.md: `<skills-root>/<name>/SKILL.md` from the native `.omp` provider, `~/.omp/agent/managed-skills` and `.agents/skills`.
24. docs/session.md: sessions are `~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<sessionId>.jsonl`, with `model_usage` entries that carry `usage` / `totalTokens`.

### Grok Build

Docs: https://docs.x.ai/build/features/hooks.md (pages carry no version). Release notes (https://docs.x.ai/developers/release-notes.md) list "Grok Build is now available in beta" under May. Installed binary reports grok 0.1.210.

25. Hooks are JSON files in `~/.grok/hooks/*.json` or `<project>/.grok/hooks/*.json`. Grok also reads Claude Code's `.claude/settings.json` and Cursor's `.cursor/hooks.json`. Hook types are `command` or `http`; default timeout 5 seconds. Payload: `hookEventName`, `sessionId`, `cwd`, `workspaceRoot`, `toolName`, `toolInput`. Events: SessionStart, SessionEnd, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, PermissionDenied, Stop, StopFailure, Notification, SubagentStart, SubagentStop, PreCompact, PostCompact. "`PreToolUse` | A tool is about to run — the only blocking event". Denies with `{"decision":"deny"}` or exit code 2. "timeouts, crashes, malformed output — is fail-open". "For passive events, stdout is ignored."
26. SubagentStart and SubagentStop fire ("A subagent starts or finishes"), but no result field is documented. Subagents "return a summary to the parent" (https://docs.x.ai/build/features/subagents.md). A hook could read the session files from disk.
27. https://docs.x.ai/build/features/background-tasks.md: "run commands, subagents, and monitors in the background". Agent types live in `.grok/agents/` and `~/.grok/agents/`; file format not documented.
28. https://docs.x.ai/build/features/mcp-servers.md: `[mcp_servers.<name>]` in `~/.grok/config.toml` or the project `.grok/config.toml`, over stdio or HTTP. Grok also imports `~/.claude.json`, `.cursor/mcp.json` and `.mcp.json`. Limits unknown.
29. AGENTS.md family plus CLAUDE.md and `.grok/rules/*.md` (https://docs.x.ai/build/features/project-rules.md). Skills: `.grok/skills/`, `~/.grok/skills/` and `~/.agents/skills/`, each with a SKILL.md (https://docs.x.ai/build/features/skills-plugins-marketplaces.md).
30. Sessions are saved with "prompts, responses, tool calls, and file snapshots — under `~/.grok/sessions/`" (https://docs.x.ai/build/features/sessions.md). Token usage on disk is not documented. On the author's machine the one session folder has `chat_history.jsonl` and `events.jsonl` with no token fields (one small session; not conclusive).

## Mode the documentation would permit (reading, not measurement)

**Codex CLI.** Full enforcement looks possible on the same paths as Claude Code: PreToolUse can deny; SubagentStop sees `last_assistant_message` and can make the subagent continue; Stop can turn the end of a turn into a continuation; PostToolUse can annotate. Caveats: hosted tools bypass tool hooks; unsupported outputs fail open; project hooks must be trusted first; the adapter must count continuations itself using `stop_hook_active`. Read it as "enforcing on local tool, subagent and stop paths", advisory for hosted tools.

**Cursor.** Enforcement looks possible before tool calls (preToolUse, beforeShellExecution, beforeMCPExecution) with `failClosed: true`. End of turn and subagent end are partial: stop and subagentStop can re-prompt with `followup_message` but cannot refuse; stop does not receive the final text (pair with afterAgentResponse or `transcript_path`); re-prompts cap at 5 unless `loop_limit` is changed. Declare the completion gate as "re-prompting enforcement, bounded" rather than a hard refusal. Keep subagentStop advisory until the August firing bug is confirmed fixed.

**OMP.** The strongest documented hooks, but the adapter must be an in-process TS/JS extension, not a shell hook: `tool_call` blocks and fails closed; `session_stop` can genuinely refuse completion, main session only; `assistant_message` can rewrite the final text. Subagent results are reachable only indirectly through the task tool's `tool_result`, which needs a test. Read it as "enforcing for the main session; advisory for workers".

**Grok Build.** Only PreToolUse can enforce, and it fails open on timeouts, crashes and bad output with a 5-second default timeout. Stop and SubagentStop only observe. Grok loads Claude Code hook files, but ignores stdout on passive events, so a Claude Stop gate placed there would load and not enforce; the adapter must not assume it does. Read it as "enforcing only for denying tool calls; advisory for completion", or advisory through MCP alone.

## Not checked

Grok's SubagentStop payload; Grok's agent file format; any MCP tool-count limits; whether Codex still reads `~/.codex/skills`; whether OMP's `tool_result` fires for the subagent tool. Each needs a small hook that logs its stdin. The Cursor subagent bug status comes from forum reports only.
