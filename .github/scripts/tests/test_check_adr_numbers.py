"""Self-test for check-adr-numbers.py (#123).

The case that matters is the one a branch-local check cannot see: two
branches off one base, each adding a differently named ADR with the same
number. The guard must pass the second branch before the first lands (nothing
was wrong then) and fail it after (the merge result now holds two ADRs with
one number). `test_second_branch_goes_red_once_the_first_lands` is that case,
with real git, not a mock.
"""
from __future__ import annotations

import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check-adr-numbers.py"
spec = importlib.util.spec_from_file_location("check_adr_numbers", SCRIPT)
assert spec and spec.loader
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

GUIDE_HEAD = "# Guide\n\n## 3. Decision log (ADRs)\n\n| # | Title |\n|---|---|\n"
GUIDE_TAIL = "\n## Where to go next\n\n[stray](./adr/not-indexed-here.md)\n"


def guide_for(*names: str) -> str:
    rows = "".join(f"| [{n[:4]}](./adr/{n}) | {n} |\n" for n in names)
    return GUIDE_HEAD + rows + GUIDE_TAIL


def adr(number: str, title: str = "A decision") -> str:
    return f"# {number} — {title}\n\n**Status:** Accepted\n"


class TreeTest(unittest.TestCase):
    """The working-tree form, over a scratch repository layout."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        (self.root / "docs" / "adr").mkdir(parents=True)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def write(self, files: dict[str, str], indexed: list[str] | None = None) -> None:
        for name, text in files.items():
            (self.root / "docs" / "adr" / name).write_text(text, encoding="utf-8")
        listed = sorted(files) if indexed is None else indexed
        (self.root / "docs" / "GUIDE.md").write_text(guide_for(*listed), encoding="utf-8")

    def run_guard(self) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out):
            code = guard.main(["--root", str(self.root)])
        return code, out.getvalue()

    def test_unique_numbers_with_gaps_pass(self) -> None:
        self.write(
            {
                "0002-first.md": adr("0002"),
                "0005-second.md": adr("ADR 0005"),
                "0020-later.md": "# 20. Numbered the other way\n",
                "0021-untitled.md": "# A heading with no number\n",
            }
        )
        code, out = self.run_guard()
        self.assertEqual(code, 0, out)

    def test_two_files_with_one_number_fail_naming_both(self) -> None:
        self.write({"0012-sdk-pins.md": adr("0012"), "0012-schema-id.md": adr("0012")})
        code, out = self.run_guard()
        self.assertEqual(code, 1)
        self.assertIn("ADR 0012 is claimed by 2 files", out)
        self.assertIn("0012-sdk-pins.md", out)
        self.assertIn("0012-schema-id.md", out)

    def test_heading_claiming_another_number_fails(self) -> None:
        self.write({"0013-schema.md": adr("0012")})
        code, out = self.run_guard()
        self.assertEqual(code, 1)
        self.assertIn("heading claims ADR 0012", out)

    def test_misnamed_adr_fails(self) -> None:
        self.write({"12-schema.md": adr("0012")}, indexed=[])
        code, out = self.run_guard()
        self.assertEqual(code, 1)
        self.assertIn("NNNN-slug.md", out)

    def test_adr_missing_from_the_decision_log_fails(self) -> None:
        self.write({"0002-a.md": adr("0002"), "0003-b.md": adr("0003")}, indexed=["0002-a.md"])
        code, out = self.run_guard()
        self.assertEqual(code, 1)
        self.assertIn("0003-b.md has no row", out)

    def test_decision_log_row_pointing_nowhere_fails(self) -> None:
        self.write({"0002-a.md": adr("0002")}, indexed=["0002-a.md", "0003-renamed.md"])
        code, out = self.run_guard()
        self.assertEqual(code, 1)
        self.assertIn("links ./adr/0003-renamed.md, which does not exist", out)

    def test_links_outside_the_decision_log_are_not_the_index(self) -> None:
        # GUIDE_TAIL links a file that does not exist from another section;
        # only the decision log is the index.
        self.write({"0002-a.md": adr("0002")})
        code, out = self.run_guard()
        self.assertEqual(code, 0, out)


class MergeResultTest(unittest.TestCase):
    """The `--against` form, with real branches and a real merge."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        self.git("init", "-q", "-b", "main")
        self.commit({"docs/adr/0001-base.md": adr("0001")}, "base", indexed=["0001-base.md"])

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(self.root), *args],
            env=self.env,
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    def commit(self, files: dict[str, str], message: str, indexed: list[str]) -> None:
        for path, text in files.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
        (self.root / "docs" / "GUIDE.md").write_text(guide_for(*indexed), encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def against_main(self) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out):
            code = guard.main(["--root", str(self.root), "--against", "main"])
        return code, out.getvalue()

    def test_second_branch_goes_red_once_the_first_lands(self) -> None:
        self.git("checkout", "-q", "-b", "a", "main")
        self.commit(
            {"docs/adr/0099-from-a.md": adr("0099")},
            "a adds 0099",
            indexed=["0001-base.md", "0099-from-a.md"],
        )
        self.git("checkout", "-q", "-b", "b", "main")
        self.commit(
            {"docs/adr/0099-from-b.md": adr("0099")},
            "b adds 0099",
            indexed=["0001-base.md", "0099-from-b.md"],
        )

        # Before a lands, b is fine: nothing on main collides with it.
        code, out = self.against_main()
        self.assertEqual(code, 0, out)

        # a lands. b's branch is byte-for-byte unchanged and still valid on
        # its own, so a branch-local check would stay green here.
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "-m", "merge a", "a")
        self.git("checkout", "-q", "b")
        with redirect_stdout(io.StringIO()):
            self.assertEqual(guard.main(["--root", str(self.root)]), 0)

        # The merge result is what is checked, and it now holds two 0099s.
        code, out = self.against_main()
        self.assertEqual(code, 1, out)
        self.assertIn("ADR 0099 is claimed by 2 files", out)
        self.assertIn("0099-from-a.md", out)
        self.assertIn("0099-from-b.md", out)

    def test_distinct_numbers_across_branches_pass_after_merge(self) -> None:
        self.git("checkout", "-q", "-b", "a", "main")
        self.commit(
            {"docs/adr/0020-from-a.md": adr("0020")},
            "a adds 0020",
            indexed=["0001-base.md", "0020-from-a.md"],
        )
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "-m", "merge a", "a")
        self.git("checkout", "-q", "-b", "b", "main")
        self.commit(
            {"docs/adr/0027-from-b.md": adr("0027")},
            "b adds 0027",
            indexed=["0001-base.md", "0020-from-a.md", "0027-from-b.md"],
        )
        code, out = self.against_main()
        self.assertEqual(code, 0, out)


if __name__ == "__main__":
    unittest.main()
