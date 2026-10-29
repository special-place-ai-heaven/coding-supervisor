#!/usr/bin/env python
"""Validate commit subjects against the conventional commits format used by release-please.

Copied from symforge execution/conventional_commits.py; only the default repository root differs.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Sequence

ALLOWED_TYPES = (
    "build",
    "chore",
    "ci",
    "docs",
    "feat",
    "fix",
    "perf",
    "refactor",
    "revert",
    "style",
    "test",
)

IGNORED_PREFIXES = (
    "Merge pull request #",
    "Merge branch ",
    "Merge remote-tracking branch ",
    # `wip:` checkpoints are an EXPLICIT changelog opt-out: release-please
    # ignores non-conventional subjects, and a deliberately-typed checkpoint
    # inside a merged feature branch is not the typo this gate exists to
    # catch. Without this, one historical `wip:` commit wedges every
    # subsequent Release run on main (2026-07-11, run #763 on 77990fb).
    "wip: ",
)


def repo_root(path: str | None = None) -> Path:
    if path is not None:
        return Path(path).resolve()
    return Path(__file__).resolve().parents[2]


def is_ignored_subject(subject: str) -> bool:
    return subject.startswith(IGNORED_PREFIXES)


def is_conventional_subject(subject: str) -> bool:
    for commit_type in ALLOWED_TYPES:
        if not subject.startswith(commit_type):
            continue

        remainder = subject[len(commit_type) :]
        if remainder.startswith("("):
            closing = remainder.find(")")
            if closing <= 1:
                return False
            remainder = remainder[closing + 1 :]

        if remainder.startswith("!"):
            remainder = remainder[1:]

        return remainder.startswith(": ") and len(remainder) > 2

    return False


def check_subjects(subjects: list[str]) -> list[str]:
    problems: list[str] = []

    for subject in subjects:
        if is_ignored_subject(subject):
            continue
        if not is_conventional_subject(subject):
            allowed = ", ".join(ALLOWED_TYPES)
            problems.append(
                f"'{subject}' is not a conventional commit subject. "
                f"Expected one of: {allowed}. Example: fix(ci): describe the change"
            )

    return problems


def read_commit_subjects(root: Path, rev_range: str) -> list[str]:
    # `--no-merges` matches release-please's own behavior: merge commits are
    # joins, not changes, so they are not bump inputs. This lets parallel-agent
    # workflows that produce custom merge subjects (e.g. "Merge swarm-N: ...")
    # pass validation without rewriting history.
    result = subprocess.run(
        ["git", "log", "--no-merges", "--format=%s", rev_range],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"git log failed for range '{rev_range}'")
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def run_git(root: Path, args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
    )


def commit_exists(root: Path, rev: str) -> bool:
    result = run_git(root, ["rev-parse", "-q", "--verify", f"{rev}^{{commit}}"])
    return result.returncode == 0


def is_ancestor(root: Path, older: str, newer: str) -> bool:
    result = run_git(root, ["merge-base", "--is-ancestor", older, newer])
    return result.returncode == 0


def resolve_push_range(root: Path, before: str, after: str) -> tuple[str, str | None]:
    zero = "0000000000000000000000000000000000000000"
    if before == zero:
        return f"{after}^!", "push.before is all-zero; validating only the pushed commit"
    if not commit_exists(root, before):
        return (
            f"{after}^!",
            f"push.before {before} is not present after checkout (likely force-push); validating only {after}",
        )
    if not is_ancestor(root, before, after):
        return (
            f"{after}^!",
            f"push.before {before} is not an ancestor of {after}; validating only the rewritten tip commit",
        )
    return f"{before}..{after}", None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate commit subjects against release-please-friendly conventional commits."
    )
    parser.add_argument(
        "--root",
        default=None,
        help="Repository root to inspect. Defaults to the current repo.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_subject = subparsers.add_parser(
        "check-subject",
        help="Validate one commit subject or PR title.",
    )
    check_subject.add_argument("subject", help="Subject/title to validate.")

    check_range = subparsers.add_parser(
        "check-range",
        help="Validate every commit subject in a git revision range.",
    )
    check_range.add_argument("rev_range", help="Git revision range, for example HEAD~3..HEAD.")

    check_push_range = subparsers.add_parser(
        "check-push-range",
        help="Validate commit subjects for a GitHub push event, tolerating force-push rewrites.",
    )
    check_push_range.add_argument("before", help="github.event.before SHA")
    check_push_range.add_argument("after", help="GitHub SHA after the push")

    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    root = repo_root(args.root)

    try:
        if args.command == "check-subject":
            problems = check_subjects([args.subject])
        elif args.command == "check-range":
            subjects = read_commit_subjects(root, args.rev_range)
            if not subjects:
                print(f"No commits found in range {args.rev_range}.")
                return 0
            problems = check_subjects(subjects)
        elif args.command == "check-push-range":
            rev_range, note = resolve_push_range(root, args.before, args.after)
            if note:
                print(note, file=sys.stderr)
            subjects = read_commit_subjects(root, rev_range)
            if not subjects:
                print(f"No commits found in range {rev_range}.")
                return 0
            problems = check_subjects(subjects)
        else:
            return 2
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 1

    if problems:
        for problem in problems:
            print(problem, file=sys.stderr)
        return 1

    if args.command == "check-subject":
        print("Conventional commit subject check passed.")
    elif args.command == "check-range":
        print(f"Conventional commit range check passed: {args.rev_range}")
    else:
        rev_range, _ = resolve_push_range(root, args.before, args.after)
        print(f"Conventional commit push-range check passed: {rev_range}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
