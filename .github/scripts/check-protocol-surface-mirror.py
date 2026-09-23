#!/usr/bin/env python3
"""
Hold docs/protocol-surface.md's conformance index to SPEC.md's requirement ids.

Usage:
    python3 .github/scripts/check-protocol-surface-mirror.py

Exits 0 if every numbered requirement in SPEC.md has a row in the
`## Conformance requirements` section of docs/protocol-surface.md, that section
names no id SPEC.md does not define, no id appears twice on either side, and
every mirror row says what enforces or verifies it. Exits 1 otherwise. Stdlib
only, offline.

Why this exists (#158):

  GOVERNANCE.md names that section as the mirror of SPEC.md's requirement
  tables. When #158 was filed, SPEC.md stated 57 numbered requirements and the
  mirror restated 23. The 34 it left out included every requirement the
  conformance suite gained a check for after the mirror was written. A provider
  author who read only that page never learned about the `kinds` filter (Q1),
  the frame cap (B4), or the forward-compatibility rules (U1-U4), and the suite
  fails a provider on each of them. Nothing noticed, because nothing compared
  the two files.

What is compared, and what deliberately is not:

  * **Id sets, not counts.** A count in prose is a promise nobody keeps. Every
    SPEC.md id must have a mirror row, and every mirror id must exist in
    SPEC.md. A mirror row naming a requirement the spec does not define is as
    misleading as a missing one.
  * **Presence, not wording.** The mirror is a summary on purpose, and several
    rows put a SPEC row in plainer words. Comparing the texts would fail on
    every one of those rewrites. The spec's header already settles
    disagreements: SPEC.md wins.
  * **Any id family.** An id is one or more capital letters followed by digits
    (`H1`, `F16`, `UR1`). Nothing here lists the families, so a requirement
    added under a new letter is checked the day it lands.

A requirement row is a Markdown table row whose first cell is an id, bold
(`| **H1** |`, SPEC.md's style) or plain (`| H1 |`, the mirror's). Both files
accept both shapes, so restyling a table cannot make this gate go blind. If
either side yields no ids at all, that is a failure, not a pass: an empty set
means the parser lost its subject.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = ROOT / "SPEC.md"
MIRROR = ROOT / "docs" / "protocol-surface.md"
MIRROR_SECTION = "## Conformance requirements"

# `| **F16** | …` or `| F16 | …`. The id is captured. The rest of the row is
# split into cells separately, so a `|` inside a later cell cannot affect it.
REQUIREMENT_ROW = re.compile(r"^\|\s*(?:\*\*)?([A-Z]+[0-9]+)(?:\*\*)?\s*\|")

failures = 0


def check(label: str, ok: bool, detail: list[str] | None = None) -> None:
    global failures
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures += 1
        for line in detail or []:
            print(f"        {line}")


def requirement_rows(lines: list[str], first_line: int = 1) -> list[tuple[str, int, list[str]]]:
    """Each requirement row as (id, line number, cells after the id)."""
    rows = []
    for offset, line in enumerate(lines):
        match = REQUIREMENT_ROW.match(line)
        if match is None:
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")][1:]
        rows.append((match[1], first_line + offset, cells))
    return rows


def section(lines: list[str], heading: str) -> tuple[list[str], int] | None:
    """The lines under `heading`, up to the next heading of the same level."""
    try:
        start = next(i for i, line in enumerate(lines) if line.rstrip() == heading)
    except StopIteration:
        return None
    level = heading.split(" ", 1)[0]
    end = next(
        (i for i in range(start + 1, len(lines))
         if lines[i].startswith(level + " ")),
        len(lines),
    )
    return lines[start + 1:end], start + 2


def duplicates(rows: list[tuple[str, int, list[str]]]) -> list[str]:
    seen: dict[str, list[int]] = {}
    for rid, lineno, _ in rows:
        seen.setdefault(rid, []).append(lineno)
    return [f"{rid} on lines {', '.join(map(str, nums))}"
            for rid, nums in sorted(seen.items()) if len(nums) > 1]


def sort_key(rid: str) -> tuple[str, int]:
    letters = rid.rstrip("0123456789")
    return letters, int(rid[len(letters):])


print("docs/protocol-surface.md mirrors every requirement SPEC.md states")

spec_rows = requirement_rows(SPEC.read_text(encoding="utf-8").splitlines())
check("SPEC.md states numbered requirements this gate can read", bool(spec_rows), [
    "no `| **X1** |` or `| X1 |` table rows found in SPEC.md.",
    "Did the requirement tables change shape? Teach REQUIREMENT_ROW the new one.",
])

found = section(MIRROR.read_text(encoding="utf-8").splitlines(), MIRROR_SECTION)
check(f"docs/protocol-surface.md has a '{MIRROR_SECTION}' section", found is not None, [
    "GOVERNANCE.md names that section as SPEC.md's mirror. If it moved, move",
    "MIRROR_SECTION here with it.",
])
mirror_rows = requirement_rows(*found) if found else []
if found:
    check("that section holds requirement rows this gate can read", bool(mirror_rows), [
        f"no `| X1 |` table rows under '{MIRROR_SECTION}'.",
    ])

if not failures:
    spec_ids = {rid for rid, _, _ in spec_rows}
    mirror_ids = {rid for rid, _, _ in mirror_rows}

    missing = sorted(spec_ids - mirror_ids, key=sort_key)
    check("every SPEC.md requirement has a mirror row", not missing, [
        f"missing: {' '.join(missing)}",
        f"remedy: add a row for each under docs/protocol-surface.md's '{MIRROR_SECTION}',",
        "summarising the requirement and naming what enforces or verifies it.",
    ])

    invented = sorted(mirror_ids - spec_ids, key=sort_key)
    check("the mirror names no requirement SPEC.md does not define", not invented, [
        f"not in SPEC.md: {' '.join(invented)}",
        "remedy: renumber the row to the SPEC.md id it restates, or delete it.",
        "SPEC.md is normative; the mirror only indexes it.",
    ])

    check("no requirement id is stated twice in SPEC.md", not duplicates(spec_rows),
          duplicates(spec_rows))
    check("no requirement id has two mirror rows", not duplicates(mirror_rows),
          duplicates(mirror_rows))

    # The issue's bar is "each naming what enforces or verifies it": a row with
    # no such cell hands the reader a rule and no way to know it is checked.
    unenforced = [f"{rid} (line {lineno})" for rid, lineno, cells in mirror_rows
                  if len(cells) < 2 or not cells[-1]]
    check("every mirror row names what enforces or verifies it", not unenforced, [
        f"no 'Enforced / verified by' cell: {', '.join(unenforced)}",
    ])

    if not failures:
        print(f"\nOK — all {len(spec_ids)} SPEC.md requirements are mirrored")
        sys.exit(0)

print(f"\n{failures} failure(s)")
sys.exit(1)
