#!/usr/bin/env python3
"""
Hold every ADR to one number, and every number to one ADR.

Usage:
    python3 .github/scripts/check-adr-numbers.py                  # this tree
    python3 .github/scripts/check-adr-numbers.py --against REF    # the merge
                                                                  # of REF and HEAD

Exits 0 if the checked tree's `docs/adr/` holds no two ADRs with one number
and every ADR is indexed in `docs/GUIDE.md`'s decision log. Exits 1 otherwise.
Stdlib only; `--against` needs git 2.38 or newer for `merge-tree --write-tree`.

Why this exists (#123):

  An ADR number is an address: prose, code comments and other ADRs cite
  "ADR 0012", so two documents holding one number make every citation
  ambiguous, permanently and silently. It has happened here: #106 and #109
  (and then #114) each added a different `0012-*.md`. The filenames differ,
  so git merges both cleanly, and each branch is individually valid, so
  every check was green on both. Each author correctly read the highest
  number and correctly picked the next one; the information needed to avoid
  the collision existed on no single branch.

  So the check that matters is against the merge result, not the branch.
  `--against origin/main` computes the tree HEAD would produce if merged into
  the base as it is *when the check runs* — not as it was when the pull
  request's merge ref was made — and checks that. A branch that was green
  before a colliding ADR landed on the base goes red on its next run. CI
  also runs the plain form on every push to main, so a collision that slips
  past a stale green check fails the merge commit that created it, naming
  both files, rather than surfacing months later in a citation.

The rules:

  1. Every Markdown file in `docs/adr/` (bar a README.md) is named
     `NNNN-slug.md`: four digits, a hyphen, a lowercase slug.
  2. No two ADRs share a number. Gaps are fine: a number reserved and then
     abandoned, or claimed by a pull request still open, costs nothing.
  3. An ADR whose first heading carries a number carries its own filename's
     number, so the document cannot claim an address the tree gave to
     another. A heading with no number is not checked.
  4. Every ADR is linked from the table in `docs/GUIDE.md`'s "Decision log"
     section, and every ADR link in that table resolves. The log is where a
     reader finds decisions; an ADR it omits is one nobody is told about,
     and a row pointing at a renamed file is a dead end.

Why sequential numbers are guarded rather than replaced:

  Date- or slug-addressed ADRs would remove the shared cell instead of
  guarding it, and #123 asked for that to be rejected explicitly if not
  taken. It is not taken, because the numbers are already the address: the
  GUIDE, SPEC, code comments, CHANGELOG entries, other ADRs and downstream
  repositories cite "ADR 0012", and every one of those citations is
  permanent. Renaming would break them or require a second, aliasing
  address system forever. A collision, by contrast, is cheap to fix while
  its pull request is still open (renumber one file), and this guard makes
  sure that is when it is found.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable, Optional

ROOT = Path(__file__).resolve().parent.parent.parent
ADR_DIR = "docs/adr"
GUIDE = "docs/GUIDE.md"
NAME = re.compile(r"^(\d{4})-[a-z0-9][a-z0-9-]*\.md$")
# `# ADR 0002 — …`, `# 0005 — …`, `# ADR-0020: …`, `# 18. …`
HEADING_NUMBER = re.compile(r"^#\s+(?:ADR[\s-]*)?(\d+)\b", re.IGNORECASE)
LOG_HEADING = re.compile(r"^##\s+.*Decision log", re.IGNORECASE)
LOG_LINK = re.compile(r"\]\(\./adr/([^)#\s]+)")


def check(names: list[str], read: Callable[[str], Optional[str]]) -> list[str]:
    """Return every violation in a tree whose `docs/adr/` holds `names`.

    `read(path)` returns a repository-relative file's text, or None if the
    tree has no such file.
    """
    errors: list[str] = []
    by_number: dict[int, list[str]] = {}

    for name in sorted(names):
        if not name.endswith(".md") or name.lower() == "readme.md":
            continue
        found = NAME.match(name)
        if not found:
            errors.append(
                f"{ADR_DIR}/{name}: an ADR is named NNNN-slug.md "
                "(four digits, a hyphen, a lowercase slug)"
            )
            continue
        number = int(found[1])
        by_number.setdefault(number, []).append(name)

        text = read(f"{ADR_DIR}/{name}") or ""
        heading = next((line for line in text.splitlines() if line.startswith("# ")), "")
        claimed = HEADING_NUMBER.match(heading)
        if claimed and int(claimed[1]) != number:
            errors.append(
                f"{ADR_DIR}/{name}: its heading claims ADR {int(claimed[1]):04d}, "
                f"but its filename is ADR {number:04d}"
            )

    for number, holders in sorted(by_number.items()):
        if len(holders) > 1:
            errors.append(
                f"ADR {number:04d} is claimed by {len(holders)} files: "
                + ", ".join(f"{ADR_DIR}/{holder}" for holder in holders)
                + " — renumber the one whose pull request landed second"
            )

    guide = read(GUIDE)
    if guide is None:
        errors.append(f"{GUIDE} is missing, so no ADR is indexed")
        return errors
    log: list[str] = []
    in_log = False
    for line in guide.splitlines():
        if line.startswith("## "):
            in_log = bool(LOG_HEADING.match(line))
            continue
        if in_log:
            log.append(line)
    if not log:
        errors.append(f"{GUIDE} has no '## … Decision log' section to index ADRs in")
        return errors
    linked = {target for line in log for target in LOG_LINK.findall(line)}
    adrs = {name for holders in by_number.values() for name in holders}
    for name in sorted(adrs - linked):
        errors.append(f"{ADR_DIR}/{name} has no row in {GUIDE}'s decision log")
    for target in sorted(linked - set(names)):
        errors.append(f"{GUIDE}'s decision log links ./adr/{target}, which does not exist")
    return errors


def git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=False
    )


def merged_tree(root: Path, against: str) -> str:
    """The tree `git merge` of HEAD into `against` would produce.

    Content conflicts elsewhere do not stop the check: merge-tree still writes
    a tree (with conflict markers in the conflicted files), and ADR names are
    what is being checked. Exit status 1 means "conflicts", anything else
    above 0 is a real failure.
    """
    result = git(root, "merge-tree", "--write-tree", against, "HEAD")
    if result.returncode not in (0, 1) or not result.stdout.strip():
        sys.exit(
            f"check-adr-numbers: git merge-tree --write-tree {against} HEAD failed "
            f"(needs git >= 2.38 and history back to the merge base): {result.stderr.strip()}"
        )
    return result.stdout.split()[0]


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--against",
        metavar="REF",
        help="check the merge of HEAD into REF (e.g. origin/main) instead of the working tree",
    )
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    root: Path = args.root

    if args.against:
        tree = merged_tree(root, args.against)
        listing = git(root, "ls-tree", "--name-only", f"{tree}:{ADR_DIR}")
        names = listing.stdout.split() if listing.returncode == 0 else []

        def read(path: str) -> Optional[str]:
            shown = git(root, "cat-file", "blob", f"{tree}:{path}")
            return shown.stdout if shown.returncode == 0 else None

        where = f"the merge of HEAD into {args.against}"
    else:
        directory = root / ADR_DIR
        names = [entry.name for entry in directory.iterdir()] if directory.is_dir() else []

        def read(path: str) -> Optional[str]:
            file = root / path
            return file.read_text(encoding="utf-8") if file.is_file() else None

        where = "the working tree"

    errors = check(names, read)
    if errors:
        print(f"check-adr-numbers: {len(errors)} problem(s) in {where}:")
        for error in errors:
            print(f"  - {error}")
        return 1
    count = sum(1 for name in names if NAME.match(name))
    print(f"check-adr-numbers: OK — {count} ADR(s) in {where}, one number each, all indexed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
