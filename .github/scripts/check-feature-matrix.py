#!/usr/bin/env python3
"""Every feature `contextgraph-types` declares must be built by the CI matrix.

## The failure this exists to prevent

A test file gated at file scope with `#![cfg(feature = "...")]` compiles to
*zero tests* when the feature is off, and cargo reports that as
`test result: ok. 0 passed`. A green line, for a suite that ran nothing.

Both cross-language vector files are shaped that way, and both exist to pin wire
format bytes — they are the loudest thing that should fail if a preimage rule
changes. Reporting success when they were not built is the exact opposite of
their job (#117).

PR #114 added the `features` CI job that builds each combination by name, which
fixed the files that existed then. It could not fix the next one: **a new gated
test file, or a new feature, added without a matching line in that job is
silently inert, and nothing says so.** The job's coverage was a thing somebody
had to remember rather than a thing anything checked.

This script makes it checkable. It reads the crate's `[features]` table and the
`features` job's commands, and fails when the job does not name every feature.

## Why parsed from both sides rather than a hardcoded list

A hardcoded list here would be a third place to forget. The crate's manifest is
the source of truth for what features exist; the workflow is the source of truth
for what CI builds; this compares them and owns neither.

Standard library only, and no TOML parser: the `[features]` table is a flat list
of `name = [...]` lines, and depending on a package that may not be installed on
a runner would make this guard the thing that fails.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "contextgraph-types" / "Cargo.toml"
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"

# `default` is not a feature to build on its own — it is the empty set here, and
# the ordinary `test` job already covers a default-features build.
NOT_A_BUILDABLE_FEATURE = {"default"}


def declared_features(manifest_text: str) -> set[str]:
    """Feature names from the crate's `[features]` table."""
    section = re.search(
        r"^\[features\]\s*$(.*?)(?=^\[|\Z)",
        manifest_text,
        re.MULTILINE | re.DOTALL,
    )
    if section is None:
        sys.exit(f"check-feature-matrix: no [features] table in {MANIFEST}")
    names = set()
    for line in section.group(1).splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        match = re.match(r"^([A-Za-z0-9_-]+)\s*=", stripped)
        if match:
            names.add(match.group(1))
    return names - NOT_A_BUILDABLE_FEATURE


def features_built_by_ci(workflow_text: str) -> set[str]:
    """Feature names the `features` job passes to cargo.

    Scoped to that job rather than the whole file, so a `--features` elsewhere
    (a different crate, a different job) cannot make this look covered.
    """
    job = re.search(
        r"^  features:\s*$(.*?)(?=^  [A-Za-z0-9_-]+:\s*$)",
        workflow_text,
        re.MULTILINE | re.DOTALL,
    )
    if job is None:
        sys.exit(
            "check-feature-matrix: no `features:` job in .github/workflows/ci.yml. "
            "That job is what builds the gated vector suites; without it they "
            "compile nowhere and report zero tests as success (#117)."
        )
    return set(re.findall(r"--features\s+([A-Za-z0-9_,-]+)", job.group(1)))


def main() -> int:
    declared = declared_features(MANIFEST.read_text())
    built = set()
    for value in features_built_by_ci(WORKFLOW.read_text()):
        built.update(part for part in value.split(",") if part)

    missing = sorted(declared - built)
    unknown = sorted(built - declared)
    problems: list[str] = []

    if missing:
        problems.append(
            "these features are declared by contextgraph-types but built by no "
            "line of the `features` CI job:\n"
            + "\n".join(f"    {name}" for name in missing)
            + "\n\n  Add `- run: cargo test -p contextgraph-types --features "
            "<name>` to that job. A gated test file for an unbuilt feature "
            "compiles to zero tests and reports `ok. 0 passed`."
        )

    if unknown:
        problems.append(
            "the `features` CI job builds features the crate does not declare:\n"
            + "\n".join(f"    {name}" for name in unknown)
            + "\n\n  cargo errors on an unknown feature, so this job is failing "
            "or the manifest lost a feature it should still have."
        )

    if problems:
        for problem in problems:
            print(f"::error::check-feature-matrix: {problem}", file=sys.stderr)
        print(f"\n{len(problems)} problem(s).", file=sys.stderr)
        return 1

    print(
        "check-feature-matrix: OK — the features job builds all "
        f"{len(declared)} declared feature(s): {', '.join(sorted(declared))}."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
