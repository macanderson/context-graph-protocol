#!/usr/bin/env python3
"""
Hold this repository to the host boundary in ADR 0008.

Usage:
    python3 .github/scripts/check-deploy-hygiene.py

Exits 0 if every absolute URL this repo advertises for one of its own static
artifacts points at a host this repo actually serves, that artifact exists at
the path the URL names, and the checkout is not Git-linked to the Vercel
project that serves the public apex. Exits 1 otherwise. Stdlib only, offline.

Why this exists (see docs/adr/0008-deploy-topology-and-advertised-urls.md):

  * The Vercel project that owns `contextgraphprotocol.org`, `cgp.oxagen.sh`,
    and `context-graph-protocol.vercel.app` is Git-connected to a *different*
    repository (`macanderson/cgp-website`). This repo's `site/` deploys
    nowhere, so every `…/schema/…`, `…/registry/…`, and `…/badges/…` URL on
    those hosts 404s — including the badge that docs/registry.md tells
    conformant providers to paste into their own READMEs. Nothing failed
    loudly, because no build step dereferences a URL. This check is that step.

  * `.vercel/` is gitignored, so a stray local link is invisible to review —
    and `vercel --prod` from a linked checkout deploys this repo over the
    microsite's production apex. That is how the 2026-07-23 flip-flop
    happened, in both directions.

An artifact URL is one naming a file under /schema/, /registry/, or /badges/
with a real filename. Prose links to the protocol homepage are not artifact
URLs and are not checked here — the apex serves those, and returns 200.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

# The one host that serves this repository's bytes by construction, regardless
# of how the Vercel deploy topology is settled. When `site/` gains a real
# deployment (ADR 0008's open decision), add that host here and the URLs it
# then permits become legal.
SERVED_HOSTS = {"raw.githubusercontent.com"}
RAW_PREFIX = "https://raw.githubusercontent.com/macanderson/context-graph-protocol/main/"

# The Vercel project serving the public apex from macanderson/cgp-website.
# This repo must never be linked to it: see ADR 0008, decision (3).
APEX_PROJECT_ID = "prj_s3lfCDvK9H9PwgvkpXiho1juiR63"

# Historical records legitimately quote URLs that were broken at the time they
# were written. Exempting them is not a loophole — a changelog that silently
# rewrote the URL it is reporting on would be lying about the past.
EXEMPT = ("docs/adr/", "CHANGELOG.md")

ARTIFACT_URL = re.compile(
    r"https?://([A-Za-z0-9.\-]+)(/[^\s)\"'`>]*?/(?:schema|registry|badges)/[A-Za-z0-9._\-]+\.(?:json|svg|png)"
    r"|/(?:schema|registry|badges)/[A-Za-z0-9._\-]+\.(?:json|svg|png))"
)

failures = 0


def check(label: str, ok: bool) -> None:
    global failures
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures += 1


def tracked_text_files() -> list[Path]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    paths = []
    for name in out.split("\0"):
        if not name or name.startswith(EXEMPT):
            continue
        path = ROOT / name
        if not path.is_file():
            continue
        try:
            path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        paths.append(path)
    return paths


print("advertised artifact URLs point at a host this repo serves")

offenders: list[str] = []
missing: list[str] = []
for path in tracked_text_files():
    rel = path.relative_to(ROOT)
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for match in ARTIFACT_URL.finditer(line):
            url = match.group(0)
            host = match.group(1)
            if host not in SERVED_HOSTS:
                offenders.append(f"{rel}:{lineno}  {url}")
                continue
            # The URL resolves to a host we serve — now make sure it resolves
            # to a file that exists. A 404 on a host we control is still a 404.
            if url.startswith(RAW_PREFIX):
                target = ROOT / url[len(RAW_PREFIX):]
                if not target.is_file():
                    missing.append(f"{rel}:{lineno}  {url}")

check("no artifact URL on a host this repo does not deploy", not offenders)
for offender in offenders:
    print(f"        {offender}")
if offenders:
    print(f"        remedy: serve it from {RAW_PREFIX}<repo-relative path>")
    print("        rationale: docs/adr/0008-deploy-topology-and-advertised-urls.md")

check("every advertised artifact exists at the path its URL names", not missing)
for gap in missing:
    print(f"        {gap}")

print("\nthis checkout is not linked to the apex Vercel project")

tracked_links = subprocess.run(
    ["git", "-C", str(ROOT), "ls-files", "--", "*.vercel/project.json", ".vercel/*"],
    capture_output=True,
    text=True,
    check=True,
).stdout.split()
check("no Vercel link is committed", not tracked_links)
for link in tracked_links:
    print(f"        {link}")

linked = []
for link_file in ROOT.glob("**/.vercel/project.json"):
    try:
        project_id = json.loads(link_file.read_text()).get("projectId")
    except (json.JSONDecodeError, OSError):
        continue
    if project_id == APEX_PROJECT_ID:
        linked.append(str(link_file.relative_to(ROOT)))

check("no local checkout is linked to the apex project", not linked)
for link in linked:
    print(f"        {link} -> {APEX_PROJECT_ID}")
if linked:
    print("        remedy: rm -rf the .vercel/ directories listed above.")
    print("        A `vercel --prod` from here would deploy this repo over")
    print("        https://contextgraphprotocol.org (served by cgp-website).")

print(f"\n{'OK — deploy hygiene holds' if failures == 0 else f'{failures} failure(s)'}")
sys.exit(1 if failures else 0)
