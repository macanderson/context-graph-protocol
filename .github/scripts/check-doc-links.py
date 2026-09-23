#!/usr/bin/env python3
"""Every `docs/...` path this repository names must resolve to a tracked file.

## The failure this exists to prevent

PR #80 deleted `docs/sketches/` as stale and repointed most of what cited it.
Ten citations survived, in Rust doc comments, `MIGRATION.md`, and the
`description` of a *published* crate — so crates.io, `cargo search`, and a
docs.rs intra-doc link all sent readers to a directory that no longer existed
(#153). Nothing failed, because nothing reads a path out of prose:
`check-deploy-hygiene.py` checks absolute artifact URLs, and no build step
dereferences a repo-relative path in a comment.

This script is that missing reader. It scans the tree's prose for `docs/`
paths and fails naming each one that does not exist.

## What counts as a reference

- A bare repo-relative path: `docs/stability.md`, `docs/adr/`.
- A relative link that reaches `docs/`: `./docs/adr/0004-...md` from the root,
  `../../docs/...` from a crate's `src/`. A path that starts with `./` or
  `../` is resolved against the *citing file's* directory, so a link with the
  wrong number of `../` is caught too, not just a link to a missing file.
- An absolute link into this repository's own tree:
  `https://github.com/macanderson/context-graph-protocol/blob/<ref>/docs/...`.
  A `docs/` path inside any *other* URL is not ours to check and is skipped.

A `#fragment` is stripped: this checks that the file exists, not that the
anchor does. A path running into a glob or placeholder character (`*`, `<`,
`{`, `$`) is a pattern, not a reference, and is skipped.

## Where it looks

Markdown, TOML, YAML, shell and Python are read whole: in those, a `docs/` path
is prose or configuration a reader acts on. In Rust, TypeScript, Go and
JavaScript only comment lines are read — that is where the doc comments and
the cited design live, while string literals there are test data (a path-shape
classifier's inputs, a fictional provider's citation labels) that name no file
by design. For the same reason test sources in any language (a `tests/`
directory, `tests.rs`, `*_test.go`, `*.test.ts`) are not read at all: their
comments and strings describe fixture data. JSON is data and is not read.

`CHANGELOG.md` is history. An entry that says "removed `docs/sketches/`" is
true and must stay true; it cannot be held to today's tree. This script's own
source is not read either, so an example in this docstring is never a finding.

## Paths in other repositories

Some prose deliberately names a `docs/` path in a *different* repository —
a downstream's design notes, the retired standing-decisions directory. Those
are listed in `FOREIGN` with the reason each is allowed. The list is held to
two rules so it cannot rot into a silencer: an entry that matches a path this
tree actually has is an error (it is not foreign, so check it), and an entry
nothing cites any more is an error (delete it).

Standard library only, and offline. The guard is itself guarded:
`.github/scripts/tests/test_check_doc_links.py` runs it on scratch trees with
known answers.
"""

from __future__ import annotations

import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Read every line.
WHOLE_FILE_SUFFIXES = {".md", ".toml", ".yml", ".yaml", ".sh", ".py"}
# Read comment lines only (see "Where it looks" above).
COMMENT_ONLY_SUFFIXES = {".rs", ".ts", ".go", ".js", ".mjs"}

# CHANGELOG.md is history, not a claim about today's tree (see the module
# docstring). This script's own docstring quotes the dead paths #153 found, as
# examples, so it is kept out too.
UNSCANNED_FILES = {"CHANGELOG.md", ".github/scripts/check-doc-links.py"}

# Path prefixes that name another repository's `docs/`, and why each is
# cited. Keys are matched against the resolved repo-relative path.
FOREIGN = {
    "docs/design/": "stella's design notes, cited by ADR 0007 as the "
    "downstream spec bundle this repository drew its boundary against",
    "docs/specs/": "stella's and oxagen-platform's spec directories, cited "
    "by ADR 0007, MIGRATION.md §4 and the downstream canary",
    "docs/scr/": "the standing-decisions directory oxagen ADR-137 retired; "
    "named only to say this repository no longer carries it",
    "docs/adr/ADR-": "oxagen's ADR naming scheme (this repository numbers "
    "ADRs `NNNN-`), cited by dod-check.yml",
    "docs/retry.md": "a citation label in the example docs provider's "
    "fictional corpus, printed in docs/composition-walkthrough.md",
}

# A `docs/` path, optionally led by `./` or `../` segments. The lookbehind
# refuses a match that starts mid-token — inside a URL (`/docs/`), inside
# another path (`sdk/docs/`), or inside an identifier (`mydocs/`) — so URLs
# are left to ABSOLUTE below and never double-counted here. The path must end
# on a word character or `/`, so sentence punctuation is not swallowed.
RELATIVE = re.compile(r"(?<![\w/.\-])((?:\.\.?/)*docs/(?:[\w\-./]*[\w/])?)")

