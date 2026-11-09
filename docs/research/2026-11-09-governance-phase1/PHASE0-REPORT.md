# Phase 0 Discovery Report and Proposed Architecture (v2)

*Read-only. This step created, changed, started and stopped nothing. Changes made on the VPS during the Phase 0 window were owner-directed and are labelled as such. Evidence is cited as `inventory:line` (C:\temp\vps-inventory\inventory.sanitized.json, collected 12:18:19Z), `spec:line` (C:\Users\rakovnik\Downloads\CLAUDE_CODE_HARNESS_BOOTSTRAP.md), repo `file:line`, a docs URL, a named Hostinger operation description, **[verified]** for commands run by this report's author, or *orchestrator record* for facts from the orchestrator session that are not in lane evidence. Lane reports are cited by lane name because their files were not available to this revision.*

## 1. Summary and recommended path

**Your request first: agentmemory** (owner-directed changes, outside the Phase 0 read-only scope, spec:468)
- 11:17:58Z: a `docker_compose_stop` succeeded (inventory:98). The record names no project; linking it to agentmemory comes from the orchestrator record.
- Its containers ran again from about 11:30Z ("Up 48 minutes" at collection, inventory:124, 133). There is no matching action on the first page (15 entries) of the actions list. The cause is unknown; `restart: unless-stopped` does not restart a container that was stopped on purpose (everos lane).
- 12:22:12Z: stopped again on your instruction ("agentmemory should be stopped as I no longer use it"). 12:24:58Z: deleted through Hostinger Docker Manager (`docker_compose_down`) after you chose "Delete the project". Both entries are in the VPS action log (orchestrator record, read after the inventory).
- Containers, network and data volume are gone; https://memory.robagentic.tech/ returns 404 (no route). Copies remain only in the 2026-11-02 weekly backup and possibly in the 12:18:28Z snapshot record (unconfirmed). Rollback means restoring from that backup, which is whole-VM.
- The credentials the project held are unchanged by the deletion (§2, decision 1).

**Governance platform**
- coding-supervisor 0.13.1 is a working local gate for Claude Code with known gaps. It requires a contract, observes the real commands that ran and keeps a hash-chained receipt log. But it fails open on adapter error, its matchers have gaps, and the chain is unsigned. Against an agent with shell access it is advisory (spec:419).
- It has no network component, no signing, no worktree creation, no reviewer record and no time or turn limits.

**Recommended path:** extend the existing coding-supervisor workspace rather than starting a new project.
1. Phase 1, local: contract versioning, worktree creation, reviewer records, limits, and fail-closed as the **default** for governed tasks (spec:242).
2. Phase 2: a new `server` crate in the same workspace with the HTTP API and an MCP interface over Streamable HTTP, backed by one SQLite file.
3. Phase 3: deploy it as one new, isolated VPS project from a pinned image built by CI, only once the hard preconditions in §7 are met.

Receipts would be signed by the machine and then by the server. CI re-running the acceptance commands, with branch protection, is the only check that can be trusted remotely. None of this is tamper-resistant while governed sessions hold Hostinger admin access (§6, decision 15).

## 2. Environment findings

