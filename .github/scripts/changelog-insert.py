#!/usr/bin/env python3
"""
Insert drafted changelog entries under the first `## [Unreleased]` heading.

Usage:
    python3 .github/scripts/changelog-insert.py CHANGELOG.md ENTRIES_FILE

Called from .github/workflows/changelog.yml with the bullets
.github/scripts/changelog-ai.sh drafted. Stdlib only, offline.

Exits 0 once the entries sit directly under the FIRST line that reads
exactly `## [Unreleased]` (trailing whitespace allowed), above whatever the
section already holds. Exits 1, naming the heading, when the file has no such
line or the entries file is empty.

Why this is a script and not the `perl -pi -e` one-liner it replaced (#149):

  `perl -p` runs its body once per line, so the one-liner appended the block
  after EVERY matching heading. CHANGELOG.md carried a second, empty
  `## [Unreleased]` deep in the released history, and every bot pull request
  wrote its bullets twice — once where they belong and once inside the 1.0.0
  notes. Anchoring to the first match fixes the insertion whatever the file
  holds; a later stray heading is reported as a warning, not written to.

  The one-liner's caller also treated "nothing was inserted" as success, so a
  renamed or removed heading kept the workflow green while it drafted
  nothing. That is not the outside service failing, which the workflow does
  degrade open on; it is this repository's file no longer having the shape
  its own tool depends on, and it fails here, loudly.

Tests: .github/scripts/tests/test_changelog_insert.py.
"""
import re
import sys
from pathlib import Path

HEADING = re.compile(r"^## \[Unreleased\][ \t]*$")


def insert(changelog: str, entries: str) -> tuple[str, int]:
    """Return `changelog` with `entries` under its first `## [Unreleased]`.

    The second value is how many matching headings the file has; 0 means
    nothing was inserted.
    """
    lines = changelog.splitlines(keepends=True)
    hits = [i for i, line in enumerate(lines) if HEADING.match(line.rstrip("\r\n"))]
    if not hits:
        return changelog, 0
    first = hits[0]
    block = "\n" + entries.rstrip() + "\n"
    heading = lines[first]
    if not heading.endswith("\n"):
        heading += "\n"
    lines[first] = heading + block
    return "".join(lines), len(hits)


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__.strip().split("\n\n")[1], file=sys.stderr)
        return 2
    changelog_path, entries_path = Path(argv[1]), Path(argv[2])
    entries = entries_path.read_text(encoding="utf-8")
    if not entries.strip():
        print(f"::error::{entries_path} is empty; there is nothing to insert.")
        return 1
    text = changelog_path.read_text(encoding="utf-8")
    updated, headings = insert(text, entries)
    if headings == 0:
        print(
            f"::error file={changelog_path}::{changelog_path} has no `## [Unreleased]`"
            " heading, so the drafted entries have nowhere to go. Restore the"
            " heading (exactly `## [Unreleased]`) and rerun this workflow."
        )
        return 1
    if headings > 1:
        print(
            f"::warning file={changelog_path}::{changelog_path} has {headings}"
            " `## [Unreleased]` headings; entries went under the first only."
            " Remove the others."
        )
    changelog_path.write_text(updated, encoding="utf-8")
    print(f"inserted {entries_path} under the first `## [Unreleased]` of {changelog_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
