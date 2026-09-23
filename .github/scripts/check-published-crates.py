#!/usr/bin/env python3
"""Documents that list the published crates must name every one of them.

## The failure this exists to prevent

`contextgraph-trace` was published in the 0.1.2 release, and `SECURITY.md`
kept telling reporters the affected-crate list was types, host and conformance
(#143). A researcher who found a bug in the trace crate read a policy that did
not mention it. The manifests are the only list of published crates that
cannot go stale — `publish = true` is what `cargo publish` obeys — so the prose
copies are checked against them rather than against each other.

## What this checks

The set of workspace crates whose `Cargo.toml` sets `publish = true`, read
fresh on every run, and for each document in `DOCUMENTS`: every crate in that
set is named in it. A crate named in the document but not published is not
flagged — prose may legitimately mention an unpublished crate — the defect this
guards is the omission.

`DOCUMENTS` is where a new copy of the list goes when one is written.
`PUBLISHING.md` is a known third copy that still describes the pre-publish,
three-crate world as a whole; it joins this list with the rewrite that brings
it up to date, rather than being held to one sentence of it now.

Standard library only, offline. Unit tests live in
`.github/scripts/tests/test_check_published_crates.py`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Documents that promise a complete list of the published crates.
DOCUMENTS = ["SECURITY.md"]

PUBLISH_TRUE = re.compile(r"^\s*publish\s*=\s*true\s*(?:#.*)?$", re.MULTILINE)
PACKAGE_NAME = re.compile(r"^\[package\](?:(?!^\[).)*?^name\s*=\s*\"([^\"]+)\"", re.MULTILINE | re.DOTALL)


def published_crates(root: Path) -> set[str]:
    """Names of the workspace crates whose manifest sets `publish = true`."""
    names = set()
    for manifest in sorted(root.glob("*/Cargo.toml")):
        text = manifest.read_text(encoding="utf-8")
        name = PACKAGE_NAME.search(text)
        if name and PUBLISH_TRUE.search(text):
            names.add(name.group(1))
    return names


def missing(crates: set[str], text: str) -> list[str]:
    """Crates from `crates` that `text` never names as a whole word."""
    return sorted(
        crate for crate in crates if not re.search(rf"(?<![\w-]){re.escape(crate)}(?![\w-])", text)
    )


def main() -> int:
    crates = published_crates(ROOT)
    if not crates:
        print("::error::check-published-crates: no crate sets `publish = true`; "
              "the manifest scan is broken", file=sys.stderr)
        return 1
    problems = []
    for document in DOCUMENTS:
        for crate in missing(crates, (ROOT / document).read_text(encoding="utf-8")):
            problems.append(
                f"{document} does not name `{crate}`, which sets `publish = true` in "
                f"{crate}/Cargo.toml. Name it, or say in the document why it is excluded."
            )
    if problems:
        for problem in problems:
            print(f"::error::check-published-crates: {problem}", file=sys.stderr)
        return 1
    print(
        f"check-published-crates: OK — {', '.join(DOCUMENTS)}: names all "
        f"{len(crates)} published crate(s): {', '.join(sorted(crates))}."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
