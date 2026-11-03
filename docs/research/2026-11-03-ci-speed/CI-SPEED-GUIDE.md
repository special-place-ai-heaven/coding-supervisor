# Faster, cheaper GitHub Actions CI: what was done in coding-supervisor and how to reuse it

Audience: a coding agent that specialises in CI and is asked to apply these changes to another
repository of the same owner. Each section says what the problem was, what was done, the exact
mechanism, how to check it worked, and what to watch for. Measured on
`special-place-ai-heaven/coding-supervisor` (Rust workspace, ~400 tests, Windows and Linux),
2026-11-02 to 2026-11-03. Source of truth: `.github/workflows/ci.yml`, `pick-runner.yml`,
`release.yml` and `release-please.yml` in that repository; read them before copying.

## Results

| Measure | Before | After |
|---|---|---|
| CI runs per pull-request push | 2 (push + pull_request) | 1 |
| CI runs for a docs-only change | 1 to 2 | 0 |
| CI runs for release-please's version-only PR and its merge | 1 each | 0 (the release tag runs CI once) |
| Linux job (fmt, clippy, tests, dependency check) | 3m0s | 1m55s to 2m0s |
| Windows job | 8m40s to 10m30s (fmt + clippy + tests) | 5m27s to 6m47s (tests only) |
| Git processes per tool-call hook (product, not CI) | up to 19 | 3 |

## The owner's standing rules (apply them everywhere)

1. **Self-hosted first, hosted as fallback.** The owner has five self-hosted Linux runners labelled
   `fresh` on a home PC that is not always on. Linux jobs use one when it is online and idle,
   GitHub-hosted `ubuntu-latest` otherwise. A job must never wait for a machine that is off.
2. **Windows runs hosted** (`windows-latest`): there is no self-hosted Windows runner.
3. **Pull requests always run hosted**, so code from a fork never executes on the owner's machine.
4. **Remove all waste** (owner, 2026-11-03): no duplicate runs, no runs that cannot change the
   outcome, no work repeated on a second operating system when the result cannot differ.
5. **Run in parallel where possible.** Five Linux runners exist; use them.
6. **On the owner's PC itself, at most one full test suite at a time**; parallel agent lanes run
   only their named tests with workers capped. CI is where full suites belong.

## 1. One CI run per change

**Problem.** `on: [push, pull_request]` runs everything twice for every push to a PR branch (once
for the branch push, once for the PR), and pushes to any branch or tag ran it too.

**Fix.**

```yaml
on:
  push:
    branches: [main]          # only main, never every branch or tag
    paths-ignore: [...]       # see section 2
  pull_request:
    paths-ignore: [...]
  workflow_call:              # so release.yml can run the same CI once on a tag

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true    # a newer push cancels the superseded run of the same ref
```

**Check.** Push to a PR branch; `gh run list --branch <branch>` shows exactly one CI run.

**Watch for.** Branch protection that requires a status check by its old name (for example a
`push`-triggered job name) blocks merges after the change; update the required check names.

## 2. Do not run CI for changes that cannot affect it

**Docs-only changes.** `paths-ignore` on both triggers: `docs/**`, `README.md`, `CHANGELOG.md`,
`CLAUDE.md`, `HANDOVER.md`, `LICENSE`. Before adding a path, check that nothing compiles it in:
in coding-supervisor the skill templates under `skills/` are embedded into the binary with
`include_str!`, so they must not be ignored. Grep for `include_str!`, `include_bytes!`, asset
bundlers and snapshot fixtures before ignoring a folder.

**Required-check trap.** If a branch rule requires the CI check, a PR that skips CI by
`paths-ignore` never reports it and cannot merge. Either don't require it, or use a job-level
skip (below) that still reports success.

**Release-bot changes.** release-please's PR only bumps versions and the changelog, and its merge
into `main` is the same content. A small first job decides; every other job `needs` it:

```yaml
changes:
  runs-on: ubuntu-latest
  outputs: { run: ${{ steps.decide.outputs.run }} }
  steps:
    - id: decide
      env:
        HEAD_REF: ${{ github.head_ref }}
        REF: ${{ github.ref }}
        MESSAGE: ${{ github.event.head_commit.message }}
      run: |
        run=true
        case "$HEAD_REF" in release-please--*) run=false ;; esac
        if [ "$REF" = "refs/heads/main" ] && printf '%s' "$MESSAGE" | head -n 1 | grep -q 'release-please--branches--'; then
          run=false
        fi
        echo "run=$run" >> "$GITHUB_OUTPUT"
```

Skipped jobs report as skipped (success for required checks). The release tag still runs the full
CI once through `workflow_call` before anything is published (section 7).

Pass event fields through `env:`, never by `${{ }}` inside the script: a commit message is
attacker-controlled text and would otherwise be spliced into the shell.

## 3. Pick the runner at run time (self-hosted when idle, hosted otherwise)

A reusable workflow `pick-runner.yml` runs on hosted Ubuntu (a few seconds), asks the GitHub API
for the organisation's runners, and returns a JSON `runs-on` value:

