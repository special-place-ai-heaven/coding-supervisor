# Gate modes: proposal (2026-11-02)

Status: agreed with the owner in conversation on 2026-11-02; not implemented.

## Why

The shipped levels `off|lite|full|ultra` (`adapters/claude-code/src/level.rs`) were named after ponytail's
intensity levels. The supervisor is a different thing: a gatekeeper for task completion. Its modes should
say how strict the gatekeeper is, not how hard a style rule pushes. Running the work loop stays the
agent's and the harness's job; a "standalone loop engine" stays out of scope (`CLAUDE.md`).

## What never varies with the mode

- Evidence binds to the current candidate; a changed candidate makes earlier checks stale.
- A self-reported check never satisfies enforcing evidence.

A mode that relaxed either would be the gate turned off, not a mode of it.

## What varies

1. Whether the gate refuses or only records.
2. How much proof a completion needs: the declared checks, or also scope and a failing-before-fix check.
3. What the harness can enforce (Claude Code can refuse; MCP alone can only advise).

## Modes

| Mode | On a completion claim | Use |
|---|---|---|
| `off` | Nothing is judged or recorded. | Project not supervised. |
| `audit` | Evaluated against evidence, verdict and measures recorded, never refused. | Measuring before trusting the gate; the only honest mode on a harness that cannot refuse. |
| `enforce` (default) | Refused unless every mandatory check passed on the current candidate, observed by the gate, for the chair and every worker. Repair cap reached: unresolved, never a pass. | Normal work. |
| `strict` | `enforce`, plus: edits outside the contract scope are refused, and a bug fix needs the reported failure observed failing on the task's starting candidate and passing on the current one (C1 then C2 of the inherited contracts). | Bug fixes, risky areas, work merged without close reading. |

## Mode capped by harness capability

A requested mode the harness cannot enforce runs as `audit` and says so: `status` and every journal line
report both the requested and the effective mode ("enforce requested, audit effective: harness cannot
refuse"). This extends principle 5 (adapters declare their mode) from the adapter to the mode.

## Compatibility

Old names stay accepted as aliases: `lite` = `audit`, `full` = `enforce`, `ultra` = `strict`. Level files
and journals written by 0.4.x keep working.

## Against main (`cb86835`)

| Item | State on main |
|---|---|
| `off`, `audit`, `enforce` behaviour | Exists as `off`, `lite`, `full`. Rename plus aliases only. |
| Scope refusal in `strict` | Exists in `ultra` (PreToolUse write denial via `scope_contains`). |
| Failing-before-fix check in `strict` | **New.** `cli/src/init.rs` declares C1 as a non-mandatory placeholder (`cargo test`); nothing checks that it failed on the starting candidate. Run records already carry the candidate, so the evidence exists to build on. |
| Requested vs effective mode | **New.** Journals carry `effective_level` and `level_source`; no capability cap. |
| Setting the mode from inside Claude Code | **New.** The plugin ships no command; today it takes `supervisor level` in a terminal. Proposal: `/supervisor <mode>` and `/supervisor status`, user-invoked only. |

## Open

- How a task is marked as a bug fix, so `strict` knows to require the failing-before-fix check
  (contract field, or the agent's proposal declaring it).
- Origin of the plugin carrying no MCP server (the "F1" note in `1ea1aad`) before one-step setup is proposed.
- No live Claude Code run of any mode yet.
- Ideas from the unmerged PR #2 (`feat/autonomous-gate`, head `120a0ed`) not covered here or on main:
  a note in a refusal when a check already failed before the change, and the plugin finding the binary
  through `npm root -g`.
