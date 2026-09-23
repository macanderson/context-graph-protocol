#!/usr/bin/env python3
"""The GUIDE's ADR decision log must index exactly the ADRs in `docs/adr/`.

## The failure this exists to prevent

`docs/GUIDE.md` §3 promises "every ADR, indexed and summarized". Its table is a
shared cell: every PR that adds an ADR must also add a row, each author's tree
is individually fine, and the omission only exists relative to a directory
listing nobody compares it against. It had already happened three times —
0009, 0010 and 0011 were Accepted with no row (#129, #135). Separately, the
anchor line the GUIDE's in-page `#adr-NNNN` links pointed at was deleted in an
unrelated edit, and every one of those links silently went nowhere.

The same directory has a second unguarded invariant: two PRs can each add an
ADR with the same number under different filenames, and git merges both
cleanly (#123). ADR numbers were being allocated by hand to avoid it.

## What this checks

Derived from `docs/adr/*.md` every run, never from a list kept here:

1. Every file in `docs/adr/` is named `NNNN-<slug>.md`.
2. No two ADR files share a number.
3. The GUIDE's §3 table has exactly one row per ADR, in both directions: an ADR
   with no row fails, and a row whose link names no file fails. A row's label
   must be its file's number, and rows run in ascending order.
4. Every in-page `](#adr-NNNN)` link in the GUIDE has a matching
   `id="adr-NNNN"` anchor.

Standard library only, offline. Unit tests live in
`.github/scripts/tests/test_check_adr_log.py`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ADR_DIR = ROOT / "docs" / "adr"
GUIDE = ROOT / "docs" / "GUIDE.md"

ADR_NAME = re.compile(r"^(\d{4})-[a-z0-9][a-z0-9-]*\.md$")
SECTION = re.compile(r"^## 3\. Decision log \(ADRs\)\s*$(.*?)(?=^## |\Z)", re.MULTILINE | re.DOTALL)
ROW = re.compile(r"^\|\s*\[(?P<label>[^\]]+)\]\(\./adr/(?P<target>[^)#\s]+)\)\s*\|", re.MULTILINE)
IN_PAGE_LINK = re.compile(r"\]\(#(adr-\d{4})\)")
ANCHOR = re.compile(r"""\bid=["'](adr-\d{4})["']""")


def check(adr_files: list[str], guide_text: str) -> list[str]:
    """Every problem found, as one human-readable line each.

    `adr_files` is the list of file names in `docs/adr/` (names only);
    `guide_text` is the full text of `docs/GUIDE.md`.
    """
    problems: list[str] = []

    # 1 and 2 — the directory is well-formed.
    by_number: dict[str, list[str]] = {}
    for name in sorted(adr_files):
        match = ADR_NAME.match(name)
        if match is None:
            problems.append(
                f"docs/adr/{name} is not named NNNN-<slug>.md (lowercase slug), so "
                "it cannot be indexed or number-checked"
            )
            continue
        by_number.setdefault(match.group(1), []).append(name)
    for number, names in sorted(by_number.items()):
        if len(names) > 1:
            problems.append(
                f"ADR number {number} is used by {len(names)} files: {', '.join(names)}. "
                "Renumber all but one — the later-merged PR takes the next free number."
            )

    # 3 — the table indexes exactly the directory.
    section = SECTION.search(guide_text)
    if section is None:
        problems.append(
            "docs/GUIDE.md has no `## 3. Decision log (ADRs)` section; this guard "
            "cannot find the table it holds to docs/adr/"
        )
        return problems
    rows = [(m.group("label"), m.group("target")) for m in ROW.finditer(section.group(1))]
    known = {name for names in by_number.values() for name in names}
    rowed: dict[str, int] = {}
    previous = ""
    for label, target in rows:
        if target not in known:
            problems.append(
                f"docs/GUIDE.md §3 row [{label}] links ./adr/{target}, which is not an ADR in docs/adr/"
            )
            continue
        number = target[:4]
        if label != number:
            problems.append(
                f"docs/GUIDE.md §3 row [{label}] links ./adr/{target}; its label must be {number}"
            )
        rowed[target] = rowed.get(target, 0) + 1
        if number < previous:
            problems.append(
                f"docs/GUIDE.md §3 row {number} comes after row {previous}; keep the rows in ADR order"
            )
        previous = max(previous, number)
    for name in sorted(known):
        count = rowed.get(name, 0)
        if count == 0:
            problems.append(
                f"docs/adr/{name} has no row in docs/GUIDE.md §3 — add one: "
                f"| [{name[:4]}](./adr/{name}) | <title> | <one plain-language line: what changed and why a reader should care> |"
            )
        elif count > 1:
            problems.append(f"docs/adr/{name} has {count} rows in docs/GUIDE.md §3; keep one")

    # 4 — in-page ADR links land somewhere.
    anchors = set(ANCHOR.findall(guide_text))
    for target in sorted(set(IN_PAGE_LINK.findall(guide_text)) - anchors):
        problems.append(
            f"docs/GUIDE.md links #{target} but has no id=\"{target}\" anchor; link the "
            "ADR file directly (./adr/NNNN-<slug>.md) instead"
        )

    return problems


def main() -> int:
    adr_files = sorted(p.name for p in ADR_DIR.iterdir() if p.is_file() and p.suffix == ".md")
    problems = check(adr_files, GUIDE.read_text(encoding="utf-8"))
    if problems:
        for problem in problems:
            print(f"::error::check-adr-log: {problem}", file=sys.stderr)
        print(f"\n{len(problems)} problem(s).", file=sys.stderr)
        return 1
    print(
        f"check-adr-log: OK — {len(adr_files)} ADR(s), unique numbers, one GUIDE §3 row each."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
