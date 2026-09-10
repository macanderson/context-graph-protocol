#!/usr/bin/env python3
"""Documentation that states a conformance-check count must state the real one,
and must not name a check that does not exist.

## Why this is a guard and not a one-time correction

The count has drifted twice that we know of. #152 was filed when six documents
said "five checks" and one said "seven" while the suite ran thirteen — and by
the time it was fixed the suite ran **fourteen**, because `attestation` had
landed in between. A number that changes whenever somebody adds a check will go
stale again the next time somebody adds a check.

The same issue found `docs/protocol-surface.md` citing a `usage-report`
conformance check that has never existed in any of the three constant families.
A "Verified by" cell naming a check nobody wrote is worse than an empty cell: it
reads as evidence.

So this reads the constants and checks the prose against them, rather than
anybody remembering to.

## What it checks

1. Every count stated in prose ("13 checks", "the five checks") matches one of
   the real family sizes.
2. Every check name cited in prose exists among the declared constants.

Standard library only: this runs in CI on a container that may have nothing
installed, and a guard whose dependency is missing is a guard that does not run.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "contextgraph-conformance" / "src"

FAMILIES = {
    "provider": (SRC / "lib.rs", r'^pub const CHECK_[A-Z0-9_]+: &str = "([a-z0-9-]+)"'),
    "host": (
        SRC / "host_conformance.rs",
        r'^pub const HCHECK_[A-Z0-9_]+: &str = "([a-z0-9-]+)"',
    ),
    "composition": (
        SRC / "composition_conformance.rs",
        r'^pub const CCHECK_[A-Z0-9_]+: &str = "([a-z0-9-]+)"',
    ),
}

# Written-out numbers a document might use for a count.
WORD_NUMBERS = {
    "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8,
    "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13,
    "fourteen": 14, "fifteen": 15, "sixteen": 16,
}

DOC_GLOBS = ("*.md",)
SKIP = {
    # An append-only record of what was true at each release. Correcting a past
    # entry would falsify the history the file exists to keep.
    "CHANGELOG.md",
}


def declared() -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for family, (path, pattern) in FAMILIES.items():
        if not path.exists():
            sys.exit(f"check-conformance-counts: {path} not found")
        names = re.findall(pattern, path.read_text(), re.MULTILINE)
        if not names:
            sys.exit(
                f"check-conformance-counts: no constants matched in {path}. "
                "The declaration style changed and this guard now checks nothing "
                "— fix the pattern rather than deleting the check."
            )
        out[family] = names
    return out


def docs() -> list[Path]:
    found: list[Path] = []
    for glob in DOC_GLOBS:
        for path in ROOT.rglob(glob):
            rel = path.relative_to(ROOT)
            if rel.name in SKIP:
                continue
            if any(part in {"target", "node_modules", ".git"} for part in rel.parts):
                continue
            found.append(path)
    return sorted(found)


COUNT_RE = re.compile(
    r"\b(\d+|" + "|".join(WORD_NUMBERS) + r")\s+(?:conformance\s+)?checks\b",
    re.IGNORECASE,
)
# `the five checks`, as a heading or a phrase.
THE_N_CHECKS_RE = re.compile(
    r"\bthe\s+(\d+|" + "|".join(WORD_NUMBERS) + r")\s+checks\b", re.IGNORECASE
)


def as_number(token: str) -> int | None:
    if token.isdigit():
        return int(token)
    return WORD_NUMBERS.get(token.lower())


def main() -> int:
    families = declared()
    sizes = {family: len(names) for family, names in families.items()}
    every_name = {name for names in families.values() for name in names}
    valid_counts = set(sizes.values())

    problems: list[str] = []

    for path in docs():
        rel = path.relative_to(ROOT)
        text = path.read_text(encoding="utf8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            for match in list(COUNT_RE.finditer(line)) + list(
                THE_N_CHECKS_RE.finditer(line)
            ):
                value = as_number(match.group(1))
                if value is None or value in valid_counts:
                    continue
                problems.append(
                    f"{rel}:{lineno} claims {match.group(0)!r}, but the suite has "
                    + ", ".join(f"{n} {f}" for f, n in sorted(sizes.items()))
                    + f".\n      {line.strip()[:110]}"
                )

    # Check names cited in prose, in backticks, that look like a check name.
    name_like = re.compile(r"`([a-z][a-z0-9]*(?:-[a-z0-9]+)+)`\s+(?:conformance\s+)?check\b")
    for path in docs():
        rel = path.relative_to(ROOT)
        for lineno, line in enumerate(
            path.read_text(encoding="utf8", errors="replace").splitlines(), start=1
        ):
            for match in name_like.finditer(line):
                cited = match.group(1)
                if cited in every_name:
                    continue
                problems.append(
                    f"{rel}:{lineno} cites a `{cited}` check, which exists in no "
                    "constant family. A 'verified by' cell naming a check nobody "
                    "wrote reads as evidence and is not."
                    f"\n      {line.strip()[:110]}"
                )

    if problems:
        print("check-conformance-counts: FAIL\n", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}\n", file=sys.stderr)
        print(f"{len(problems)} problem(s).", file=sys.stderr)
        return 1

    print(
        "check-conformance-counts: OK — "
        + ", ".join(f"{n} {f} checks" for f, n in sorted(sizes.items()))
        + "; every count and check name in the docs matches."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
