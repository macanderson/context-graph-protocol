#!/usr/bin/env python3
"""
Hold every commit a pull request adds to the Developer Certificate of Origin.

Usage:
    python3 .github/scripts/check-dco.py --range BASE..HEAD

Exits 0 if every non-merge commit in the range carries a `Signed-off-by:`
trailer that certifies it, 1 otherwise. Stdlib plus git; offline.

Why this exists (see docs/adr/0025-the-dco-is-enforced-not-requested.md and
#141):

  CONTRIBUTING.md, GOVERNANCE.md and the pull request template all say a
  contribution is licensed "as certified by your DCO sign-off", and nothing
  checked it. On main at the time #141 was filed, 21 of 81 commits carried a
  sign-off. A licensing rule that is stated as mandatory and not enforced
  reads to an adopter's lawyer as a policy in force that the history does
  not support. ADR 0025 chose to make the rule true rather than delete it;
  this is the check that does.

The rule:

  A commit is certified when one of its `Signed-off-by:` trailers names the
  commit's author email (compared case-insensitively). That is the DCO's
  own shape: the person who wrote the change certifies it.

  A commit authored by a GitHub App (`name[bot]`, or an author email
  ending `[bot]@users.noreply.github.com`) is certified by any
  `Signed-off-by:` trailer. An app cannot certify anything; the trailer it
  carries is the human (or the app's operator, as Dependabot's
  `support@github.com` sign-off is) who does. An app commit with no
  trailer at all still fails.

  Merge commits are skipped: they add no authored change of their own.

  The range is what a pull request adds, so history before this check
  landed is neither checked nor rewritten; ADR 0025 records that cut-over.

To fix a failing pull request:

    git rebase --signoff <base>      # adds your trailer to every commit
    git push --force-with-lease
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import NamedTuple, Optional

ROOT = Path(__file__).resolve().parent.parent.parent
FIELD, RECORD, TRAILER = "\x1f", "\x1e", "\x1d"
FORMAT = (
    f"%H{FIELD}%an{FIELD}%ae{FIELD}%s{FIELD}"
    f"%(trailers:key=Signed-off-by,valueonly,separator=%x1d){RECORD}"
)
EMAIL = re.compile(r"<([^<>\s]+)>\s*$")


class Commit(NamedTuple):
    sha: str
    author: str
    email: str
    subject: str
    signoffs: list[str]


def is_app(commit: Commit) -> bool:
    return commit.author.endswith("[bot]") or commit.email.lower().endswith(
        "[bot]@users.noreply.github.com"
    )


def problem(commit: Commit) -> Optional[str]:
    """Why `commit` is not certified, or None if it is."""
    if is_app(commit):
        if commit.signoffs:
            return None
        return "is authored by a GitHub App and carries no Signed-off-by trailer"
    emails = []
    for value in commit.signoffs:
        found = EMAIL.search(value)
        if found:
            emails.append(found[1].lower())
    if commit.email.lower() in emails:
        return None
    if not commit.signoffs:
        return "has no Signed-off-by trailer"
    return (
        f"is signed off by {', '.join(commit.signoffs)}, "
        f"but authored by <{commit.email}>"
    )


def commits(root: Path, revision_range: str) -> list[Commit]:
    result = subprocess.run(
        ["git", "-C", str(root), "log", "--no-merges", f"--format={FORMAT}", revision_range],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.exit(f"check-dco: git log {revision_range} failed: {result.stderr.strip()}")
    found = []
    for record in result.stdout.split(RECORD):
        record = record.strip("\n")
        if not record:
            continue
        sha, author, email, subject, trailers = record.split(FIELD)
        signoffs = [value.strip() for value in trailers.split(TRAILER) if value.strip()]
        found.append(Commit(sha, author, email, subject, signoffs))
    return found


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--range", required=True, help="BASE..HEAD, the commits a pull request adds")
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    checked = commits(args.root, args.range)
    failures = [(commit, why) for commit in checked if (why := problem(commit))]
    if failures:
        print(f"check-dco: {len(failures)} of {len(checked)} commit(s) are not signed off:")
        for commit, why in failures:
            print(f"  - {commit.sha[:12]} {commit.subject!r} {why}")
        print(
            "\nSign every commit to certify the Developer Certificate of Origin "
            "(CONTRIBUTING.md). To fix this pull request:\n"
            "    git rebase --signoff <base>\n"
            "    git push --force-with-lease"
        )
        return 1
    print(f"check-dco: OK — {len(checked)} commit(s) in {args.range}, each signed off by its author.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
