#!/usr/bin/env python3
"""Assert this provider is conformant: run ``contextgraph-inspect`` against the
stdio provider and fail (non-zero exit) if any check is not ``pass``. This is
what the bundled CI workflow runs -- and what you can run locally.

The inspect binary is found via the CONTEXTGRAPH_INSPECT env var if set,
otherwise ``contextgraph-inspect`` on PATH (install it with
``cargo install contextgraph-conformance``).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INSPECT = os.environ.get("CONTEXTGRAPH_INSPECT", "contextgraph-inspect")


def main() -> int:
    provider = ROOT / "provider.py"
    if not provider.exists():
        print(f"{provider} not found", file=sys.stderr)
        return 1

    try:
        proc = subprocess.run(
            [INSPECT, "stdio", "--json", "--", sys.executable, str(provider)],
            capture_output=True,
            text=True,
            cwd=ROOT,
        )
    except FileNotFoundError:
        print(
            f'could not run "{INSPECT}". Install it with '
            "`cargo install contextgraph-conformance`, or set CONTEXTGRAPH_INSPECT "
            "to a prebuilt binary.",
            file=sys.stderr,
        )
        return 1

    # The report is the JSON block after the human-readable probe output; parse
    # from the first line that starts with `{` (CLICOLOR_FORCE-safe: the JSON
    # block itself carries no ANSI).
    lines = proc.stdout.splitlines()
    start = next((i for i, line in enumerate(lines) if line.lstrip().startswith("{")), None)
    if start is None:
        print("no JSON report in inspect output:\n" + proc.stdout + proc.stderr, file=sys.stderr)
        return 1

    try:
        report = json.loads("\n".join(lines[start:]))
    except json.JSONDecodeError as error:
        print(f"could not parse inspect report: {error}", file=sys.stderr)
        return 1

    checks = report.get("checks", [])

    for c in checks:
        mark = {"pass": "OK", "skipped": "--"}.get(c["status"], "XX")
        print(f"  {mark} {c['name']}: {c['evidence']}")

    # A report with no checks is not a passing report. Before this, an empty
    # list printed "All 0 checks passed -- provider is conformant" and exited 0,
    # so an inspect run that probed nothing was indistinguishable from a clean
    # one. This is the first quality signal a provider author ever sees; it has
    # to tell "everything passed" from "nothing ran".
    if not checks:
        print(
            "\nNOT conformant: the report contains no checks at all, so nothing "
            "was verified. This usually means inspect could not reach the "
            "provider, or the provider exited before the handshake. Run "
            "`contextgraph-inspect stdio -- python -m provider.stdio` by hand "
            "and read the probe output above the JSON.",
            file=sys.stderr,
        )
        return 1

    # Anything that is not `pass` and not `skipped` counts against the provider,
    # rather than only the exact string "fail". A status this script does not
    # know -- an `error`, or one a future inspect adds -- must not be read as
    # success by a check whose whole job is to be strict.
    passed = [c for c in checks if c["status"] == "pass"]
    skipped = [c for c in checks if c["status"] == "skipped"]
    failed = [c for c in checks if c["status"] not in ("pass", "skipped")]

    if failed:
        print(
            "\nNOT conformant: "
            + ", ".join(f"{c['name']} ({c['status']})" for c in failed),
            file=sys.stderr,
        )
        return 1

    # Skipped checks are reported separately rather than folded into the total.
    # A transport legitimately skips some -- an HTTP provider cannot answer the
    # three stdio-only ones -- but "13 checks passed" when 8 were skipped
    # overstates what was verified, and the number is what an author quotes.
    if skipped:
        summary = f"{len(passed)} passed, {len(skipped)} skipped"
    else:
        summary = f"all {len(passed)} checks passed"
    print(f"\nConformant -- {summary}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