```yaml
on:
  workflow_call:
    outputs:
      linux: { value: ${{ jobs.pick.outputs.linux }} }
jobs:
  pick:
    runs-on: ubuntu-latest
    outputs: { linux: ${{ steps.pick.outputs.linux }} }
    steps:
      - id: pick
        env:
          STATUS_TOKEN: ${{ secrets.RUNNER_STATUS_TOKEN }}
          RELEASE_TOKEN: ${{ secrets.RELEASE_PLEASE_TOKEN }}
          ORG: ${{ github.repository_owner }}
          EVENT: ${{ github.event_name }}
        run: |
          hosted='"ubuntu-latest"'
          fresh='["self-hosted","Linux","X64","fresh"]'
          choice="$hosted"
          filter='[.runners[] | select(.status == "online" and .busy == false and any(.labels[]; .name == "fresh"))] | length'
          if [ "$EVENT" != "pull_request" ]; then
            for name in STATUS_TOKEN RELEASE_TOKEN; do
              token="${!name}"; [ -n "$token" ] || continue
              if counts=$(GH_TOKEN="$token" gh api "orgs/$ORG/actions/runners" --paginate --jq "$filter" 2>/dev/null); then
                idle=$(echo "$counts" | awk '{s += $1} END {print s + 0}')
                [ "$idle" -gt 0 ] && choice="$fresh"
                break
              fi
            done
          fi
          echo "linux=$choice" >> "$GITHUB_OUTPUT"
```

Callers: `uses: ./.github/workflows/pick-runner.yml` with `secrets: inherit`, then
`runs-on: ${{ fromJSON(needs.runner.outputs.linux) }}`. In a matrix, store the value as a JSON
string per entry (`os: '"windows-latest"'` and `os: ${{ needs.runner.outputs.linux }}`) and use
`runs-on: ${{ fromJSON(matrix.os) }}`.

Rules built in: pull requests always hosted; any failure to read the list (token missing, no
permission, API error) falls back to hosted, never to waiting. The token needs read access to the
organisation's self-hosted runners (`RUNNER_STATUS_TOKEN`, org-wide secret). Known limit
(`shortcut:` comment in the file): idleness is checked once; if another job takes the runner
first, this job queues for the next free `fresh` runner.

**Check.** The job log prints `idle fresh runners: N` and the choice; the job's runner name in
`gh api repos/<o>/<r>/actions/runs/<id>/jobs --jq '.jobs[] | .runner_name'` is a `fresh` machine
when one was idle.

**Do not use** a self-hosted runner for `pull_request` from forks, for anything with write tokens
in a fork context, or for `pull_request_target`.

## 4. Make jobs work on bare self-hosted runners without `sudo`

The `fresh` runners are minimal and have no password-less `sudo`. Every tool install must be
user-local and idempotent:

- **Rust:** install rustup only if missing, with no default toolchain, then let it read the pinned
  `rust-toolchain.toml` (single source of the version; `dtolnay/rust-toolchain` cannot read it):
  ```sh
  command -v rustup >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal
  echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"; export PATH="$HOME/.cargo/bin:$PATH"
  rustup toolchain install
  ```
- **cargo-nextest:** `taiki-e/install-action` calls `sudo` on Linux and failed on the fresh runners
  with a password prompt. Use the prebuilt archive into the cargo bin folder, once per runner:
  ```sh
  command -v cargo-nextest >/dev/null || curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C "${CARGO_HOME:-$HOME/.cargo}/bin"
  ```
  On hosted Windows `taiki-e/install-action@nextest` is fine.
- **Environment leaks from the runner:** the fresh runners export `RUSTC_WRAPPER=sccache` without
  sccache installed, so every build failed. Workflow-level `env: RUSTC_WRAPPER: ""` turns it off.
  Look for similar inherited variables (`CC`, `NODE_OPTIONS`, proxies) when a job passes hosted and
  fails self-hosted.
- **Caches:** `Swatinem/rust-cache@v2` with `continue-on-error: true` on self-hosted jobs, so a
  cache service hiccup never fails a build.
- **Interpreters:** Ubuntu images have `python3` but often no `python`; check and install only if
  missing (this one does use `sudo` and only runs on hosted images in practice).

**Check.** The first run on a fresh runner passes the install steps; the second run skips them
(the `command -v` guard).

## 5. Do each check once, on the cheapest machine that can answer it

- **Formatting, lint and dependency policy run on Linux only.** They give the same answer on every
  operating system. Windows runs only the tests. This alone removed about 3 minutes from the
  Windows job, the longest pole.
- **Windows stays for tests** because path handling and process behaviour differ there.
- Keep one job per operating system rather than a matrix that repeats every step.

## 6. Run tests in parallel: cargo-nextest

`cargo test` runs test binaries one after another; nextest runs every test in its own process and
test files in parallel, and `--no-fail-fast` reports every failure in one run.

```sh
cargo nextest run --workspace --no-fail-fast
```

