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
    failed = [c for c in checks if c["status"] == "fail"]
    for c in checks:
        mark = {"pass": "OK", "skipped": "--"}.get(c["status"], "XX")
        print(f"  {mark} {c['name']}: {c['evidence']}")

    if failed:
        print("\nNOT conformant: " + ", ".join(c["name"] for c in failed), file=sys.stderr)
        return 1
    print(f"\nAll {len(checks)} checks passed -- provider is conformant.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
