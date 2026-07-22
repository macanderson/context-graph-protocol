#!/usr/bin/env bash
# Assert that the conformance suite is GREEN against the conformant reference
# provider — every check passes, none is skipped away.
#
# Pairs with conformance-red.sh: green alone would be satisfied by a suite that
# never fails, and red alone by a suite that never passes. Both together are
# what make "CGP conformant" a checkable claim.
set -euo pipefail

BIN="${BIN:-./target/debug}"
INSPECT="$BIN/contextgraph-inspect"
PROVIDER="$BIN/contextgraph-example-docs"

report=$("$INSPECT" stdio --json -- "$PROVIDER" 2>&1 | sed -n '/^{/,$p')

if [[ -z "$report" ]]; then
  echo "::error::conformance run produced no JSON report"
  exit 1
fi

printf '%s' "$report" | python3 -c '
import json, sys

report = json.load(sys.stdin)
checks = report["checks"]
if not checks:
    sys.exit("::error::report contained no checks at all")

failed = [c for c in checks if c["status"] != "pass"]
for c in checks:
    print(f'"'"'  {"✓" if c["status"] == "pass" else "✗"} {c["name"]}: {c["evidence"]}'"'"')

if failed:
    names = ", ".join(c["name"] for c in failed)
    sys.exit(f"::error::the reference provider is not conformant: {names}")

print(f"\nAll {len(checks)} checks passed.")
'