# This repository's own blob/tree URLs. The captured group is the in-tree path.
ABSOLUTE = re.compile(
    r"https://github\.com/macanderson/context-graph-protocol/(?:blob|tree)/"
    r"[^/\s]+/(docs/(?:[\w\-./]*[\w/])?)"
)

# What may follow a match when the path is really a pattern
# (`docs/adr/*.md`, `docs/adr/NNNN-<slug>.md`, `docs/{a,b}.md`).
PATTERN_TAIL = re.compile(r"[\-.]*[*<{$]")


def tracked_files(root: Path) -> list[str]:
    """Paths git tracks, relative to `root`, POSIX-separated."""
    out = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [path for path in out.split("\0") if path]


def tracked_dirs(files: list[str]) -> set[str]:
    """Every directory that contains at least one tracked file."""
    dirs: set[str] = set()
    for path in files:
        parent = posixpath.dirname(path)
        while parent:
            dirs.add(parent)
            parent = posixpath.dirname(parent)
    return dirs


def is_test_source(path: str) -> bool:
    """A test source: its comments describe fixture data, not documents."""
    parts = path.split("/")
    stem = Path(path).stem
    return (
        any(part in ("tests", "test", "testdata") for part in parts[:-1])
        or stem == "tests"
        or stem.endswith(("_test", ".test"))
    )


def scanned_lines(path: str, text: str) -> list[tuple[int, str]]:
    """`(line number, text)` for the lines of `path` this guard reads."""
    suffix = Path(path).suffix
    if is_test_source(path):
        return []
    lines = list(enumerate(text.splitlines(), start=1))
    if suffix in WHOLE_FILE_SUFFIXES:
        return lines
    if suffix in COMMENT_ONLY_SUFFIXES:
        return [(n, line) for n, line in lines if line.lstrip().startswith(("//", "/*", "*"))]
    return []


def references(citing: str, lines: list[tuple[int, str]]) -> list[tuple[int, str, str]]:
    """`(line, as_written, resolved)` for each `docs/` reference.

    `resolved` is the repo-relative path the reference must name, with any
    `#fragment` removed and any leading `./`/`../` resolved against the
    directory of `citing`.
    """
    found: list[tuple[int, str, str]] = []
    base = posixpath.dirname(citing)
    for number, line in lines:
        for match in ABSOLUTE.finditer(line):
            if PATTERN_TAIL.match(line, match.end(1)):
                continue
            found.append((number, match.group(0), posixpath.normpath(match.group(1))))
        for match in RELATIVE.finditer(line):
            if PATTERN_TAIL.match(line, match.end(1)):
                continue
            written = match.group(1)
            if written.startswith("."):
                resolved = posixpath.normpath(posixpath.join(base, written))
            else:
                resolved = posixpath.normpath(written)
            found.append((number, written, resolved))
    return found


def dangling(root: Path, foreign: dict[str, str] = FOREIGN) -> list[str]:
    """One message per problem: a dangling reference or a rotten FOREIGN entry."""
    files = tracked_files(root)
    exists = set(files) | tracked_dirs(files)
    problems: list[str] = []
    used: set[str] = set()

    for prefix in foreign:
        local = [p for p in exists if p == prefix.rstrip("/") or p.startswith(prefix)]
        if local:
            problems.append(
                f"FOREIGN entry `{prefix}` matches `{sorted(local)[0]}`, which this "
                "tree has — it is not foreign; remove the entry so the path is checked"
            )

    for citing in files:
        if citing in UNSCANNED_FILES:
            continue
        try:
            text = (root / citing).read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError):
            continue
        for number, written, resolved in references(citing, scanned_lines(citing, text)):
            if resolved in exists:
                continue
            prefix = next(
                (p for p in foreign if resolved == p.rstrip("/") or resolved.startswith(p)),
                None,
            )
            if prefix is not None:
                used.add(prefix)
                continue
            problems.append(f"{citing}:{number}: `{written}` names no tracked file")

    for prefix in sorted(set(foreign) - used):
        problems.append(
            f"FOREIGN entry `{prefix}` is cited nowhere any more — delete it so it "
            "cannot silence a future dangling path"
        )
    return problems


def main() -> int:
    problems = dangling(ROOT)
    if problems:
        for problem in problems:
            print(f"::error::check-doc-links: {problem}", file=sys.stderr)
        print(
            f"\n{len(problems)} problem(s). Point each dangling path at a document "
            "that exists, write the document, or drop the pointer: a path that "
            "reads as a document a reader failed to find is worse than no "
            "pointer at all (#153). A path that genuinely names another "
            "repository's docs/ belongs in FOREIGN, with its reason.",
            file=sys.stderr,
        )
        return 1
    print("check-doc-links: OK — every docs/ path named in the tree resolves.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
