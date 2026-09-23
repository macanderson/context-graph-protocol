#!/usr/bin/env python3
"""
Hold every published SDK package to the crates' homepage and repository.

Usage:
    python3 .github/scripts/check-sdk-metadata.py

Exits 0 if every package manifest under `sdk/*/` declares the same homepage,
repository and issue tracker as `Cargo.toml`'s `[workspace.package]`, and the
Go module path lives in that repository. Exits 1 otherwise. Stdlib only,
offline.

Why this exists (#112):

  The SDKs were published with almost no project metadata. Both npm packages
  had no `homepage` and no `repository`, so their registry pages linked
  nowhere, `npm repo <pkg>` failed, and npm had no source link to show beside
  a provenance statement. The Python package's only URL was the GitHub repo,
  while the crates named the protocol domain as their homepage. That gave one
  project two answers to "where does this live", each picked by whoever wrote
  the file.

  Fixing the three manifests once would not stop the next SDK from starting
  with nothing again, which is how this happened. So manifests are found by
  walking `sdk/*/`, not listed here. A new SDK directory is held to this rule
  as soon as it has a manifest.

The reference values are read from Cargo.toml, never restated here. The crates
were published first and their pair is what ADR 0008 settled. This check
compares every SDK against it and owns no value of its own.

What each ecosystem must say:

  * npm (`package.json`): `homepage`; `repository` in object form, with its
    `url` naming the repository and `directory` naming the package's own
    subpath, so the registry deep-links into the monorepo; `bugs.url` naming
    the issue tracker. A `"private": true` package publishes nothing and is
    skipped.
  * PyPI (`pyproject.toml`): `[project.urls]` `Homepage`, `Repository` and
    `Issues`, the labels PyPI renders with their own icons.
  * Go (`go.mod`): a module takes its identity from its path, so there is no
    metadata field to fill in. The path must be the repository's path plus
    the module's subdirectory, or `go get` resolves somewhere else.

Scaffolder templates under `sdk/*/templates/` are other people's projects, not
this repository's packages, and a one-level glob never reaches them.
"""
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CARGO = ROOT / "Cargo.toml"
SDK_DIR = ROOT / "sdk"

failures = 0


def check(label: str, ok: bool, detail: str = "") -> None:
    global failures
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures += 1
        for line in detail.splitlines():
            print(f"        {line}")


def normalise_git_url(url: str) -> str:
    """`git+https://host/o/r.git` -> `https://host/o/r`, the form Cargo.toml uses."""
    url = url.strip()
    url = url.removeprefix("git+")
    url = url.removesuffix("/")
    return url.removesuffix(".git")


print("published SDK packages carry the crates' homepage and repository")

workspace = tomllib.loads(CARGO.read_text(encoding="utf-8"))["workspace"]["package"]
HOMEPAGE = workspace.get("homepage", "")
REPOSITORY = workspace.get("repository", "")
ISSUES = f"{REPOSITORY}/issues"
check(
    "Cargo.toml [workspace.package] states a homepage and a repository",
    bool(HOMEPAGE) and bool(REPOSITORY),
    f"homepage={HOMEPAGE!r} repository={REPOSITORY!r}; these are the reference values.",
)
if failures:
    sys.exit(1)

npm_manifests = sorted(SDK_DIR.glob("*/package.json"))
py_manifests = sorted(SDK_DIR.glob("*/pyproject.toml"))
go_manifests = sorted(SDK_DIR.glob("*/go.mod"))
check(
    "sdk/ holds package manifests this check can read",
    bool(npm_manifests or py_manifests or go_manifests),
    "no sdk/*/package.json, sdk/*/pyproject.toml or sdk/*/go.mod found.\n"
    "Did the SDKs move? Point SDK_DIR at their new home.",
)

for path in npm_manifests:
    rel = path.relative_to(ROOT).as_posix()
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("private") is True:
        print(f"  SKIP  {rel} is private and publishes nothing")
        continue
    subdir = path.parent.relative_to(ROOT).as_posix()

    check(f"{rel}: homepage is {HOMEPAGE}", manifest.get("homepage") == HOMEPAGE,
          f"found {manifest.get('homepage')!r}")

    repo = manifest.get("repository")
    repo_ok = (isinstance(repo, dict)
               and normalise_git_url(str(repo.get("url", ""))) == REPOSITORY
               and repo.get("directory") == subdir)
    check(
        f"{rel}: repository names {REPOSITORY} at directory {subdir}",
        repo_ok,
        f"found {repo!r}\n"
        f'remedy: "repository": {{"type": "git", "url": "git+{REPOSITORY}.git",'
        f' "directory": "{subdir}"}}',
    )

    bugs = manifest.get("bugs")
    bugs_url = bugs.get("url") if isinstance(bugs, dict) else bugs
    check(f"{rel}: bugs names {ISSUES}", bugs_url == ISSUES, f"found {bugs!r}")

for path in py_manifests:
    rel = path.relative_to(ROOT).as_posix()
    urls = tomllib.loads(path.read_text(encoding="utf-8")).get("project", {}).get("urls", {})
    for label, expected in (("Homepage", HOMEPAGE), ("Repository", REPOSITORY),
                            ("Issues", ISSUES)):
        check(f"{rel}: [project.urls] {label} is {expected}", urls.get(label) == expected,
              f"found {urls.get(label)!r}")

for path in go_manifests:
    rel = path.relative_to(ROOT).as_posix()
    module = next((line.split(None, 1)[1].strip()
                   for line in path.read_text(encoding="utf-8").splitlines()
                   if line.startswith("module ")), "")
    expected = (REPOSITORY.removeprefix("https://") + "/"
                + path.parent.relative_to(ROOT).as_posix())
    check(f"{rel}: module path is {expected}", module == expected, f"found {module!r}")

print(f"\n{'OK — SDK metadata matches the crates' if failures == 0 else f'{failures} failure(s)'}")
sys.exit(1 if failures else 0)
