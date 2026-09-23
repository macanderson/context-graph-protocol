#!/usr/bin/env python3
"""
Hold the scaffold templates' action pins to the majors this repository runs.

Usage:
    python3 .github/scripts/check-scaffold-action-pins.py [--root DIR]

`--root` points the guard at another checkout. Its self-tests
(.github/scripts/tests/test_check_scaffold_action_pins.py) use it to run the
script against fixture trees.

Exits 0 if every `uses: owner/action@ref` in a scaffold template's workflow
names the same major that this repository's own workflows and composite
actions pin for that action. Exits 1 otherwise. Stdlib only, offline.

Why this exists (see docs/adr/0012-sdk-version-pins-share-a-major.md and
#194, which owns this guard):

  `sdk/create-contextgraph-provider` hands every new provider a
  `.github/workflows/conformance.yml`. In the tree that file lives under
  `templates/<lang>/_github/`, and the scaffolder renames `_github` to
  `.github` only at scaffold time. Dependabot scans `.github/workflows` and
  `.github/actions/**/action.yml` and never sees the copy the templates ship.
  #194 moved `actions/setup-python` to v7 in this repository's workflows
  while the Python template stayed on v5, and #198 does the same for
  `actions/checkout`. No job could have noticed: the scaffold job runs the
  conformance script directly and never executes the nested workflow.

  That is the failure mode ADR 0012 records for SDK pins, on a different
  axis. The remedy is the same: an offline guard that fails the commit which
  opens the gap, because that commit is the one that can still close it
  cheaply.

The rule:

  For each action a template pins with a readable major, the repository's
  workflows and composite actions must pin that action on exactly one major,
  and the template's major must equal it. A major is readable from `@v<N>`,
  `@v<N>.<x>.<y>`, or a SHA ref followed by a `# v<N>` comment, which is the
  form Dependabot maintains for SHA pins. Prose after the version in that
  comment is ignored.

  These are skipped and reported, because there is nothing to compare:
  a ref with no major (`@master`, `@stable`), an action only the template
  uses, a local `./` action, a `docker://` image, and a reusable workflow
  (`owner/repo/.github/workflows/x.yml@ref`).

  Any line whose `uses:` value this guard cannot parse fails. A pin the
  parser misses would otherwise pass silently, which is the failure this
  guard exists to prevent.

  Majors, not exact refs, because the repository pins tags (`@v7`) while a
  hardened workflow may pin a SHA with a version comment, and the two are
  the same release line. A major is where an action's inputs and its Node
  runtime change, which is what a scaffolded provider cannot survive.
"""
import argparse
import re
import sys
from pathlib import Path

_args = argparse.ArgumentParser(description=__doc__.split("\n\n")[0].strip())
_args.add_argument(
    "--root",
    type=Path,
    default=Path(__file__).resolve().parent.parent.parent,
    help="repository root to check (default: this checkout)",
)
ROOT = _args.parse_args().root.resolve()
REPO_WORKFLOWS = sorted(
    list((ROOT / ".github" / "workflows").glob("*.yml"))
    + list((ROOT / ".github" / "workflows").glob("*.yaml"))
    + list((ROOT / ".github" / "actions").glob("**/action.yml"))
    + list((ROOT / ".github" / "actions").glob("**/action.yaml"))
)
TEMPLATE_WORKFLOWS = sorted(
    list(
        (ROOT / "sdk" / "create-contextgraph-provider" / "templates").glob(
            "*/_github/workflows/*.yml"
        )
    )
    + list(
        (ROOT / "sdk" / "create-contextgraph-provider" / "templates").glob(
            "*/_github/workflows/*.yaml"
        )
    )
)

