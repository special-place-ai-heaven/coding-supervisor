# Adopting receipt verification in another repository

as_of 2026-11-11. Requires a `supervisor` with `receipt export` and `receipt verify --bundle`
(WP7). The reference job is `receipts` in this repository's `.github/workflows/ci.yml`.

## What it enforces

Every pull request, whatever its base, must add (or change) at least one file under
`supervisor-receipts/` that `supervisor receipt verify --bundle` accepts, and CI re-runs the
acceptance commands of each bundled contract. Exemptions are decided by
`supervisor receipt required` from data CI reads itself: a head commit already on the default
branch's tip (CI ran after the merge; the job passes `--head-on-default-branch` only when
`git merge-base --is-ancestor <head> <tip>` exits 0), a documentation-only change (the default
branch's `.github/docs-paths.txt`), release-please's version-only pull request (head branch
exactly `release-please--branches--main--components--coding-supervisor`, or the name given with
`--release-branch`, opened by the release author, touching only the files the default branch's
`.github/release-please-config.json` lists plus the manifest), or the label `receipt-exempt`
whose last application in the pull request's timeline was by the owner or a listed maintainer
and later than the pull request's last update. Branch names exempt nothing on their own. The
base branch does not matter: changing a pull request's base to `main` starts no new run, so a
pass given while it targeted another branch would stay on the head commit. The job never uses the event's base: it reads the key list, the
documentation list and the release-please config from the default branch's tip (fetched at run
time) and takes the changed paths from where the pull request leaves that branch, so a pull
request to another branch is judged on everything it would bring to the default branch. A push
to the default branch and a tag (release.yml's call carries its tag push) are a no-op success;
the job fails on any other event. `supervisor receipt required` itself answers "nothing
required" only for a push to `main` and a tag, and "required" for any other event shape.

## Steps

1. **Key list.** On a machine that runs the supervised tasks, run `supervisor receipt export`
   (or `supervisor receipt sign`) once; it prints a `keys_entry`. Commit it to
   `supervisor-keys.json` at the repository root in its own pull request, merged before any
   pull request that relies on it: CI reads the key list from the default branch's tip only, never from
   the pull request. To retire a key, set its `revoked_at`.
2. **Job.** Copy the `receipts` job from this repository's `ci.yml` into the new repository's
   workflow (it needs no other job). Its toolchain, cache and build steps build `supervisor`
   from this repository's own sources, which another repository does not have: replace them
   with an install of a pinned released `supervisor` (for example `npm install -g
   coding-supervisor@<version>`), checked by version. As of this writing no release carries
   `receipt export`, `receipt verify --bundle` or `receipt required` yet. If the workflow is
   called from another workflow (as `release.yml` calls `ci.yml` here), the calling job must
   grant `pull-requests: read` and `issues: read` too. Set `OWNER_LOGIN` to the person who may apply the label;
   for an organisation-owned repository this must be a person's login, because the default
   (the repository owner) is the organisation, which never applies labels. Add
   `--maintainers a,b` to the `receipt required` line for more people, and
   `--release-author <login>` when release-please's pull requests are opened by another
   identity than the owner.
3. **Required check.** In the branch protection or ruleset for `main`, require the check named
   exactly `Verify supervisor receipts`.
4. **Label.** Create the label `receipt-exempt`. The owner's steps: push the last commit first;
   then apply the label; then re-run the `Verify supervisor receipts` job (label events do not
   start CI; the re-run reads the timeline again). The label counts only when it was applied
   after the pull request's last update in the event that started the run, so after any later
   push: remove the label, apply it again, and re-run the job.
5. **Per pull request.** Finish the task under the hooks, run `supervisor receipt export` right
   away, and commit the bundle with the work. Export runs verify's scope check on the contract's `base_sha`
   and refuses, naming the paths, when a changed path is outside the contract's scope (for
   example a file already changed before the task started, which the local gate does not charge
   to the task). Export refuses once the completed work has been
   committed on its own: the completion stays bound to the candidate it was accepted on. Start the task from the commit the branch leaves `main`
   at: CI recomputes the changed paths from the merge base, and a bundle from a later start
   lists fewer paths and is refused. After a rebase or a merge of `main`, export again.

## Limits

- The key is this machine's and the user's: an agent running as the same OS user can read it,
  so a signature is an attestation, not proof. The CI re-run runs the commands the contract
  names; a contract can name a trivial command, so the contract is part of what a person
  reviews.
- On every pull request, a fork's included, the workflow file and the verifier binary both
  come from the pull request itself; the protection rests on review plus the key list from the
  default branch. A re-run reads the default branch's tip again at its own run time.
- The job fetches the default branch with no stored credentials; that works for a public
  repository. A private one needs that fetch to authenticate.
- The label is tied to a time, not to a head commit: a push in the seconds before the owner
  applies it is covered. `pull_request.updated_at` on a real push payload has not been observed
  yet (the job has never run).
- A label applied through the owner's own token (for example by an agent using the owner's
  `gh` login) cannot be told apart from the owner applying it.
- release-please's pull requests in this repository are opened with the owner's own token, so
  the author check adds little there, and an agent using the owner's token can open such a pull
  request (the same class as the label limit). The rule checks which files changed, not what
  changed in them.
- The bundle publishes the contract's goal, scope, criterion ids, check names, commands,
  `forbidden_effects`, `artifacts` and `approval.via` as written, and reviewer ids and times;
  `approval.by` is replaced by a label and an unsalted SHA-256 of the name.
- Repository identity is the root commits, so a fork passes as the same repository.
- Only the signed statement is covered by the signature; the `signature` object's other fields
  (`storage`, any extra key) are not.
- Anything under `supervisor-receipts/` is outside the local gate, and CI checks only the files a
  pull request adds or changes there.
- `--base`, the contract's `base_sha` (also when `--base` is given) and the bundle's top-level
  `base_sha` are checked only to be full commit ids.
- Commands recorded on Windows are run unchanged with `bash` on Linux; one that needs Windows
  (PowerShell, `.exe` paths, backslash paths, `cmd` syntax) fails the job. Environment the
  command relied on locally (for example `CARGO_TARGET_DIR`) is not recorded and not set.
- Documentation paths are outside the tree and the changed-path list, so they are not checked
  against the scope.
- Two bundles in one pull request can both verify only if they name the same tree; in practice
  one task per pull request.
- A mandatory criterion of kind `baseline_reproduces_reported_failure` binds to the task's
  starting candidate, so `supervisor verdict`, and therefore export, refuse such a task.
- A head merged by squash or rebase is not an ancestor of the default branch's tip, so a run
  after such a merge still needs a receipt. A pull request to another branch whose head commit
  is already on the default branch needs none.
- The job builds `supervisor` on every pull request, documentation-only ones included, and the
  re-run repeats part of the test job.
