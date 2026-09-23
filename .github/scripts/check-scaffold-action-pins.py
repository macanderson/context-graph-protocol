#!/usr/bin/env python3
"""
Hold the scaffold templates' action pins to the majors this repository runs.

Usage:
    python3 .github/scripts/check-scaffold-action-pins.py

Exits 0 if every `uses: owner/action@ref` in a scaffold template's workflow
names the same major that this repository's own workflows pin for that action.
Exits 1 otherwise. Stdlib only, offline.

Why this exists (see docs/adr/0012-sdk-version-pins-share-a-major.md):

  `sdk/create-contextgraph-provider` hands every new provider a
  `.github/workflows/conformance.yml`. In the tree that file lives under
  `templates/<lang>/_github/`, and the scaffolder renames `_github` to
  `.github` only at scaffold time. Dependabot scans `.github/workflows`
  alone, so it bumps every action in this repository's workflows and never
  sees the copy the templates ship. #194 moved `actions/setup-python` to v7
  in three workflows while the Python template stayed on v5, and no job
  could have noticed: the scaffold job runs the conformance script directly
  and never executes the nested workflow.

  That is the failure mode ADR 0012 records for SDK pins, on a different
  axis. The remedy is the same: an offline guard that fails the commit which
  opens the gap, because that commit is the one that can still close it
  cheaply.

The rule:

  For each action a template pins with a readable major, the repository's
  workflows must pin that action, on exactly one major, and the template's
  major must equal it. A ref without a major (`@master`, `@stable`, a local
  `./` path, a reusable workflow at a bare SHA) is skipped, and so is an
  action only the template uses, because there is nothing to compare it to.
  A major is readable from `@v<N>`, `@v<N>.<x>.<y>`, or a SHA ref followed by
  a `# v<N>` comment, which is the form Dependabot maintains for SHA pins.

  Majors, not exact refs, because the repository pins tags (`@v7`) while a
  hardened workflow may pin a SHA with a version comment, and the two are
  the same release line. A major is where an action's inputs and its Node
  runtime change, which is what a scaffolded provider cannot survive.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
REPO_WORKFLOWS = sorted(
    list((ROOT / ".github" / "workflows").glob("*.yml"))
    + list((ROOT / ".github" / "actions").glob("**/action.yml"))
)
TEMPLATE_WORKFLOWS = sorted(
    (ROOT / "sdk" / "create-contextgraph-provider" / "templates").glob(
        "*/_github/workflows/*.yml"
    )
)

# `uses:` may be a step key or a list item, and the value may be quoted. A
# trailing `# v7.0.0` comment is captured so a SHA pin can state its major.
USES = re.compile(
    r"""^\s*(?:-\s*)?uses:\s*['"]?(?P<action>[^@\s'"]+)@(?P<ref>[^\s'"#]+)['"]?"""
    r"""(?:\s*#\s*(?P<comment>.*))?$"""
)
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
    """The major named by `@ref`, or by its `# vN` comment for a SHA pin."""
    match = MAJOR_TAG.match(ref)
    if match:
        return int(match[1])
    if SHA.match(ref) and comment:
        match = MAJOR_TAG.match(comment.split()[0])
        if match:
            return int(match[1])
    return None


def pins(path: Path) -> list[tuple[int, str, str, int | None]]:
    """Every `uses:` in `path` as `(line, action, ref, major)`."""
    found = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = USES.match(line)
        if match is None or match["action"].startswith("./"):
            continue
        found.append(
            (number, match["action"], match["ref"], major_of(match["ref"], match["comment"]))
        )
    return found


def relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


print("the scaffold templates pin the action majors this repository runs")

repo_majors: dict[str, dict[int, list[str]]] = {}
for workflow in REPO_WORKFLOWS:
    for number, action, _ref, major in pins(workflow):
        if major is not None:
            repo_majors.setdefault(action, {}).setdefault(major, []).append(
                f"{relative(workflow)}:{number}"
            )

check(
    "the scaffolder ships at least one workflow to compare",
    bool(TEMPLATE_WORKFLOWS),
    "no sdk/create-contextgraph-provider/templates/*/_github/workflows/*.yml found\n"
    "remedy: if the templates moved, point TEMPLATE_WORKFLOWS at them.",
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
                f"remedy: settle the repository on one major first.",
            )
            continue
        (repo_major, sites), = used.items()
        check(
            f"{where} pins {action} on the major this repository runs (v{repo_major})",
            major == repo_major,
            f"template pins {action}@{ref}; the repository pins v{repo_major} at"
            f" {sites[0]}\n"
            f"remedy: move {where} to {action}@v{repo_major}. Dependabot cannot,"
            f" because the template lives under _github/, which the scaffolder"
            f" renames to .github/ only at scaffold time.",
        )

if failures:
    print(f"\n{failures} check(s) failed")
    sys.exit(1)
print("\nall checks passed")
