# Adopting receipt verification in another repository

as_of 2026-11-11. Requires a `supervisor` with `receipt export` and `receipt verify --bundle`
(WP7). The reference job is `receipts` in this repository's `.github/workflows/ci.yml`.

## What it enforces

Every pull request to `main` must add (or change) at least one file under
`supervisor-receipts/` that `supervisor receipt verify --bundle` accepts, and CI re-runs the
acceptance commands of each bundled contract. Exemptions are decided by
`supervisor receipt required` from data CI reads itself: a documentation-only change (the base
commit's `.github/docs-paths.txt`), release-please's version-only pull request (head branch
`release-please--branches--main...`, opened by the release author, touching only the files the
base commit's `.github/release-please-config.json` lists plus the manifest), or the label
`receipt-exempt` whose last application in the pull request's timeline was by the owner or a
listed maintainer. Branch names exempt nothing on their own. A push, a tag and a pull request to
another base need nothing; the job is then a no-op success.

## Steps

1. **Key list.** On a machine that runs the supervised tasks, run `supervisor receipt export`
   (or `supervisor receipt sign`) once; it prints a `keys_entry`. Commit it to
   `supervisor-keys.json` at the repository root in its own pull request, merged before any
   pull request that relies on it: CI reads the key list from the base commit only, never from
   the pull request. To retire a key, set its `revoked_at`.
2. **Job.** Copy the `receipts` job from this repository's `ci.yml` into the new repository's
   workflow (it needs no other job). Set `OWNER_LOGIN` to the person who may apply the label;
   for an organisation-owned repository this must be a person's login, because the default
   (the repository owner) is the organisation, which never applies labels. Add
   `--maintainers a,b` to the `receipt required` line for more people, and
   `--release-author <login>` when release-please's pull requests are opened by another
   identity than the owner.
3. **Required check.** In the branch protection or ruleset for `main`, require the check named
   exactly `Verify supervisor receipts`.
4. **Label.** Create the label `receipt-exempt`. After applying it, re-run the job: label events
   do not start CI, and the re-run reads the timeline again.
5. **Per pull request.** Finish the task under the hooks, run `supervisor receipt export`, and
   commit the bundle with the work. Start the task from the commit the branch leaves `main`
   at: CI recomputes the changed paths from the merge base, and a bundle from a later start
   lists fewer paths and is refused. After a rebase or a merge of `main`, export again.

## Limits

- The key is this machine's and the user's: an agent running as the same OS user can read it,
  so a signature is an attestation, not proof. The CI re-run of the acceptance commands is the
  independent check.
- On a same-repository pull request the workflow file and the verifier binary both come from
  the pull request itself; the protection rests on review plus the key list from the base
  commit. A fork's pull request has the same limit for the binary it builds.
- A label applied through the owner's own token (for example by an agent using the owner's
  `gh` login) cannot be told apart from the owner applying it.
- release-please's pull requests in this repository are opened with the owner's own token, so
  the author check adds little there; the changed-file check carries that rule.
- The bundle publishes the contract (goal, check names, commands, scope) as written by the
  person or the agent; `approval.by` is replaced by a label and an unsalted SHA-256 of the
  name.
- Commands recorded on Windows are run unchanged with `bash` on Linux; one that needs Windows
  (PowerShell, `.exe` paths, backslash paths, `cmd` syntax) fails the job. Environment the
  command relied on locally (for example `CARGO_TARGET_DIR`) is not recorded and not set.
- Documentation paths are outside the tree and the changed-path list, so they are not checked
  against the scope.
- Two bundles in one pull request can both verify only if they name the same tree; in practice
  one task per pull request.
- A mandatory criterion of kind `baseline_reproduces_reported_failure` binds to the task's
  starting candidate, so `supervisor verdict`, and therefore export, refuse such a task.
- The job builds `supervisor` on every pull request, documentation-only ones included, and the
  re-run repeats part of the test job.
