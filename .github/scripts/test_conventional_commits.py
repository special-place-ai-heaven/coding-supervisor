# Copied from symforge execution/test_conventional_commits.py; the golden corpus is symforge's
# history, kept as test data. Only the scratch repositories moved to the system temp directory.
from __future__ import annotations

import subprocess
import tempfile
import unittest
import uuid
from pathlib import Path

import conventional_commits


# Golden corpus pinned from real symforge history.
# Each entry: (commit_sha_prefix, subject, expected_classification).
# Classifications:
#   "conventional"     -> is_conventional_subject is True; check_subjects yields no problem
#   "ignored"          -> is_ignored_subject is True; check_subjects skips it
#   "non_conventional" -> neither; check_subjects yields exactly one problem
#
# Reproducibility: the SHAs are real symforge commits as of this corpus's pinning.
# To regenerate, run from the repo root:
#     git log --format='%H %s' <sha>~..<sha>
# and confirm the subject still matches. New entries should be appended; do not
# reorder, so future bisects against this corpus stay stable.
GOLDEN_CORPUS_REAL: tuple[tuple[str, str, str], ...] = (
    # --- conventional, basic types from real history ---
    ("c8b5c51", "fix: restore full self-hosting Rust parsing", "conventional"),
    ("2173acb", "feat: derive fallback explore clusters", "conventional"),
    ("7c3dd1d", "perf: eliminate per-query allocations in trigram search", "conventional"),
    ("c7dc297", "refactor: add SYMFORGE_FRECENCY_DB_PATH constant", "conventional"),
    ("a925279", "docs: rewrite README with full tool reference, move verbose content to wiki", "conventional"),
    ("b45653f", "test: extend edit_hook_behavior parity tests with edge cases", "conventional"),
    ("0c728c9", "chore: trigger ci after title fix", "conventional"),
    # --- conventional, scoped ---
    ("77cabec", "fix(parsing): cap AST walk depth so recursive walkers don't stack-overflow", "conventional"),
    ("5c6a13b", "fix(ci): use cargo check for workflow verification", "conventional"),
    ("80c69e4", "fix(build): silence tree-sitter-scss scanner warnings", "conventional"),
    ("0c0fbe7", "fix(trust): tighten discovery and context signals", "conventional"),
    ("d5e86bc", "chore(main): release 6.1.0", "conventional"),
    ("e3062f0", "feat(worktree-awareness#4): health misuse counter + conventions answer", "conventional"),
    ("fff3666", "docs(worktree-awareness#5): README subsection + ADR 0010", "conventional"),
    ("9a14a7d", "fix(parsing#1): clamp error-snippet window to UTF-8 char boundaries", "conventional"),
    # --- conventional, breaking change ---
    ("b585660", "feat!: trust-calibrate SymForge release", "conventional"),
    # --- ignored merge commits (release-please / GitHub-style) ---
    ("cb16de1", "Merge pull request #222 from special-place-administrator/release-please--branches--main--components--symforge", "ignored"),
    ("a0cd3cc", "Merge pull request #214 from special-place-administrator/release-please--branches--main--components--symforge", "ignored"),
    # --- non-conventional, real history ---
    ("6485a52", "Add conformance suite for MCP tool surface", "non_conventional"),
    ("3c0d3b1", "Update README.md", "non_conventional"),
    ("3a3abe0", "remove unused separator match", "non_conventional"),
    ("eb991d6", "remove legacy spacetime scaffolding", "non_conventional"),
    ("653a319", "Remove local statusLine override — use global config instead", "non_conventional"),
    # lowercase "merge" prefix is NOT in IGNORED_PREFIXES — pin this behavior
    ("24268ec", "merge parsing tentacle: 4 parser audit/test items (UTF-8 fix, Ruby xref, diagnostic contract, idempotence)", "non_conventional"),
    ("3ba2acd", "merge swarm-1: ArcSwap concurrent-read stress test", "non_conventional"),
    ("0b26e28", "merge: edit-and-ranker-hooks swarm — head_sha/commit_distance helpers + SYMFORGE_FRECENCY_DB_PATH", "non_conventional"),
    # capitalized "Merge" but NOT one of the three IGNORED_PREFIXES variants
    ("4ddecc5", "Merge todo #1: parity harness (edit_hooks + rank_signals)", "non_conventional"),
)