# A line that carries a `uses:` key, as a step key or a list item. Every such
# line must parse with USES below, or the guard fails.
USES_KEY = re.compile(r"^\s*(?:-\s*)?uses:")
# The value may be quoted. The action may carry a subpath
# (`github/codeql-action/init`). A trailing `# v7.0.0 ...` comment is
# captured whole so a SHA pin can state its major.
USES = re.compile(
    r"""^\s*(?:-\s*)?uses:\s*(?P<q>['"]?)(?P<action>[^@\s'"#]+)@(?P<ref>[^\s'"#]+)(?P=q)"""
    r"""\s*(?:#\s*(?P<comment>.*))?$"""
)
# Nothing to compare: a local action, or a container image run directly.
LOCAL = re.compile(r"""^\s*(?:-\s*)?uses:\s*['"]?(?:\./|docker://)""")
MAJOR_TAG = re.compile(r"^v(\d+)(?:\.\d+)*$")
SHA = re.compile(r"^[0-9a-f]{40}$")

failures = 0


def check(label: str, ok: bool, detail: str = "") -> bool:
    global failures
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures += 1
        for line in detail.splitlines():
            print(f"        {line}")
    return ok


def major_of(ref: str, comment: str | None) -> int | None:
    """The major named by `@ref`, or by the first word of its `# vN` comment on a SHA pin."""
    match = MAJOR_TAG.match(ref)
    if match:
        return int(match[1])
    if SHA.match(ref) and comment and comment.split():
        match = MAJOR_TAG.match(comment.split()[0])
        if match:
            return int(match[1])
    return None


def relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def pins(path: Path) -> list[tuple[int, str, str, int | None]]:
    """Every action `uses:` in `path` as `(line, action, ref, major)`.

    Local actions, `docker://` images, and reusable workflows are left out. A `uses:` line that
    does not parse is a failure, never a silent skip.
    """
    found = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not USES_KEY.match(line) or LOCAL.match(line):
            continue
        match = USES.match(line)
        if match is None:
            # Reported only when it fails: a PASS line per parsed `uses:`
            # would bury the few lines a reader needs.
            check(
                f"{relative(path)}:{number} has a `uses:` value this guard can parse",
                False,
                f"line: {line.strip()}\n"
                "remedy: write it as `uses: owner/action@ref`, optionally quoted, with"
                " any comment after `#`, or extend USES in this script.",
            )
            continue
        if "/.github/workflows/" in match["action"]:
            continue
        found.append(
            (number, match["action"], match["ref"], major_of(match["ref"], match["comment"]))
        )
    return found


print("the scaffold templates pin the action majors this repository runs")

if not check(
    "the scaffolder ships at least one workflow to compare",
    bool(TEMPLATE_WORKFLOWS),
    "no sdk/create-contextgraph-provider/templates/*/_github/workflows/*.yml found\n"
    "remedy: if the templates moved, point TEMPLATE_WORKFLOWS at them.",
):
    sys.exit(1)

repo_majors: dict[str, dict[int, list[str]]] = {}
for workflow in REPO_WORKFLOWS:
    for number, action, _ref, major in pins(workflow):
        if major is not None:
            repo_majors.setdefault(action, {}).setdefault(major, []).append(
                f"{relative(workflow)}:{number}"
            )

for template in TEMPLATE_WORKFLOWS:
    for number, action, ref, major in pins(template):
        where = f"{relative(template)}:{number}"
        if major is None:
            print(f"  SKIP  {where} pins {action}@{ref}, which names no major")
            continue
        used = repo_majors.get(action)
        if not used:
            print(f"  SKIP  {where} pins {action}@{ref}; this repository does not use it")
            continue
        if len(used) > 1:
            split = "\n".join(f"v{m}: {', '.join(sites)}" for m, sites in sorted(used.items()))
            check(
                f"{where} can follow one repository major for {action}",
                False,
                f"this repository pins {action} on more than one major:\n{split}\n"
                "remedy: settle the repository on one major first.",
            )
            continue
        ((repo_major, sites),) = used.items()
        check(
            f"{where} pins {action} on the major this repository runs (v{repo_major})",
            major == repo_major,
            f"template pins {action}@{ref}; the repository pins v{repo_major} at"
            f" {sites[0]}\n"
            f"remedy: move {where} to {action}@v{repo_major}. Dependabot cannot,"
            " because the template lives under _github/, which the scaffolder"
            " renames to .github/ only at scaffold time.",
        )

if failures:
    print(f"\n{failures} check(s) failed")
    sys.exit(1)
print("\nall checks passed")