Differences to check before switching: nextest does not run doctests (add `cargo test --doc` if
the project has any); tests that share global state, a fixed port or a fixed temp path collide
when run in parallel processes (give each a unique path or port; in coding-supervisor each test
uses `temp_dir()` plus process id plus a counter); timing-bound tests can fail under heavy local
load (here a 15 s hook bound failed only when ~400 tests ran at once on a loaded workstation and
passed alone and in CI).

For other ecosystems the same idea: `jest --maxWorkers` / `vitest --pool`, `pytest -n auto`
(pytest-xdist), `go test -p`.

## 7. One CI run per release, on the tag, before publishing

release-please opens a version PR, the workflow auto-merges it, release-please tags the merge
commit (with a PAT, `RELEASE_PLEASE_TOKEN`: a tag created with `GITHUB_TOKEN` does not start other
workflows), and the tag runs `release.yml`:

- job `ci` calls `ci.yml` through `workflow_call`: the only CI run the release gets;
- build jobs `needs: [ci, runner]`; Linux build on the picked runner, Windows hosted;
- the GitHub release is created as a **draft** and published only after CI and every build pass,
  so a red run leaves a draft and nothing on npm;
- npm platform packages first, the launcher last, waiting until the registry lists them.

The commit-subject check (`conventional_commits.py`) runs in the release-please workflow and
fails on any non-conventional subject, so release notes stay complete.

## 8. Remaining waste (not yet fixed here; candidates)

- **Release-cutting merges start a `main` run that is cancelled.** A `feat:`/`fix:` merge starts
  CI on `main`; about a minute later the release-please merge pushes to `main`, and
  `cancel-in-progress` cancels the first run (~2.5 runner-minutes wasted per release); the tag then
  runs CI in full. Fix: skip the `main` push run when the merge will cut a release (the `changes`
  job can check for `feat`/`fix` subjects since the last tag), or drop `main` push runs entirely
  when branch protection requires PRs to be up to date with `main` (the PR run then tested the
  same tree).
- **Shard tests across the five fresh runners:** `cargo nextest run --partition count:K/N` in a
  matrix of N jobs; build once with `cargo nextest archive` and share the archive as an artifact
  so N runners do not compile N times. Worth it once the Linux test step exceeds a few minutes.
- **sccache for real** on the fresh runners (local disk cache), instead of turning the wrapper off.
- **Windows I/O:** on hosted Windows, a Dev Drive (ReFS) for the target directory speeds up Rust
  and Node builds; on self-hosted Windows, exclude the work folder from Defender scanning.
- **Per-job path filters** (`dorny/paths-filter`) when a repository has independent parts (for
  example `backend/` and `frontend/`): run each part's jobs only when it changed; keep a full run
  on the release tag.

## 9. Speed inside the code under test

CI time is often spent in the product, not the pipeline. In coding-supervisor every tool-call hook
started up to 19 `git` processes (16 identical `rev-parse --show-toplevel`); process start costs
tens of milliseconds on Windows, multiplied across every hook-driven test. Fix: ask git once per
process and remember found roots (validated cheaply by checking `.git` on disk), merge two
`rev-parse` calls into one. A test seam (`SUPERVISOR_GIT_TRACE=<file>` appends each subcommand) and
a test pin the count at 3. Look for the same pattern elsewhere: repeated subprocesses, repeated
network calls, sleeps in tests.

## 10. Working practice that saves CI rounds

- Run the formatter check before every push (two CI rounds were lost to formatting alone).
- Run the whole affected test files locally, not one test, before pushing.
- Watch runs with `gh run watch <id> --exit-status` in the background instead of polling.
- Cancel superseded or stale runs (`gh run cancel <id>`); `cancel-in-progress` does it for new
  pushes to the same ref.
- When a job passes hosted and fails self-hosted, diff the environment first (`env | sort`,
  `command -v` of each tool, `sudo -n true`).

## Applying this to another repository: checklist

1. Read its workflows; list triggers, jobs per OS, and what each job checks.
2. Triggers: `push` to the default branch only, `pull_request`, `workflow_call`; `concurrency`
   with `cancel-in-progress`.
3. `paths-ignore` for docs, after checking nothing embeds them; adjust required checks.
4. Add the `changes` job if a release bot opens version-only PRs.
5. Copy `pick-runner.yml`; make sure the repository can read `RUNNER_STATUS_TOKEN` (org secret
   visibility); point Linux jobs at it; keep PRs hosted.
6. Make every install step user-local and guarded by `command -v`; set `RUSTC_WRAPPER: ""` (or the
   project's equivalent) if the fresh runners leak it.
7. Move OS-independent checks (format, lint, dependency policy, type check) to Linux only.
8. Switch the test runner to its parallel mode; fix shared temp paths and ports it exposes.
9. Release: CI once on the tag via `workflow_call`, draft release until builds pass.
10. Measure before and after from `gh run view <id> --json jobs` (`startedAt`, `completedAt`) and
    record the numbers in the repository's docs.