# Synthetic edge cases — pin parser behavior on shapes that don't appear in real
# history but would otherwise drift if the parser were rewritten. Keep these
# stable: the value of pinning edge cases is precisely that they catch silent
# rule changes.
GOLDEN_CORPUS_SYNTHETIC: tuple[tuple[str, str], ...] = (
    # ALLOWED_TYPES not represented in real symforge history
    ("revert: bring back legacy parser", "conventional"),
    ("style: re-format module per rustfmt", "conventional"),
    ("build: bump tree-sitter to 0.22", "conventional"),
    ("ci: pin actions/checkout to v5", "conventional"),
    # Scoped breaking change variant
    ("feat(api)!: drop deprecated /v1 routes", "conventional"),
    # Two ignored-merge prefix variants not in repo history
    ("Merge branch 'main' into topic", "ignored"),
    ("Merge remote-tracking branch 'origin/main'", "ignored"),
    # Edge cases: invalid forms must NOT classify as conventional
    ("feat:no-space-after-colon", "non_conventional"),
    ("feat: ", "non_conventional"),
    ("feat", "non_conventional"),
    ("feat(): empty scope", "non_conventional"),
    ("FEAT: uppercase type", "non_conventional"),
    ("Feat: title case type", "non_conventional"),
    # type prefix accidentally appears at start but isn't actually a real type
    ("featuring: not an allowed type", "non_conventional"),
    ("chores: trailing-s typo", "non_conventional"),
)


