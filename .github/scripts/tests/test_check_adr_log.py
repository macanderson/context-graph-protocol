"""Unit tests for `.github/scripts/check-adr-log.py` (#129).

`check()` is pure — a directory listing and the GUIDE's text in, problems out —
so each case states its inputs inline. Running it against the real tree is
CI's `adr-log` job, not a unit test, so a missing row fails in one place.
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-adr-log.py"
_spec = importlib.util.spec_from_file_location("check_adr_log", SCRIPT)
assert _spec is not None and _spec.loader is not None
check_adr_log = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_adr_log)


def guide(*rows: str, extra: str = "") -> str:
    """A minimal GUIDE with a §3 table holding `rows` (`NNNN-slug.md` names,
    or full row strings starting with `|`)."""
    lines = [
        "# Guide",
        extra,
        "## 3. Decision log (ADRs)",
        "",
        "| # | Title | One-line takeaway |",
        "|---|---|---|",
    ]
    for row in rows:
        lines.append(row if row.startswith("|") else f"| [{row[:4]}](./adr/{row}) | T | L |")
    lines += ["", "## Where to go next", "", "| [9999](./adr/9999-outside.md) | not in §3 | x |"]
    return "\n".join(lines) + "\n"


FILES = ["0002-a.md", "0003-b.md"]


class CheckAdrLog(unittest.TestCase):
    def assertProblems(self, files: list[str], text: str, *needles: str) -> None:
        problems = check_adr_log.check(files, text)
        self.assertEqual(len(problems), len(needles), problems)
        for problem, needle in zip(problems, needles):
            self.assertIn(needle, problem)

    def test_a_table_matching_the_directory_passes(self) -> None:
        self.assertProblems(FILES, guide(*FILES))

    def test_an_adr_with_no_row_fails(self) -> None:
        self.assertProblems(FILES + ["0099-x.md"], guide(*FILES), "0099-x.md has no row")

    def test_a_deleted_row_fails_naming_the_adr(self) -> None:
        self.assertProblems(FILES, guide("0002-a.md"), "0003-b.md has no row")

    def test_a_row_naming_a_missing_file_fails(self) -> None:
        self.assertProblems(
            FILES, guide(*FILES, "0004-gone.md"), "links ./adr/0004-gone.md, which is not an ADR"
        )

    def test_two_files_with_one_number_fail(self) -> None:
        files = FILES + ["0003-other.md"]
        self.assertProblems(
            files, guide("0002-a.md", "0003-b.md", "0003-other.md"), "ADR number 0003 is used by 2 files"
        )

    def test_a_misnamed_file_fails(self) -> None:
        self.assertProblems(FILES + ["ADR-7-thing.md"], guide(*FILES), "is not named NNNN-<slug>.md")

    def test_a_label_that_disagrees_with_its_file_fails(self) -> None:
        rows = ["0002-a.md", "| [0004](./adr/0003-b.md) | T | L |"]
        self.assertProblems(FILES, guide(*rows), "its label must be 0003")

    def test_rows_out_of_order_fail(self) -> None:
        self.assertProblems(FILES, guide("0003-b.md", "0002-a.md"), "keep the rows in ADR order")

    def test_a_duplicated_row_fails(self) -> None:
        self.assertProblems(FILES, guide("0002-a.md", "0003-b.md", "0003-b.md"), "has 2 rows")

    def test_a_missing_section_fails(self) -> None:
        self.assertProblems(FILES, "# Guide\n", "no `## 3. Decision log (ADRs)` section")

    def test_an_in_page_link_with_no_anchor_fails(self) -> None:
        self.assertProblems(FILES, guide(*FILES, extra="See [ADR 2](#adr-0002)."), "no id=\"adr-0002\" anchor")

    def test_an_in_page_link_with_its_anchor_passes(self) -> None:
        text = guide(*FILES, extra='See [ADR 2](#adr-0002). <a id="adr-0002"></a>')
        self.assertProblems(FILES, text)


if __name__ == "__main__":
    unittest.main()
