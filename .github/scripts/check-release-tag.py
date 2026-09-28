#!/usr/bin/env python3
"""Refuse to publish a crate version that no tag on `main` names.

Usage:
    python3 .github/scripts/check-release-tag.py --tag TAG --commit SHA \\
        [--main REF] [--root DIR] [--skip-registry]

`release.yml`'s `preflight` job runs this before anything else, with the tag
that triggered the run and the commit it points at. Exits 0 and prints the
version to release when all three rules below hold, and exits 1 naming every
rule that does not. When `GITHUB_OUTPUT` is set, it also writes
`version=<version>` there, so the publish job releases the version this script
checked rather than re-deriving it from the tag name.

## The failure this exists to prevent (#103, ADR 0034)

Every crate version before 2.0.0 was published with no git tag naming it.
0.1.0, 0.1.1 and 0.1.2 all went up by hand, and the only record of which
commit each came from is the `.cargo_vcs_info.json` that `cargo publish`
writes into the `.crate` tarball. None of those three commits is on `main`. Meanwhile three tag conventions were in play (`ocp-v*`, a
`v0.0.2` that `MIGRATION.md` named and nobody cut, and the `contextgraph-v*`
this workflow triggers on), so nothing could say which tag a release should
have had.

ADR 0034 fixes the convention. This script makes the workflow unable to break
it: a release happens only from a tag whose name is the version it publishes,
on a commit that went through review on `main`.

## The rules

1. **The tag names the version.** Every workspace crate whose manifest sets
   `publish = true` must carry one version (the crates release in lockstep),
   and the tag must be exactly `contextgraph-v<that version>`. A tag pushed
   with a typo, or pushed before the version bump merged, stops here instead
   of publishing whatever `Cargo.toml` says under a name that says otherwise.
2. **The tagged commit is on `main`.** `--main` (default `origin/main`) must
   contain `--commit`. `main` takes changes only through a reviewed pull
   request, so this is what ties a published crate to a reviewed tree. A tag
   on a feature branch, or on a commit that was never pushed anywhere else,
   stops here.
3. **The version is not already on crates.io.** crates.io rejects a second
   upload of a version, but only at the upload, after a human has approved
   the `crates-io` environment. Checking the sparse index first turns that
   into a preflight failure, so a tag re-pushed for a version already live
   asks nobody to approve anything. (The retroactive tags ADR 0034 cuts for
   0.1.0 to 0.1.2 never reach this script: a tag push runs the workflow file
   in the tagged commit, and those commits predate it. PUBLISHING.md says
   what to do with those runs.) `--skip-registry` turns this rule off, for
   the self-tests; the workflow never passes it.

The registry rule fails closed. An index that cannot be read proves nothing,
so a network error is a failure, not a pass. Re-run the job.

Standard library plus git. Rule 3 needs the network; rules 1 and 2 do not.
Unit tests live in `.github/scripts/tests/test_check_release_tag.py`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]

# The one workspace tag prefix (ADR 0034). `release.yml` triggers on
# `contextgraph-v*`; this is the same string, and the two must not drift.
TAG_PREFIX = "contextgraph-v"

# Semantic Versioning 2.0.0, as Cargo accepts it. Checked before the version is
# spliced into a tag name or an index lookup, so neither sees anything else.
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)

INDEX = "https://index.crates.io"

# (status, body) for a URL. `None` status means the request itself failed.
Fetch = Callable[[str], tuple[int | None, str]]


def published_crate_versions(root: Path) -> dict[str, str]:
    """Each workspace member with `publish = true`, mapped to its version.

    Reads the manifests rather than running `cargo metadata`, so the check
    needs no toolchain and the self-tests need no Rust project. A member
    version of `version.workspace = true` resolves to the root's
    `[workspace.package] version`, as Cargo resolves it.
    """
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    workspace_version = workspace.get("workspace", {}).get("package", {}).get("version")
    versions: dict[str, str] = {}
    for member in workspace.get("workspace", {}).get("members", []):
        manifest = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))
        package = manifest.get("package", {})
        if package.get("publish") is not True:
            continue
        version = package.get("version")
        if isinstance(version, dict) and version.get("workspace") is True:
            version = workspace_version
        if not isinstance(version, str):
            raise SystemExit(f"check-release-tag: cannot read a version for {member}/Cargo.toml")
        versions[package["name"]] = version
    return versions


def tag_problems(tag: str, versions: dict[str, str]) -> list[str]:
    """Rule 1: the published crates share one version, and `tag` names it."""
    if not versions:
        return ["no workspace crate sets `publish = true`; the manifest scan is broken"]
    distinct = sorted(set(versions.values()))
    if len(distinct) > 1:
        listing = ", ".join(f"{name} {version}" for name, version in sorted(versions.items()))
        return [f"the published crates must release in lockstep, but carry {len(distinct)} versions: {listing}"]
    version = distinct[0]
    if not SEMVER.match(version):
        return [f"the workspace version `{version}` is not a semantic version"]
    expected = f"{TAG_PREFIX}{version}"
    if tag != expected:
        return [
            f"tag `{tag}` does not name the version this commit would publish. "
            f"The publishing crates are at {version}, so the tag must be `{expected}` (ADR 0034). "
            "Delete the tag, then tag the commit where the version bump merged."
        ]
    return []


def on_main_problems(root: Path, commit: str, main: str) -> list[str]:
    """Rule 2: `commit` is `main` or one of its ancestors."""
    result = subprocess.run(
        ["git", "-C", str(root), "merge-base", "--is-ancestor", commit, main],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        return []
    if result.returncode == 1:
        return [
            f"commit {commit} is not on `{main}`. A release is cut only from a commit that "
            "merged to main through review (ADR 0034). Delete the tag and re-tag the merge commit."
        ]
    return [f"could not compare {commit} with `{main}`: {result.stderr.strip() or 'git failed'}"]


def index_path(crate: str) -> str:
    """The crate's file under the sparse index, per Cargo's layout rule."""
    name = crate.lower()
    if len(name) == 1:
        return f"1/{name}"
    if len(name) == 2:
        return f"2/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[0:2]}/{name[2:4]}/{name}"


def fetch_url(url: str) -> tuple[int | None, str]:
    """GET `url`, three attempts, returning the status and body."""
    last: tuple[int | None, str] = (None, "")
    for _ in range(3):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "context-graph-protocol release preflight"})
            with urllib.request.urlopen(request, timeout=20) as response:
                return response.status, response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return 404, ""
            last = (error.code, "")
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            last = (None, str(error))
    return last


def published_versions(body: str) -> set[str]:
    """The versions a sparse-index file lists, yanked or not."""
    versions = set()
    for line in body.splitlines():
        line = line.strip()
        if line:
            versions.add(json.loads(line).get("vers"))
    return versions


def registry_problems(versions: dict[str, str], fetch: Fetch = fetch_url) -> list[str]:
    """Rule 3: no publishing crate already has its version on crates.io."""
    problems = []
    for crate, version in sorted(versions.items()):
        url = f"{INDEX}/{index_path(crate)}"
        status, body = fetch(url)
        if status == 404:
            continue  # never published: a first release of this crate
        if status != 200:
            problems.append(
                f"could not read {url} (status {status or 'none'}{': ' + body if body else ''}); "
                "an unreadable index proves nothing, so this fails closed. Re-run the job."
            )
            continue
        if version in published_versions(body):
            problems.append(
                f"{crate} {version} is already on crates.io. A version is published once; "
                "this tag records a release that already happened and starts nothing. "
                "If a run failed partway, finish the remaining crates by hand (PUBLISHING.md)."
            )
    return problems


def main(argv: list[str] | None = None, fetch: Fetch = fetch_url) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", required=True, help="the tag name that triggered the release")
    parser.add_argument("--commit", required=True, help="the commit that tag points at")
    parser.add_argument("--main", default="origin/main", help="the ref the commit must be on")
    parser.add_argument("--root", type=Path, default=ROOT, help="the workspace checkout")
    parser.add_argument("--skip-registry", action="store_true", help="do not read crates.io (self-tests only)")
    args = parser.parse_args(argv)

    versions = published_crate_versions(args.root)
    problems = tag_problems(args.tag, versions)
    problems += on_main_problems(args.root, args.commit, args.main)
    if not problems and not args.skip_registry:
        problems += registry_problems(versions, fetch)

    if problems:
        for problem in problems:
            print(f"::error::check-release-tag: {problem}", file=sys.stderr)
        return 1

    version = next(iter(versions.values()))
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"version={version}\n")
    crates = ", ".join(sorted(versions))
    print(f"check-release-tag: `{args.tag}` on `{args.main}` releases {version} of {crates}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
