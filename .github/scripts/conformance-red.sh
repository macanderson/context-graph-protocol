#!/usr/bin/env bash
# Assert that every `--misbehave` mode of the reference provider is CAUGHT by
# the conformance suite.
#
# This is the inverse of an ordinary test, and it is the one that matters most
# for a conformance suite: a suite that only ever passes proves nothing about
# its ability to catch a broken provider. `docs/protocol-advantages.md` claims
# the suite "genuinely catches broken providers rather than rubber-stamping
# everything" — this script is what makes that claim machine-verified.
#
# The mode list is derived from the binary's own `--help`, not hardcoded, so
# adding a misbehaviour mode without a check that catches it turns CI red
# instead of passing unnoticed.
set -euo pipefail

BIN="${BIN:-./target/debug}"
INSPECT="$BIN/contextgraph-inspect"
PROVIDER="$BIN/contextgraph-example-docs"

for exe in "$INSPECT" "$PROVIDER"; do
  if [[ ! -x "$exe" ]]; then
    echo "::error::$exe not built — run 'cargo build --workspace --bins' first"
    exit 1
  fi
done

# Discover the `--misbehave` modes from the binary's own help.
#
# Because each variant carries a doc comment, clap renders the values as a
# bulleted "- mode: description" list rather than the compact
# "[possible values: a, b]" form, and it emits ANSI styling. Strip the escapes
# first, then handle both renderings so this keeps working if the doc comments
# are ever shortened.
help_text=$("$PROVIDER" --help 2>&1 | sed $'s/\033\\[[0-9;]*m//g')

modes=$(printf '%s\n' "$help_text" |
  sed -n 's/^[[:space:]]*-[[:space:]]\{1,\}\([a-z0-9][a-z0-9-]*\):.*/\1/p')

if [[ -z "$modes" ]]; then
  modes=$(printf '%s\n' "$help_text" | tr '\n' ' ' |
    sed -n 's/.*\[possible values: \([^]]*\)\].*/\1/p' |
    tr -d ' ' | tr ',' '\n' | sed '/^$/d')
fi

if [[ -z "$modes" ]]; then
  echo "::error::could not discover --misbehave modes from $PROVIDER --help"
  exit 1
fi

echo "Discovered misbehave modes:"
echo "$modes" | sed 's/^/  - /'
echo

failed=0
while read -r mode; do
  [[ -z "$mode" ]] && continue
  # `|| true` is load-bearing: inspect exits non-zero precisely when it catches
  # a broken provider, which is the outcome this script is asserting. Without
  # it, `set -e` would abort on the first successfully-detected misbehaviour.
  report=$("$INSPECT" stdio --json -- "$PROVIDER" --misbehave "$mode" 2>&1 |
    sed -n '/^{/,$p' || true)

  if [[ -z "$report" ]]; then
    echo "::error::mode '$mode' produced no JSON report"
    failed=1
    continue
  fi

  tripped=$(printf '%s' "$report" | python3 -c '
import json, sys
report = json.load(sys.stdin)
print(",".join(c["name"] for c in report["checks"] if c["status"] != "pass"))
')

  if [[ -z "$tripped" ]]; then
    echo "::error::mode '$mode' passed every check — the suite does not catch it"
    failed=1
  else
    echo "  ✓ $mode -> caught by: $tripped"
  fi
done <<<"$modes"

if [[ "$failed" -ne 0 ]]; then
  echo
  echo "::error::at least one misbehaviour mode went undetected"
  exit 1
fi

echo
echo "All misbehaviour modes were caught."
