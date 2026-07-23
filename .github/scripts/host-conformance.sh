#!/usr/bin/env bash
# Assert the reference host upholds every host-binding rule (SPEC.md §11.1,
# issue #14).
#
# This is the host-side dual of conformance-red.sh. The provider suite proves
# it catches broken *providers*; this proves the reference *host* catches broken
# providers — each host-side check drives an ADVERSARIAL in-process provider
# that tries to make the host fail (blow the budget, flood frames, egress
# without consent, tamper a provenance digest, ...) and passes only if the host
# catches it. So a CONFORMANT verdict here means every adversarial provider was
# caught: a host that failed to gate consent, drop an over-budget provider, or
# detect a tampered digest turns this red.
set -euo pipefail

BIN="${BIN:-./target/debug}"
INSPECT="$BIN/contextgraph-inspect"

if [[ ! -x "$INSPECT" ]]; then
  echo "::error::$INSPECT not built — run 'cargo build --workspace --bins' first"
  exit 1
fi

# `|| true` mirrors conformance-red.sh: inspect exits non-zero precisely when a
# host-binding rule is violated, which is the outcome this script inspects.
report=$("$INSPECT" host --json 2>&1 | sed -n '/^{/,$p' || true)

if [[ -z "$report" ]]; then
  echo "::error::host conformance produced no JSON report"
  exit 1
fi

echo "Host-binding checks:"
printf '%s' "$report" | python3 -c '
import json, sys
report = json.load(sys.stdin)
marks = {"pass": "PASS", "fail": "FAIL", "skipped": "SKIP"}
for check in report["checks"]:
    status = marks.get(check["status"], check["status"].upper())
    print("  [" + status + "] " + check["name"] + ": " + check["evidence"])
'
echo

failed=$(printf '%s' "$report" | python3 -c '
import json, sys
report = json.load(sys.stdin)
print(",".join(c["name"] for c in report["checks"] if c["status"] != "pass"))
')

if [[ -n "$failed" ]]; then
  echo "::error::the reference host violated host-binding rule(s): $failed"
  exit 1
fi

echo "The reference host upholds every checked host-binding rule."
