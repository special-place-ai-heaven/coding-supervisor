# Open items handed over from the home-PC session (2026-11-11)

Recorded as received from the owner on 2026-11-11. The home-PC session ended with no unpushed
code; these notes existed only in that session. Status was assessed there on 0.17.3 source plus
#91 and #94, so each item must be re-checked against `main` before work starts.

Sources, none of them in the repository: the field brief
`coding-supervisor-change-brief-2026-11-09.md` (owner approved its four changes; all four are
merged: #73, #78, #79, #83), a follow-up brief (five items, "nothing here is approved yet"), and
a 14-item report from the SymForge project's Claude session (2026-11-10).

Rule from the first brief, "what must not change": no mutation, mutating command or delegation
before an accepted intent; a live session's unfinished task stays protected from other sessions;
source changes need the full check floor; hooks stay fail-closed for mutation.

## Not approved by the owner yet (follow-up brief)

1. One task per git worktree, so sub-agents in linked worktrees are not refused. Today a task can
   be bound to a worktree (`worktree` on start/revise), but there is still one active task per
   project. Overlaps WP3 of `docs/research/2026-11-09-governance-phase1/PLAN.md`.
2. Read-only shell allow-list before an intent and in discussion turns. Named in the brief:
   `git ls-remote`, `which`, `cat`, `gh repo view`. Also on the other-session path, where the
   refusal should tell the session to start in a worktree. Seen refused under 0.19.1 in a
   discussion turn: `git stash list`, `gh api ... --jq ...`, and any `cd X && git ...` compound of
   read-only commands.
3. (Done with approved change 1, owner's condition of 2026-11-10.)
4. Acceptance line: the brief's item on how a completion is stated. Only the name survives; the
   follow-up brief itself is needed.
5. Pending-job answer: after a long Terminal Commander job the agent must call `command_status`
   or Stop refuses with "AUTO_TEST latest execution has no observed completion". Ask: the gate
   reads the job's end itself, or says so earlier than Stop. G50 (2026-11-11) names the
   `command_status` action in the Stop refusal; reading the exit from the `bucket_wait` payload
   is not built.

## SymForge report items still open

- 2, no-repo task: partly fixed (#83). A start with an empty scope is still refused:
  "autonomous task scope cannot be empty". A read-only or merge-only prompt has to invent a scope.
- 4, `revise` friction. Messages seen: "prompt event already consumed by a different proposal",
  "contract identity and initial goal cannot be rewritten"; a revision also clears receipts.
  Effect: one task per prompt, and widening scope mid-turn is not possible.
- 5, sub-agents in linked worktrees: same as item 1 above.
- 6, Stop during background work: fixed for `Agent` launches (G16); a `Workflow` launch is not
  recognised as outstanding background work.
- 8, five prompt events for one message: not reproduced; possibly hooks registered twice.
- 10, redirected or piped check commands do not count as the criterion's command; a piped one
  only gets a note (G19).
- 11: same as pending-job answer above.
- 13, "advisory" labels: `intent_apply` answers say `mode: advisory`,
  `answer_authority: advisory` while the hooks enforce; agents read it as optional. Wording only.
- 14, a teammate's message arriving mid-turn is treated as a changed prompt: "GATE: current
  prompt intent changed; retry before stopping". Only `<task-notification>` is exempt today.

Fixed and merged, for reference: 1 dead-session lock (#73; a crash or kill still needs a resume
or the person's `supervisor task cancel`), 3 inherited dirty files (#94, G48), 7 Stop gate in
sub-agents with an unchanged tree (#78), 9 harmless environment variables (#83), 12 wrong
`continue` advice after completion (#91, G47).

## Other notes

- CI has no job timeout: a Windows run of
  `concurrent_updates_serialize_without_lost_journal_lines` once hung 35 minutes until cancelled
  (PR #79), then passed on re-run.
- Owner decisions still outstanding from #83 and #79: built-in versus per-project list of
  harmless environment variables; whether thread-count and colour variables belong on it; the
  sentence added to `CLAUDE.md` principle 2; other skills and web fetch staying refused before an
  intent.
- On the home PC `supervisor status` shows `registration: not_configured` under 0.19.1 although
  the hooks fire. Not investigated. (Also seen on the work PC for a project whose hooks come from
  a `--settings` file.)
- G48 limits are in PR #94's description.

## Decision recorded on the home PC, 2026-11-11 (from shared memory)

No "remote evidence" criterion (a CI result or reviewer verdict fetched by the local gate) will be
built. Reason: the local hook cannot observe at merge time (Completed is terminal), there is no
network code, a `gh` on PATH can be replaced, and a new check kind would need contract schema 3.
The SymForge lead's objections are met by receipt export plus the CI job
`Verify supervisor receipts` (WP7) together with branch protection (required check, required
approval, dismiss stale approvals), which the owner sets in GitHub. An optional local refusal at
Stop unless the tree is clean and HEAD is pushed is not built.

## The follow-up brief, as recovered on 2026-11-11

Observed on supervisor 0.13.1 from a session in another project. "Nothing here is approved yet."

1. A second session in the same project is locked out completely. Once another live session
   started a task, this session was refused everything ("GATE: this task belongs to another
   session; explicit user resume is required before mutation"), read-only shell and a file write
   outside the project included. Options: (a) one task per session when path scopes do not
   overlap; (b) one task per git worktree; (c) one task per project, with a real read-only mode
   for other sessions. In every option, writes outside the project root should not be gated by
   that project's task. Accept when: two live sessions with disjoint scopes or worktrees can each
   start, verify and complete; overlapping scopes are refused with the overlap named.
   Shared memory (2026-11-10) records the owner's words as "b, and four first"; the home-PC
   session's last note calls option (b) its own leaning. To be confirmed with the owner.
2. Read-only shell commands are treated as mutation. Refused: `git ls-remote`, `ls`, `which`.
   Wanted: an allow-list (git status/log/diff/show/ls-remote, ls, cat, which, gh pr view,
   gh repo view), whole-command match, no redirects, pipes into writers or chaining.
3. The hook advises cancelling a task that may be live. Say whether the owner is live; offer
   cancel only when it is not. Mostly done by #73; the cancel wording was not re-checked.
4. A session is not told when its task is accepted. Wanted: one line with task id, state and
   counted receipts.
5. A stop while a required check is still running counts as a refusal ("GATE refuses completion
   (refusal 1 of 3)" three times while the suite ran). Wanted: "pending: job <id> still running",
   without using a refusal.

## Older friction report (another project's session, 2026-11-06, supervisor 0.11.1)

Supervisor items it raised; none re-checked against `main` yet:

- A1.1 check discovery missed a virtual Cargo workspace manifest in a direct subfolder and a
  `package.json` two levels down; A1.2 the hook's own `git diff --check` criterion did not count
  as a repository check.
- A1.3 no documented way for a person to declare a repository's checks.
- A1.4 contract fields learnable only by trial, one missing field per refusal.
- A1.5 the intent requirement re-injected on every prompt after a non-retryable refusal;
  A1.6 also on system notifications.
- A1.7 "advisory" label beside "required before mutations" (same as SymForge item 13).
- A1.8 lower-cased project path; A1.9 worktree binding untested there.
- A3 a read of the gate's state folder is refused with a message that says "not writable".