class ConventionalCommitTests(unittest.TestCase):
    def test_accepts_basic_conventional_subject(self) -> None:
        self.assertEqual(conventional_commits.check_subjects(["fix: handle daemon proxy drift"]), [])

    def test_accepts_scoped_breaking_subject(self) -> None:
        self.assertEqual(
            conventional_commits.check_subjects(["feat(cli)!: require explicit project root"]),
            [],
        )

    def test_accepts_release_chore_subject(self) -> None:
        self.assertEqual(
            conventional_commits.check_subjects(["chore(main): release 4.9.6"]),
            [],
        )

    def test_ignores_merge_commit_subjects(self) -> None:
        self.assertEqual(
            conventional_commits.check_subjects(
                ["Merge pull request #186 from special-place-administrator/release-please"]
            ),
            [],
        )

    def test_ignores_wip_checkpoint_subjects(self) -> None:
        """A `wip:` checkpoint is an explicit changelog opt-out (release-please
        skips non-conventional subjects), not the typo this gate catches —
        one historical checkpoint must not wedge the Release run on main."""
        self.assertEqual(
            conventional_commits.check_subjects(
                ["wip: checkpoint outstanding-work hardening"]
            ),
            [],
        )
        # A bare `wip` without the marker shape is still a problem.
        problems = conventional_commits.check_subjects(["wip checkpoint"])
        self.assertEqual(len(problems), 1)

    def test_rejects_nonconventional_subject(self) -> None:
        problems = conventional_commits.check_subjects(["Add conformance suite for MCP tool surface"])
        self.assertEqual(len(problems), 1)
        self.assertIn("not a conventional commit subject", problems[0])

    def test_golden_corpus_real_history(self) -> None:
        """Pin classification of real symforge commits across all three outcomes.

        The corpus snapshots representative commits from `git log` so that any
        future change to the parser that flips a real commit's classification
        fails loudly with the offending SHA in the diff.
        """
        for sha, subject, expected in GOLDEN_CORPUS_REAL:
            with self.subTest(sha=sha, subject=subject):
                self._assert_classification(subject, expected)

    def test_golden_corpus_synthetic_edges(self) -> None:
        """Pin classification of synthetic edge cases the real corpus can't reach.

        Covers ALLOWED_TYPES with no real-history example (revert/style/build/ci),
        the two `Merge branch ...` ignored-prefix variants, and malformed shapes
        that must NOT classify as conventional (empty scope, missing space after
        colon, uppercase type, near-miss prefixes like `featuring:`).
        """
        for subject, expected in GOLDEN_CORPUS_SYNTHETIC:
            with self.subTest(subject=subject):
                self._assert_classification(subject, expected)

    def _assert_classification(self, subject: str, expected: str) -> None:
        is_ignored = conventional_commits.is_ignored_subject(subject)
        is_conventional = conventional_commits.is_conventional_subject(subject)
        problems = conventional_commits.check_subjects([subject])

        if expected == "ignored":
            self.assertTrue(is_ignored, f"expected ignored: {subject!r}")
            self.assertEqual(problems, [], f"ignored subject should produce no problems: {subject!r}")
        elif expected == "conventional":
            self.assertFalse(
                is_ignored, f"conventional subject should not be ignored: {subject!r}"
            )
            self.assertTrue(
                is_conventional, f"expected conventional: {subject!r}"
            )
            self.assertEqual(
                problems, [], f"conventional subject should produce no problems: {subject!r}"
            )
        elif expected == "non_conventional":
            self.assertFalse(
                is_ignored, f"non-conventional subject should not be ignored: {subject!r}"
            )
            self.assertFalse(
                is_conventional, f"expected non-conventional: {subject!r}"
            )
            self.assertEqual(
                len(problems), 1, f"non-conventional subject should produce exactly one problem: {subject!r}"
            )
            self.assertIn("not a conventional commit subject", problems[0])
        else:
            self.fail(f"unknown expected classification {expected!r} for {subject!r}")

    def test_read_commit_subjects_from_range(self) -> None:
        root = self.make_repo()
        self.git(root, "init")
        self.git(root, "config", "user.name", "Hermes")
        self.git(root, "config", "user.email", "hermes@example.com")

        (root / "README.md").write_text("one\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: first")

        (root / "README.md").write_text("two\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "feat: second")

        subjects = conventional_commits.read_commit_subjects(root, "HEAD~1..HEAD")
        self.assertEqual(subjects, ["feat: second"])

    def test_read_commit_subjects_skips_merge_commits(self) -> None:
        """Merge commits with custom subjects (e.g. from parallel-agent workflows)
        must not appear in the validation range. Locks in the `--no-merges` policy."""
        root = self.make_repo()
        self.git(root, "init", "-b", "main")
        self.git(root, "config", "user.name", "Hermes")
        self.git(root, "config", "user.email", "hermes@example.com")

        (root / "README.md").write_text("base\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: base")
        base = self.git_stdout(root, "rev-parse", "HEAD")

        self.git(root, "checkout", "-b", "topic")
        (root / "topic.md").write_text("topic\n", encoding="utf-8")
        self.git(root, "add", "topic.md")
        self.git(root, "commit", "-m", "feat: topic work")

        self.git(root, "checkout", "main")
        self.git(
            root,
            "merge",
            "--no-ff",
            "-m",
            "Merge swarm-1: parallel agent work",
            "topic",
        )

        subjects = conventional_commits.read_commit_subjects(root, f"{base}..HEAD")
        self.assertEqual(
            subjects,
            ["feat: topic work"],
            "merge commit subject must be skipped; only the feat subject should remain",
        )

    def test_resolve_push_range_uses_before_after_when_ancestor(self) -> None:
        root = self.make_repo()
        self.git(root, "init")
        self.git(root, "config", "user.name", "Hermes")
        self.git(root, "config", "user.email", "hermes@example.com")

        (root / "README.md").write_text("one\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: first")
        before = self.git_stdout(root, "rev-parse", "HEAD")

        (root / "README.md").write_text("two\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "feat: second")
        after = self.git_stdout(root, "rev-parse", "HEAD")

        rev_range, note = conventional_commits.resolve_push_range(root, before, after)
        self.assertEqual(rev_range, f"{before}..{after}")
        self.assertIsNone(note)

    def test_resolve_push_range_falls_back_when_before_missing(self) -> None:
        root = self.make_repo()
        self.git(root, "init")
        self.git(root, "config", "user.name", "Hermes")
        self.git(root, "config", "user.email", "hermes@example.com")

        (root / "README.md").write_text("one\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: first")
        after = self.git_stdout(root, "rev-parse", "HEAD")

        missing_before = "dcd7f495a913fa5dbe36f3311dc9bb175c6acd49"
        rev_range, note = conventional_commits.resolve_push_range(root, missing_before, after)
        self.assertEqual(rev_range, f"{after}^!")
        self.assertIsNotNone(note)
        self.assertIn("likely force-push", note)

    def test_resolve_push_range_falls_back_when_before_is_not_ancestor(self) -> None:
        root = self.make_repo()
        self.git(root, "init")
        self.git(root, "config", "user.name", "Hermes")
        self.git(root, "config", "user.email", "hermes@example.com")

        (root / "README.md").write_text("one\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: base")
        base = self.git_stdout(root, "rev-parse", "HEAD")

        self.git(root, "checkout", "-b", "topic")
        (root / "README.md").write_text("topic\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "feat: topic")
        before = self.git_stdout(root, "rev-parse", "HEAD")

        self.git(root, "checkout", "master")
        (root / "README.md").write_text("main\n", encoding="utf-8")
        self.git(root, "add", "README.md")
        self.git(root, "commit", "-m", "fix: mainline")
        after = self.git_stdout(root, "rev-parse", "HEAD")

        self.assertNotEqual(base, before)
        rev_range, note = conventional_commits.resolve_push_range(root, before, after)
        self.assertEqual(rev_range, f"{after}^!")
        self.assertIsNotNone(note)
        self.assertIn("not an ancestor", note)

    def make_repo(self) -> Path:
        temp_root = Path(tempfile.gettempdir()) / "conventional-commits-tests"
        temp_root.mkdir(parents=True, exist_ok=True)
        root = temp_root / f"repo-{uuid.uuid4().hex}"
        root.mkdir()
        return root

    def git(self, root: Path, *args: str) -> None:
        subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True)

    def git_stdout(self, root: Path, *args: str) -> str:
        return subprocess.run(
            ["git", *args],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()


if __name__ == "__main__":
    unittest.main()
