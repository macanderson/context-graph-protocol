"""Self-tests for .github/scripts/changelog-insert.py (#149).

Run: python3 -m unittest discover -s .github/scripts/tests -v

The script replaced a `perl -p` one-liner that wrote the drafted entries
under every `## [Unreleased]` heading, and a caller that stayed green when
the heading was gone. These tests hold both: a two-heading changelog gets
one copy, under the first heading, and a changelog with no heading fails.
"""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "changelog-insert.py"

ENTRIES = "### Fixed\n- **A drafted entry** (#1) — test.\n"

TWO_HEADINGS = """# Changelog

## [Unreleased]

### Added
- **Existing entry** (#0) — already here.

## [0.1.2]

- old notes

## [Unreleased]

## [1.0.0] — 2026-08-11
"""


class ChangelogInsert(unittest.TestCase):
    def run_insert(self, changelog: str, entries: str = ENTRIES):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "CHANGELOG.md"
            path.write_text(changelog, encoding="utf-8")
            drafted = Path(tmp) / "entries.md"
            drafted.write_text(entries, encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(path), str(drafted)],
                capture_output=True,
                text=True,
            )
            return result, path.read_text(encoding="utf-8")

    def test_two_headings_get_one_copy_under_the_first(self):
        result, text = self.run_insert(TWO_HEADINGS)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(text.count("A drafted entry"), 1)
        self.assertLess(text.index("A drafted entry"), text.index("## [0.1.2]"))
        self.assertIn("::warning", result.stdout)

    def test_entries_go_directly_under_the_heading_above_existing_ones(self):
        result, text = self.run_insert(TWO_HEADINGS)
        self.assertEqual(result.returncode, 0)
        self.assertIn(
            "## [Unreleased]\n\n### Fixed\n- **A drafted entry** (#1) — test.\n\n"
            "### Added\n- **Existing entry**",
            text,
        )

    def test_one_heading_inserts_without_a_warning(self):
        result, text = self.run_insert("# Changelog\n\n## [Unreleased]\n\n## [1.0.0]\n")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(text.count("A drafted entry"), 1)
        self.assertNotIn("::warning", result.stdout)

    def test_renamed_heading_fails_and_leaves_the_file_alone(self):
        original = "# Changelog\n\n## [Unreleased] (main)\n\n## [1.0.0]\n"
        result, text = self.run_insert(original)
        self.assertEqual(result.returncode, 1)
        self.assertIn("no `## [Unreleased]`", result.stdout)
        self.assertEqual(text, original)

    def test_missing_heading_fails(self):
        result, _ = self.run_insert("# Changelog\n\n## [1.0.0]\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("::error", result.stdout)

    def test_empty_entries_fail(self):
        result, text = self.run_insert(TWO_HEADINGS, entries="  \n")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(text, TWO_HEADINGS)

    def test_the_repository_changelog_takes_exactly_one_copy(self):
        changelog = Path(__file__).resolve().parents[3] / "CHANGELOG.md"
        result, text = self.run_insert(changelog.read_text(encoding="utf-8"))
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(text.count("A drafted entry"), 1)


if __name__ == "__main__":
    unittest.main()
