#!/usr/bin/env bash
# Poll the crates.io sparse index until a just-published crate version is
# visible, so the next `cargo publish` in the dependency chain (which
# resolves its path dependency's version requirement against the registry,
# not the local path — see PUBLISHING.md) doesn't race the CDN. Usually
# resolves in seconds; PUBLISHING.md notes it can occasionally take a minute
# or two.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <crate-name> <version> [max-attempts] [sleep-seconds]" >&2
  exit 2
fi

crate="$1"
version="$2"
max_attempts="${3:-30}"
sleep_seconds="${4:-10}"

# Sparse index path convention: https://doc.rust-lang.org/cargo/reference/registry-index.html#index-files
lower=$(printf '%s' "$crate" | tr '[:upper:]' '[:lower:]')
len=${#lower}
if [[ $len -eq 1 ]]; then
  path="1/$lower"
elif [[ $len -eq 2 ]]; then
  path="2/$lower"
elif [[ $len -eq 3 ]]; then
  path="3/${lower:0:1}/$lower"
else
  path="${lower:0:2}/${lower:2:2}/$lower"
fi

url="https://index.crates.io/$path"

# The version crosses into Python through the environment, never through the
# program text. `$version` is a git tag name with a fixed prefix stripped
# (`release.yml` passes "${GITHUB_REF_NAME#contextgraph-v}"), and git accepts
# apostrophes and newlines in a tag. Interpolated into the source of the
# snippet below, an apostrophe closed the string literal and killed the release
# with a bare SyntaxError, and a newline plus a statement ran arbitrary Python
# in the job that holds CARGO_REGISTRY_TOKEN. Reading it from os.environ is the
# whole fix: the value is data on both sides of the boundary.
read -r -d '' find_version <<'PYTHON' || true
import json, os, sys

target = os.environ["CRATE_VERSION"]
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    entry = json.loads(line)
    if entry.get("vers") == target:
        sys.exit(0)
sys.exit(1)
PYTHON

for attempt in $(seq 1 "$max_attempts"); do
  if curl -fsSL "$url" 2>/dev/null |
    CRATE_VERSION="$version" python3 -c "$find_version"; then
    echo "$crate $version is live on the sparse index."
    exit 0
  fi
  echo "Attempt $attempt/$max_attempts: $crate $version not yet visible on the sparse index, waiting ${sleep_seconds}s..."
  sleep "$sleep_seconds"
done

echo "::error::$crate $version did not appear on the sparse index after $((max_attempts * sleep_seconds))s"
exit 1
