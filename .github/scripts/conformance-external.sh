#!/usr/bin/env bash
# Assert that an EXTERNAL (non-Rust) provider passes the conformance suite —
# every check green, none skipped. This is how "≥2 independent implementations
# pass conformance" (the GOVERNANCE.md freeze criterion) is machine-verified:
# point it at an SDK's example provider and the same Rust suite that judges the
# reference provider judges it too.
#
# Usage:
#   .github/scripts/conformance-external.sh -- <program> [args...]
#   .github/scripts/conformance-external.sh -- node sdk/typescript/dist/examples/example-docs.js
#
# Pairs with conformance-red.sh, which proves the SUITE catches cheaters using
# the Rust fixture; a conformant external provider only needs to be GREEN.
set -euo pipefail

BIN="${BIN:-./target/debug}"
INSPECT="$BIN/contextgraph-inspect"

if [[ ! -x "$INSPECT" ]]; then
  echo "::error::$INSPECT not built — run 'cargo build --workspace --bins' first"
  exit 1
fi

# Everything after the first `--` is the provider command.
provider=()
seen_sep=0
for arg in "$@"; do
  if [[ "$seen_sep" -eq 1 ]]; then
    provider+=("$arg")
  elif [[ "$arg" == "--" ]]; then
    seen_sep=1
  fi
done

if [[ "${#provider[@]}" -eq 0 ]]; then
  echo "::error::no provider command given; usage: $0 -- <program> [args...]"
  exit 1
fi

echo "Probing external provider: ${provider[*]}"
report=$("$INSPECT" stdio --json -- "${provider[@]}" 2>&1 | sed -n '/^{/,$p')

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
    print(f'"'"'  {"OK" if c["status"] == "pass" else "XX"} {c["name"]}: {c["evidence"]}'"'"')

if failed:
    names = ", ".join(c["name"] for c in failed)
    sys.exit(f"::error::external provider is not conformant: {names}")

print(f"\nAll {len(checks)} checks passed — external provider is conformant.")
'