**Recorded before or outside discovery** (orchestrator record unless cited)
- agentmemory events: see §1.
- The inventory shows a snapshot record with `created_at` equal to `expires_at` (12:18:28Z, inventory:80-83), 9 s after collection, and the actions list has no `snapshot_create`. Whether a usable snapshot exists is UNVERIFIED.
- **Credential exposure:** before the discovery workflow, a Hostinger docker-project read (`vps_docker_get` on agentmemory) in the orchestrator session returned the stored values of `GITHUB_TOKEN`, `OPENROUTER_API_KEY`, `OPENAI_API_KEY` and `AGENTMEMORY_SECRET` (names at inventory:143, 145, 153, 155). Those values now sit in Claude Code session transcript files under `C:\Users\rakovnik\.claude\projects\E--project\`. Whether they also reached workflow journals or lane outputs is UNVERIFIED. The project that held them is deleted, but the credentials are unchanged. Under spec:473 this is an incident. What to do about it is your decision.
- Access path: the hostinger lane reports that outbound SSH from your work network is blocked (no evidence cited, UNVERIFIED). From this PC the VPS is reached over HTTPS through the Hostinger plugin (auth method UNVERIFIED) and the REST API using the `HOSTINGER_API_TOKEN` user env var [verified: set; value not read].
- Node v26.13.0, resolving to `C:\nvm4w\nodejs` with `NVM_HOME` set (nvm-windows); npm prefix `C:\Users\rakovnik\.npm-global` [verified, `node --version`, `npm config get prefix`].
- **Docker Manager API:** at about 12:45Z the Hostinger Docker project-list endpoint returned HTTP 500 ("[VPS:9999] Request failed") through both the plugin and the REST API. The VM was running, and crawl4ai and everos `/health` returned 200. The current project list is UNVERIFIED.

**This PC**
- Windows 11 Pro 10.0.26300 (session environment). Claude Code **2.1.319** [verified, `claude --version`].
- The supervisor binary on the npm path is 0.13.1, used by two wired locations (installed-integration lane F5, F6).

**Claude Code capabilities** (docs lane; hook claims come from https://code.claude.com/docs/en/hooks, of which the lane read only the first 100 KB of 289 KB)
- 33 hook events are listed in the part that was read.
- PreToolUse can return allow, deny, ask or defer. Exit code 2 blocks on PreToolUse, Stop and SubagentStop, not on PostToolUse.
- Remote MCP supports Streamable HTTP with OAuth, static headers or `headersHelper`. Reserved credential variable names read as **empty** in a remote server's `url` and `headers`.
- @-file mentions bypass PreToolUse. Hooks, permission modes and MCP servers in a plugin subagent's frontmatter are ignored. `isolation: worktree` branches from the default branch, not the current HEAD.
- **UNVERIFIED:** whether PreToolUse matches MultiEdit or the Agent tool, settings precedence, the Windows sandbox, and what `--worktree` checks.

**VPS srv1640039:** KVM 4, 4 vCPU, 16,384 MB RAM, 204,800 MB disk (about 214.7 GB), Ubuntu 24.04 with Docker and Traefik (inventory:5-12).

| App | State at 12:18Z | Images | Route (Traefik label) | Notes |
|---|---|---|---|---|
| agentmemory | running (2); *deleted 12:24:58Z, owner-directed* | `agentmemory:local`, `iiidev/iii:0.11.2` | memory.robagentic.tech → 3111 (now 404) | Mounted a file under /home/svetipeter (inventory:162) |
| crawl4ai | running (1) | `crawl4ai:unmasked` | crawl4ai. → 11235 | |
| everos | running (1) | `python:3.12-slim` + everos 1.5.1 installed on boot | everos. (header-matched) + `/health` | No health check. Token value sits in a router label (everos lane) |
| feedback | running (2) | `gw2bo-feedback:latest`, `postgres:16-alpine` | feedback. → 8080 | Its own Postgres, not shareable |
| obsidian-web | running (1) | `lscr.io/linuxserver/obsidian:latest` | obsidian. → 3000 | Fixed container name. Compose file and volumes under /home/svetipeter (inventory:253, 269) |
| traefik | running (1) | `traefik:latest` | host network mode, Docker provider, `exposedbydefault=false`, resolver `letsencrypt` | Docker socket mounted read-only (inventory:288) |

**Resources and backups** (decimal GB)
- CPU: 2.5–4% used.
- RAM: about 5.3 GB last and 7.2 GB peak of about 17.2 GB (16,384 MB), so about 10 GB free at peak. Measured before agentmemory was removed. Whether it includes page cache is UNVERIFIED.
- Disk: about 46.5 GB used of about 214.7 GB, about 168 GB free (inventory:10, 24-45).
- Firewall: inbound 22, 80, 443 and ICMP only (inventory:90-93).
- Backups: whole-VM only, two kept (2026-11-02 and 2026-10-25). No evidence of a restore test was found (inventory:68-79).
- No shared Docker network (hostinger lane).

## 3. coding-supervisor today versus the spec

The repo is at `main` `e25a0d3`, dated 2026-11-07. The only untracked path is `.claude/`. Workspace members: `adapters/claude-code`, `adapters/mcp`, `cli` [verified].

| Spec item | Exists | Partial | Missing | Evidence |
|---|---|---|---|---|
| **P1 Contract** | id, goal, criteria with exact command, scope, repair budget | `forbidden_effects` enforced only for typed entries in strict mode; the core comment saying "Declared only. Not enforced by the gate" is stale | schema version, base SHA, approval identity, time/turn/spend limits, required artifacts, reviewer policy | lib.rs:13-47, src/lib.rs:40-43 (stale), adapters/claude-code/src/lifecycle.rs:230-232 |
| **P2 Isolation** | | binds a task to a worktree and checks it | creating, resetting or cleaning worktrees; branch per task | binding.rs:70; cli/tests/worktree.rs |
| **P3 Verification** | observes real commands through hooks | an open finding blocks acceptance | reviewer identity, model, APPROVED/REJECTED record | lib.rs:178 |
| **P4 Gate/receipt** | gate at Stop, SubagentStop and PreToolUse | SHA-256 hash chain, **unsigned**; MCP receipts are caller-asserted | signing, pre-commit hook, CI receipt check, branch protection (API returned 404) | chain.rs:1-40; adapters/mcp/src/lib.rs:1 [verified]; grep found no signing or network code [verified] |
| Fail-closed (spec:242) | PreToolUse with `onFailure: block` | | the adapter fails open on error | adapter.rs:194-197 [verified] |
| **D1 Central service** | | | all of it (no HTTP server, no sockets) | coding-supervisor lane |
| **D2 Local supervisor** | CLI, local state, command capture | | registration, auth, policy fetch, offline rules | CLAUDE.md:50 |
| **D3 Adapters** | Claude Code, enforcing | Codex, Cursor, OMP, Grok: MCP only, advisory | enforcing adapters for those four | CLAUDE.md:53, 59 |
| **E MCP tools** | `task_start`, `receipt_record`, `finding_raise`, `task_state`, `task_complete`, … (stdio) | | remote transport, auth, `policy_get`, `review_submit`, `receipt_verify`, `rules_get` | mcp/lib.rs:218-756 |
| **F State machine** | 10 states with a tested transition matrix | `Blocked` is terminal | `EXPIRED`, `NEEDS_HUMAN`, `POLICY_UNAVAILABLE`, `REVIEW_PENDING`/`REJECTED`, `RECEIPTED`, `COMMIT_AUTHORIZED` | lib.rs:115-126 |
| **G Receipts** | command, cwd, exit code, base commit, dirty digest, timestamps | output reference | signer, key ID, reviewer, changed-file manifest, final tree, machine ID | lib.rs:81-91 |
| §5 Limits | repair rounds, cap of 8 continuations | worker tokens are journaled | wall-clock, turn and spend limits ("hook payloads carry no USD cost") | measure.rs:363; journal.rs:109-115 |

## 4. Reuse map

**Extend (coding-supervisor):** the contract format, gate, receipt chain, Claude Code adapter, CLI and MCP tool names. A new `server` crate joins the workspace. The core crate stays std-only; network, database and signing code live outside it.

**Integrate narrowly**
- **Jev:** an optional, advisory label (`persona`/`model_role`) for picking a reviewer. Use it only when `action=recommend` and `usable=true`, and only after the defect at jev_router.py:656-666 is fixed (jev lane). Never feed its `control` or `complexity` answers into the gate, and never treat Jev telemetry as usage evidence. Jev has no provider model IDs, so the service needs its own small label-to-model table.
- **EverOS:** a read-only `rules_get` source using `memory_search` with a project filter and keyword mode, every result labelled "advisory memory, not evidence". If EverOS is down the answer is "memory unavailable" and governance carries on. The service never writes to EverOS. It needs a new project tag in `C:\Users\rakovnik\.everos-mcp\server.py:32` (your decision). Its token already sits in a Traefik label that anyone with Hostinger API access can read (everos lane).
- **fresh-verifier / code-reviewer / codex-subagents:** candidate reviewers. Their verdicts count only through `review_submit` with a reviewer identity kept separate from the builder's (§5). Cross-provider review is available through the codex-subagents MCP.

**Overlaps: keep separate or turn off in supervised repos** (installed-integration lane)
- ECC GateGuard denies the first edit. ECC `format-typecheck` runs on Stop for up to 300 s and is not observed as evidence. Both are candidates to turn off where the supervisor runs.
- ECC continuous-learning-v2 and autoharness are two learning loops; pick one or leave both outside governance.
- unlazy `GATES.md` and the ECC orch gates are prompts only. assay stays a tool for writing proof.

**Do not build**
- The `coding-harness/` layout in spec §7 (replaced by the addendum); a model router; memory as an evidence ledger; a separate database or contract format per agent; shell, file, Docker or SQL access through MCP.
- Any change to the existing VPS apps or to the existing Traefik configuration. The new router labels in §7 are still a Traefik change and need their own approval (spec:469, 734).

## 5. Proposed architecture (proposals)

**Central service (VPS)**
- **Components:** one Rust binary from the `server` crate: the HTTP API `/v1/…`, the MCP endpoint over Streamable HTTP, a deterministic gate and verification step, an append-only audit table and a scheduled dump job. The Rust MCP SDK's Streamable HTTP support is **UNVERIFIED**.
- **Database:** SQLite in WAL mode in a named volume: one service and one writer, no database container or port to secure, and a single-file dump copied off the VM gives the per-app restore that does not exist today. Alternative: a dedicated Postgres container if multiple writers or larger scale appear; never reuse feedback's database. Large evidence logs are content-addressed files in a second volume; the database keeps their hashes and size limits.
- **Network:** its own project network plus a `traefik.docker.network` label, no published ports. Whether a host-mode Traefik can reach an `internal: true` network is UNVERIFIED (hostinger lane), so use a normal project network.
- **Route (a Traefik change):** router `governance` with rule `` Host(`governance.robagentic.tech`) `` only, and router `governance-health` with that Host plus `` Path(`/healthz`) `` and a higher explicit priority. No PathPrefix catch-all. Certificate resolver `letsencrypt`, the one every existing router uses (HTTP challenge on `web`, stored in `acme.json` in the shared `traefik-letsencrypt` volume, inventory:288). None of the router rules read uses this Host (inventory:162, 183, 210, 247, 269). everos's header-matched rule was not read, because its line holds the token. Rollback is in §7.
- **Machine auth:** `machine_register` is completed by a human and issues a per-machine token kept in the OS credential store. Claude Code passes it through `headersHelper` calling `supervisor`, or a non-reserved env var name such as `GOVERNANCE_MACHINE_TOKEN`, because reserved names read as empty (docs lane).
- **Human approval:** a separate channel the machine token cannot use, plus a step that needs a person physically present: a WebAuthn/passkey check with user verification, or approval from a second device. The audit event records the authenticator type. Reason: this machine has browser automation (Charlotte, chrome-devtools MCP) running as your OS user next to your logged-in profile, so a plain OAuth page could be clicked by an agent (spec:613, 627). Remaining risk: if you approve from the governed machine, the passkey step is the only barrier.
- **Reviewer identity:** the builder holds the same machine token, so `review_submit` with that token alone proves nothing (spec:159, 167, 186). Proposed: the supervisor starts the reviewer run itself with a per-review credential and records the reviewer's model and transcript hash; alternatively the review runs in CI. Until one exists, reviewer records are labelled "caller-asserted" and do not count as independent approval.

**Local supervisor**
- creates worktrees with `git worktree add` from the contract's base SHA, then checks the tree is clean;
- records time and turn limits;
- caches the policy with an expiry and marks evidence created offline as pending sync;
- refuses new governed work if the policy cannot be verified;
- **fails closed by default** for governed tasks when a required validator is unavailable, crashes or returns malformed output (spec:242). Fail-open is allowed only as an explicit, logged per-project exception labelled "advisory".

**Adapters:** Claude Code hooks call the local `supervisor`. Codex and the others stay advisory over MCP until they have hooks.

**State machine:** keep the existing states, add the missing ones from §3 and actor permissions per transition. Any diff change after review returns the task to `REVIEW_PENDING`.

**Receipt signing options** (Ed25519 candidate; library chosen in Phase 1)

| Option | Proves | Weakness |
|---|---|---|
| A. Machine key held by the local supervisor | "this machine's supervisor signed" | An agent with a shell as the same OS user can read the key (CLAUDE.md:50): an attestation, not proof |
| B. Server key on the VPS only | the central record matches the evidence submitted | Cannot show that local commands ran |
| **C. Layered (recommended):** machine signs the evidence bundle, server countersigns after its gate, CI verifies both and re-runs the acceptance commands | The CI re-run is independent evidence | More parts to build and test |

**Key custody**
- Server key: generated in the container on first boot and written to a volume; only the public key and key ID are published. Not in compose environment variables, because a `vps_docker_get` read returned stored env values into a session (§2). Backup: an encrypted export to a location you hold. The key is also inside Hostinger's whole-VM backups, and anyone who can replace the project can mount its volume (§6). Rotation by a key-ID list; revocation by marking a key ID revoked.
- Machine keys: one per machine in the OS credential store, registered by key ID at `machine_register`, revoked by key ID on the server. Same-user agents can read them (option A).

## 6. Security boundaries

| Layer | Can enforce | Cannot enforce / known bypasses |
|---|---|---|
| Claude Code hooks | Denying or blocking matched tools before they run; blocking at Stop | Tools the matchers miss (robert's matcher leaves out MultiEdit); @-mentions; indirect writes from Bash; fails open on adapter error today; plugin subagent frontmatter ignored |
| Local supervisor | Contract scope, recording real commands, the hash chain | An agent with a shell as the same OS user can change state and keys |
| Git pre-commit (proposed) | Fast local feedback | `--no-verify`, or deleting the hook |
| CI plus branch protection | An independent re-run and a required check | Admin override. Today `main` is **unprotected** (404) |
| Remote MCP | Authentication, schema checks, state transitions, audit | Enforces nothing on the machine; receipts submitted through it are caller-asserted (spec:450, 635) |
| Human approval (proposed) | User presence through passkey or second device | Approving from the governed machine leaves the passkey step as the only barrier |
| VPS | Isolation from the other apps, TLS | Anyone with Hostinger API access can read compose env values and replace or delete projects |
| **Hostinger access held by governed sessions (today)** | Nothing for governance | The Hostinger plugin and `HOSTINGER_API_TOKEN` are available to agents on this PC. `vps_docker_create` replaces a project with the same name and `vps_docker_delete` removes volumes (Hostinger operation descriptions). An agent could swap in a compose file that mounts the governance database and key volumes, or delete the project. Until that access is removed from governed sessions, the central record and server signature are **not tamper-resistant against the governed machine** |

## 7. VPS deployment plan (not to be executed)

**Hard preconditions for Phase 3** (to be met, not left open)
1. Governed sessions no longer hold Hostinger admin access (decision 15).
2. You confirm the existing snapshot may be overwritten, or no snapshot is taken. `vps_snapshots_create` states that a new snapshot overwrites the existing one (spec:479).
3. Evidence that a backup can actually be restored (spec:479, 800).
4. The off-VM dump destination is chosen and a restore from it is tested (decision 7, spec:804).
5. The cause of agentmemory's unexplained restart is found, or shown not to affect new projects.
6. The Docker Manager API answers project-list calls (it returned HTTP 500 at about 12:45Z).

**Shape**
- **Project:** `coding-governance`, one container `coding-governance-server-1`, volumes `governance-db` and `governance-evidence`, no published ports. Restart policy `unless-stopped`, health check on `/healthz`, limits `mem_limit: 512m`, `cpus: 1.0`, `pids_limit: 256`. The name avoids every existing project, router, service and host (hostinger lane).
- **Server limits and logs:** request-size limits, timeouts, per-token rate limits and audit logging in the server (spec:631); structured JSON logs to stdout.
- **Resources (estimate):** under 0.5 GB RAM and under 5 GB disk in the first year, against about 10 GB RAM and about 168 GB disk free.
- **Image:** built by CI from the existing `release.yml`, published to GHCR as a **public** image with a pinned digest and no secrets inside. A private image would need a pull credential stored on the VPS, readable by anyone with Hostinger API access. Hostinger MCP cannot build from source.
- **Migrations:** forward-only SQLite migrations at start-up, recorded in a version table. Every upgrade starts with a dump. Rollback means the previous image digest plus the pre-upgrade dump.
- **Monitoring and alerting:** an external check of `/healthz` and certificate expiry that alerts you (tool to be chosen). No monitoring app is in the inventory.

**Steps, all through Hostinger MCP**
1. `vps_docker_list` to confirm the name is free. If the call fails, stop: `vps_docker_create` silently replaces a project with the same name.
2. Add the DNS A record for 72.62.155.186. Whether the zone is in this Hostinger account is UNVERIFIED.
3. Take a snapshot only if precondition 2 allows it.
4. `vps_docker_create` with the compose content and the §5 router labels.
5. Check `/healthz`, TLS and authentication.
6. `vps_docker_list` to confirm the other apps are unchanged. That list is now four apps plus traefik, because agentmemory is deleted.

**Upgrade, rollback, uninstall, backup**
- **Upgrade:** take a dump, then run `vps_docker_create` under the same name with the new digest. Whether volumes survive the replacement is UNVERIFIED, so test it on a throwaway project first.
- **Rollback:** `vps_docker_stop coding-governance`, then confirm both routers are gone (the host returns 404, as memory. does now). The issued certificate stays in the shared `acme.json` until it expires and is not served. A whole-VM restore is the last resort, since it rolls back every app.
- **Uninstall:** export the dump, stop the project, then `vps_docker_delete` (it removes volumes), then remove the DNS record.
- **Backup:** the scheduled job writes a dump inside the container and copies it off the VM (precondition 4).

## 8. Phased plan

| Phase | Work | Stop point and acceptance check |
|---|---|---|
| 1 Local | Contract v2 schema; worktree create and verify; reviewer record; time/turn limits; fail-closed by default; Ed25519 signing outside the core; mock server; Git pre-commit hook; CI step running `supervisor verdict` | All 20 cases in spec §9 run against disposable fixtures with exit codes recorded; a validator crash blocks the task |
| 2 Server | `server` crate: API, MCP over HTTP, SQLite migrations, auth, audit, countersigning, reviewer credentials, passkey approval | Integration tests with mock machines: rejected transitions, idempotent retries, stale policy rejected; a builder submitting a review under its own identity is refused; an agent-driven browser trying to approve is refused |
| 3 Deploy | As in §7, after your approval | §7 preconditions met; health, TLS, auth and persistence pass after a restart; restore from a dump tested; other apps unchanged |
| 4 Rollout | Second machine; Codex adapter later | One task from two machines checks out against one central record; offline, revoked-credential and concurrent cases pass |

**Proposed limits** (spec §5 examples): 20 orchestration iterations; 600 s wall-clock, likely too short for real tasks, so set per project. **No spend ceiling is claimed:** Jev does not save usage and hook payloads carry no USD cost. Journaled tokens may later support a clearly labelled estimate.

## 9. Risks and trade-offs

- **Something on the VPS can start projects outside the API.** agentmemory came back about 11:30Z with no matching action on the first page of the actions list. The project is deleted, but whatever started it could act on coding-governance. Find the cause before Phase 3.
- **Governed sessions hold Hostinger admin access** (§6). Today this defeats the central record and server countersigning.
- **Self-approval paths:** browser automation next to your logged-in profile, and a builder that can submit its own review (§5).
- **Credentials already exposed:** four values sit in transcript files on this PC (§2), unchanged by the deletion. The EverOS token sits in a Traefik label.
- **Docker Manager API reliability:** HTTP 500 on project listing through both routes at about 12:45Z. Deployment steps 1, 4 and 6 depend on it.
- **agentmemory leftovers on this PC** (installed-integration and everos lanes): plugin 0.9.33 installed, two scheduled tasks `Ready`, `AGENTMEMORY_*` user env vars, allow rules in four `settings.local.json` files. They now point at a deleted service.
- **Too many hooks:** roughly ten Stop hooks run, and several PreToolUse hooks make deny decisions in parallel, which makes outcomes hard to attribute.
- **Jev sends prompt text to an outside service** on every Claude prompt, while its docs say it runs in shadow mode (jev lane, finding 8).
- **Unpinned binary:** the npm global path swaps the supervisor binary for every wired project without review.
- **Single snapshot slot, whole-VM-only restore, no restore evidence.**
- **Out-of-band state on the VPS:** locally built images, plus obsidian-web's compose file and volumes under `/home/svetipeter` (inventory:253, 269). These show past shell use that MCP cannot see.
- **Trade-off:** SQLite is simple but limits concurrent writes. Layered signing gives more trust for more work.

## 10. Decisions needed from you (recommended option first)

1. **agentmemory follow-ups** (project deleted on your instruction): (a) clean up this PC: uninstall the plugin, disable the two scheduled tasks, remove the `AGENTMEMORY_*` env vars and the stale allow rules; (b) read page 2 onward of the actions list to find what restarted it; (c) the exposed credentials are unchanged, and what to do about them is your call. All of these are later, approved actions.
2. **Foundation:** a new `server` crate in the coding-supervisor workspace (recommended), rather than a separate repo?
3. **Database:** SQLite (recommended) or a dedicated Postgres?
4. **Signing:** layered with a CI re-run (recommended), or a server key only?
5. **Image registry:** a public GHCR image from the existing release workflow (recommended), or install on boot as everos does?
6. **Hostname:** `governance.robagentic.tech`, after confirming the DNS zone is in this Hostinger account?
7. **Off-VM dump destination and frequency:** daily (recommended), to a destination you choose. This is a Phase 3 precondition.
8. **coding-supervisor `main`:** turn on branch protection with a required CI check (recommended)?
9. **Fail-closed rollout date:** fail-closed becomes the default for governed tasks either way (spec:242). Ship it with Phase 1 (recommended), or after the "month of clean journals"?
10. **ECC GateGuard and `format-typecheck`:** turn them off in supervised repos (recommended)?
11. **Human approval:** a browser page behind GitHub OAuth plus a passkey user-presence check (recommended), or approval from a second device?
12. **Jev mode:** return Claude, Codex, Cursor and OMP to shadow mode until the held-out evaluation is done (recommended)?
13. **EverOS tag:** add a `SUPERVISOR` tag to the bridge allowlist later (recommended)?
14. **robert:** retire it as a dogfood target, or fix its matchers?
15. **Hostinger access separation:** remove the Hostinger plugin and `HOSTINGER_API_TOKEN` from supervised sessions (recommended), or move VPS operations to a separate operator-only machine or profile? This is a Phase 3 precondition.
16. **Reviewer identity:** a supervisor-started reviewer run with a per-review credential (recommended), a review in CI, or accept reviewer records labelled "caller-asserted"?

## 11. Capability gaps and UNVERIFIED items

**Gaps:** Hostinger MCP has no shell, file access, image build, network or volume inspection, or per-app backup; logs are capped at 300 lines. No spend data exists from any source.

**UNVERIFIED**
- The current Docker project list (HTTP 500 at about 12:45Z).
- What restarted agentmemory, and which project the 2026-10-25 19:10 stop / 19:21 up pair applied to.
- Whether the 12:18:28Z snapshot is usable, and whether it holds agentmemory data.
- Whether the exposed values also reached workflow journals or lane outputs.
- The Hostinger plugin's auth method, and evidence for the SSH block.
- Whether the robagentic.tech DNS zone is in this Hostinger account.
- Whether a host-mode Traefik can reach an `internal: true` network.
- Whether volumes survive when `vps_docker_create` replaces a project.
- Whether the RAM figure includes page cache. Whether the backups can be restored.
- Whether the 33 hook events are the complete list. Whether PreToolUse matches MultiEdit or the Agent tool. Whether Stop fires for subagents. Settings precedence and the Windows sandbox.
- The Rust MCP SDK's HTTP transport.
- The org-level ruleset mentioned at HANDOVER.md:226.
- Whether the agentmemory plugin is enabled or only installed. Whether you approved Jev's live mode.

The report's author personally checked: the coding-supervisor HEAD, the Claude Code version, the fail-open function, the mode line in the MCP adapter, the workspace members, that grep finds no signing or network code, the Node/npm/nvm facts, that `HOSTINGER_API_TOKEN` is set (value not read), and the Traefik resolver and router rules in the inventory. Owner-directed events after 12:18:19Z come from the orchestrator record. Every other claim comes from the lane reports.

## Review disposition

| # | Issue (short) | Severity | How addressed |
|---|---|---|---|
| R1-1 | Credential exposure claim uncited | major | Applied: source named (`vps_docker_get` on agentmemory, orchestrator session, before discovery), names only, transcript directory given (§2); §5 key-custody argument now points to it |
| R1-2 | SSH, OAuth, token env var and Node facts uncited | minor | Applied: Node, npm prefix and nvm-windows path [verified]; "single prefix" dropped; token env var name [verified set]; SSH block and plugin auth marked UNVERIFIED |
| R1-3 | 11:17:58Z action names no project; "no API action" covers only page 1 | minor | Applied (§1) |
| R1-4 | Snapshot stated as existing | minor | Applied: "snapshot record … usable snapshot UNVERIFIED" (§2, §11) |
| R1-5 | "Never a restore test" overstated | minor | Applied: "No evidence of a restore test was found" |
| R1-6 | 33 hook events from a partial page read | minor | Applied: qualified; completeness listed as UNVERIFIED |
| R1-7 | obsidian image ref; its /home/svetipeter paths missing | minor | Applied: `lscr.io/...` and inventory:253, 269 in the table and §9 |
| R1-8 | `forbidden_effects` sources conflict | minor | Applied: full adapter path cited, core comment src/lib.rs:40-43 marked stale (§3) |
| R1-9 | Mixed binary and decimal units | minor | Applied: MB totals with decimal GB equivalents, decimal GB throughout |
| R2-1 | Governed machine holds Hostinger admin access | major | Applied: new §6 row, §9 risk, decision 15, Phase 3 precondition 1, "not tamper-resistant" label |
| R2-2 | Human approval can be driven by an agent | major | Applied: passkey or second-device presence, authenticator type in audit, Phase 2 negative test, remaining risk stated (§5, §6, §8, decision 11) |
| R2-3 | Builder can self-approve via `review_submit` | major | Applied: separation options, "caller-asserted" label until built, builder-self-review negative test, decision 16 |
| R2-4 | Fail-closed made optional, against spec:242 | major | Applied: default for governed tasks, logged "advisory" exception only; decision 9 now asks only for the date |
| R2-5 | Traefik change contradicts "Do not build" | major | Applied: exact Host-only rules, explicit priority, resolver `letsencrypt`, check against the router rules read, route rollback and certificate fate; "Do not build" reworded. everos's header-matched rule was not read (its line holds the token), so the overlap check against it is left open |
| R2-6 | Snapshot overwrite, restore evidence, off-VM dump left open | major | Applied: hard Phase 3 preconditions 2–4 (§7), not open items |
| R2-7 | Missing deployment items (spec H/E) and pull credentials | major | Applied: limits and logs, migrations and rollback, monitoring, upgrade and uninstall, public image with pinned digest (§7). Monitoring tool left to choose |
| R2-8 | Key backup and machine-key custody undefined | minor | Applied: encrypted owner-held export, key present in VM backups, machine-key storage and revocation (§5) |
| R2-9 | "Solid" overstates the gate | minor | Applied: "working local gate with known gaps … advisory against shell access" |
| R2-10 | Lane references instead of file:line or URL | minor | Partially applied: hooks docs URL added, Hostinger operation descriptions cited, author-verified items marked. Lane report files were not available to this revision, so lane names remain. The 12:22Z qualifier was superseded: the stop and deletion are in the VPS action log (orchestrator record) |
| R2-11 | "Not repeated" understates the exposure | minor | Applied: persistence in transcript files stated, journals and lane outputs UNVERIFIED, spec:473 incident, EverOS token exposure noted (§2, §4). No rotation advice given; the owner decides |
| R2-12 | Stops not labelled owner-directed; restart is a boundary risk | minor | Applied with an adapted fix: all three VPS actions labelled owner-directed. The proposed rollback `vps_docker_start` no longer applies after deletion, so rollback is a restore from the 2026-11-02 backup. The unknown-starter risk is in §9 and Phase 3 precondition 5 |
