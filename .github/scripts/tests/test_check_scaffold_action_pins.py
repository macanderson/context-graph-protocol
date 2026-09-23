"""Self-tests for .github/scripts/check-scaffold-action-pins.py.

Run: python3 -m unittest discover -s .github/scripts/tests -v

Each test writes a throwaway repository tree (repository workflows, a
composite action, and one scaffold template) and runs the guard against it
with `--root`, exactly as CI runs it against this checkout. A guard is only
evidence if it goes red on the drift it names, so most cases here are the
red ones: a quoted pin, a multi-word `#` comment, an unparsed `uses:` line,
a template major the repository no longer runs, and a repository split
across majors (#207, #208, #209).
"""
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check-scaffold-action-pins.py"
SHA = "0123456789abcdef0123456789abcdef01234567"


def workflow(*steps: str) -> str:
    body = "".join(f"      {step}\n" for step in steps)
    return (
        "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n" + body
    )


class ScaffoldActionPins(unittest.TestCase):
    def run_guard(
        self,
        template: str,
        repo: str,
        action_yml: str | None = None,
    ) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".github" / "workflows").mkdir(parents=True)
            (root / ".github" / "workflows" / "ci.yml").write_text(repo)
            if action_yml is not None:
                action = root / ".github" / "actions" / "setup"
                action.mkdir(parents=True)
                (action / "action.yml").write_text(action_yml)
            tdir = (
                root
                / "sdk/create-contextgraph-provider/templates/ts/_github/workflows"
            )
            tdir.mkdir(parents=True)
            (tdir / "conformance.yml").write_text(template)
            return subprocess.run(
                [sys.executable, str(SCRIPT), "--root", str(root)],
                capture_output=True,
                text=True,
            )

    def assertGreen(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("FAIL", result.stdout)

    def assertRed(self, result: subprocess.CompletedProcess, needle: str) -> None:
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(needle, result.stdout)

    # --- green: the shapes the parser must accept ------------------------

    def test_matching_major_passes(self):
        self.assertGreen(
            self.run_guard(
                workflow("- uses: actions/checkout@v7"),
                workflow("- uses: actions/checkout@v7"),
            )
        )

    def test_quoted_values_parse(self):
        self.assertGreen(
            self.run_guard(
                workflow('- uses: "actions/checkout@v7"'),
                workflow("- uses: 'actions/checkout@v7.1.0'"),
            )
        )

    def test_sha_pin_reads_major_from_first_comment_word(self):
        self.assertGreen(
            self.run_guard(
                workflow(f"- uses: actions/checkout@{SHA} # v7.0.0 pinned for supply chain"),
                workflow("- uses: actions/checkout@v7"),
            )
        )

    def test_action_subpath_parses(self):
        self.assertGreen(
            self.run_guard(
                workflow("- uses: github/codeql-action/init@v3"),
                workflow("- uses: github/codeql-action/init@v3"),
            )
        )

    def test_composite_action_counts_as_repository_side(self):
        composite = (
            "runs:\n  using: composite\n  steps:\n"
            "    - uses: actions/setup-node@v7\n"
        )
        self.assertGreen(
            self.run_guard(
                workflow("- uses: actions/setup-node@v7"),
                workflow("- uses: ./.github/actions/setup"),
                action_yml=composite,
            )
        )

    def test_skipped_shapes_do_not_fail(self):
        result = self.run_guard(
            workflow(
                "- uses: ./local-action",
                "- uses: docker://alpine:3.20",
                "- uses: dtolnay/rust-toolchain@stable",
                "- uses: someone/only-in-template@v1",
            ),
            workflow("- uses: actions/checkout@v7"),
        )
        self.assertGreen(result)
        self.assertIn("names no major", result.stdout)
        self.assertIn("this repository does not use it", result.stdout)

    def test_reusable_workflow_is_skipped(self):
        self.assertGreen(
            self.run_guard(
                "on: push\njobs:\n  call:\n"
                "    uses: org/repo/.github/workflows/x.yml@v1\n",
                workflow("- uses: actions/checkout@v7"),
            )
        )

    # --- red: every way the guard used to pass for the wrong reason ------

    def test_quoted_pin_on_stale_major_fails(self):
        self.assertRed(
            self.run_guard(
                workflow('- uses: "actions/setup-node@v4"'),
                workflow("- uses: actions/setup-node@v7"),
            ),
            "remedy: move",
        )

    def test_multi_word_comment_on_stale_major_fails(self):
        self.assertRed(
            self.run_guard(
                workflow(f"- uses: actions/checkout@{SHA} # v5.0.0 kept for now"),
                workflow("- uses: actions/checkout@v7"),
            ),
            "repository pins v7",
        )

    def test_template_major_the_repository_no_longer_runs_fails(self):
        self.assertRed(
            self.run_guard(
                workflow("- uses: actions/setup-python@v5"),
                workflow("- uses: actions/setup-python@v7"),
            ),
            "remedy: move",
        )

    def test_unparsed_template_uses_line_fails(self):
        self.assertRed(
            self.run_guard(
                workflow("- uses: actions/checkout"),
                workflow("- uses: actions/checkout@v7"),
            ),
            "has a `uses:` value this guard can parse",
        )

    def test_mismatched_quotes_fail_to_parse(self):
        self.assertRed(
            self.run_guard(
                workflow("- uses: \"actions/checkout@v7'"),
                workflow("- uses: actions/checkout@v7"),
            ),
            "has a `uses:` value this guard can parse",
        )

    def test_repository_split_across_majors_fails(self):
        composite = (
            "runs:\n  using: composite\n  steps:\n"
            "    - uses: actions/setup-node@v4\n"
        )
        self.assertRed(
            self.run_guard(
                workflow("- uses: actions/setup-node@v7"),
                workflow("- uses: actions/setup-node@v7"),
                action_yml=composite,
            ),
            "more than one major",
        )

    def test_no_template_workflow_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--root", tmp],
                capture_output=True,
                text=True,
            )
        self.assertRed(result, "ships at least one workflow")


if __name__ == "__main__":
    unittest.main()
