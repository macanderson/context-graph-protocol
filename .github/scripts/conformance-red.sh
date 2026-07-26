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

# Each mode's doc comment already names the check it is supposed to trip —
# "(trips `budget-honesty`)" — so the expected check is derived from the binary
# too, never hardcoded here. That closes the hole this script used to have: it
# asserted only that *some* check went red, so a mode could be "caught" by an
# unrelated check while the check that actually owns the guarantee silently
# stopped working. `crash-on-query` legitimately names two alternatives, so the
# contract is "at least one of the checks this mode claims to trip".
#
# Output is one `mode<TAB>expected1,expected2` record per line.
mode_records=$(printf '%s\n' "$help_text" | python3 -c '
import re, sys

help_text = sys.stdin.read()
# The bulleted rendering clap uses when a variant carries a doc comment.
for mode, description in re.findall(
    r"^\s*-\s+([a-z0-9][a-z0-9-]*):\s*(.*)$", help_text, re.MULTILINE
):
    trips = re.search(r"\(trips ([^)]*)\)", description)
    expected = re.findall(r"`([a-z][a-z0-9-]*)`", trips.group(1)) if trips else []
    print(mode + "\t" + ",".join(expected))
')

if [[ -z "$mode_records" ]]; then
  # Compact "[possible values: a, b]" rendering — no doc comments, so no
  # expected-check information is available and every mode falls back to
  # "caught by anything".
  mode_records=$(printf '%s\n' "$help_text" | tr '\n' ' ' |
    sed -n 's/.*\[possible values: \([^]]*\)\].*/\1/p' |
    tr -d ' ' | tr ',' '\n' | sed '/^$/d' | sed 's/$/\t/')
fi

modes=$(printf '%s\n' "$mode_records" | cut -f1)

if [[ -z "$modes" ]]; then
  echo "::error::could not discover --misbehave modes from $PROVIDER --help"
  exit 1
fi

echo "Discovered misbehave modes:"
echo "$modes" | sed 's/^/  - /'
echo

failed=0
while IFS=$'\t' read -r mode expected; do
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
    continue
  fi

  if [[ -z "$expected" ]]; then
    echo "  ✓ $mode -> caught by: $tripped (no declared check to match against)"
    continue
  fi

  # The mode must be caught by a check it actually claims to trip.
  if MODE_TRIPPED="$tripped" MODE_EXPECTED="$expected" python3 -c '
import os, sys
tripped = {c for c in os.environ["MODE_TRIPPED"].split(",") if c}
expected = {c for c in os.environ["MODE_EXPECTED"].split(",") if c}
sys.exit(0 if tripped & expected else 1)
'; then
    echo "  ✓ $mode -> caught by: $tripped (expected: $expected)"
  else
    echo "::error::mode '$mode' was caught by [$tripped], but none of the checks it declares it trips [$expected] went red — the check that owns this guarantee has stopped catching it"
    failed=1
  fi
done <<<"$mode_records"

if [[ "$failed" -ne 0 ]]; then
  echo
  echo "::error::at least one misbehaviour mode went undetected"
  exit 1
fi

echo
echo "All misbehaviour modes were caught."
