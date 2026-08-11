#!/usr/bin/env bash
# Downstream canary (issue #29): build stella against THIS repo's HEAD.
#
# stella consumes contextgraph-types::ContextFrame (and, transitively,
# contextgraph-host / contextgraph-trace / contextgraph-conformance) as a
# pinned git dependency. That pin only moves when a human bumps it, so a
# breaking change here can sit unnoticed until someone does. This script
# closes that gap: it patches a stella checkout to build against a *local*
# CGP checkout (this repo, at whatever ref is checked out — HEAD in CI) via
# Cargo's `[patch]` table, then builds and tests every stella crate that
# actually depends on a contextgraph-* crate.
#
# This is the code-side half of the #27 boundary enforcement (see
# docs/adaptive-context-reconciliation.md and docs/adr/0007-protocol-product-
# boundary.md); the docs-side half is stella's own `normative-home` workflow
# (stella PR #500), which checks the *pointer* rather than the *build*.
#
# Usage (matches the .github/scripts/conformance-*.sh convention — env vars,
# no flags, safe to run twice):
#   CGP_DIR=/path/to/context-graph-protocol \
#   STELLA_DIR=/path/to/stella \
#   .github/scripts/downstream-canary-stella.sh
#
# Deliberately advisory (see the calling workflow's continue-on-error): a
# real break here is exactly the kind of pre-freeze signal issue #29 wants,
# but a canary that could fail *this* repo's own required checks would just
# get muted, which defeats the point.
#
# Grep, not rg; find, not fd — this script has to run unmodified on GitHub's
# stock ubuntu-latest runner and on a contributor's machine with no extra
# tools installed, so it only uses what a bare POSIX + coreutils + cargo
# environment already guarantees.
set -euo pipefail

CGP_DIR="${CGP_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
STELLA_DIR="${STELLA_DIR:-}"

if [[ -z "$STELLA_DIR" ]]; then
  echo "::error::STELLA_DIR is not set — point it at a checkout of macanderson/stella"
  exit 1
fi

CGP_DIR="$(cd "$CGP_DIR" && pwd)"
STELLA_DIR="$(cd "$STELLA_DIR" && pwd)"
STELLA_MANIFEST="$STELLA_DIR/Cargo.toml"
CGP_GIT_SOURCE="https://github.com/macanderson/context-graph-protocol"
SENTINEL="# --- downstream-canary-stella.sh: local CGP patch (do not commit) ---"

if [[ ! -f "$STELLA_MANIFEST" ]]; then
  echo "::error::$STELLA_MANIFEST not found — is STELLA_DIR a stella checkout?"
  exit 1
fi

# Discover the contextgraph-* crates this checkout actually ships, from their
# own `[package] name`, rather than hardcoding the list — so a rename or a
# split crate is picked up automatically instead of silently going unpatched.
crates=()
for manifest in "$CGP_DIR"/contextgraph-*/Cargo.toml; do
  [[ -f "$manifest" ]] || continue
  name=$(grep -m1 '^name = ' "$manifest" | cut -d'"' -f2)
  [[ -n "$name" ]] && crates+=("$name")
done

if [[ "${#crates[@]}" -eq 0 ]]; then
  echo "::error::no contextgraph-*/Cargo.toml found under $CGP_DIR"
  exit 1
fi

echo "CGP crates available to patch in: ${crates[*]}"

# A local patch must still satisfy the downstream's declared semver requirement.
# During a release-major canary, temporarily align exact workspace pins with
# this checkout so the canary tests source compatibility rather than stopping
# at dependency resolution. This only mutates the disposable downstream checkout.
cgp_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$CGP_DIR/Cargo.toml" | head -1)
for crate in "${crates[@]}"; do
  sed -i -E "s|^(${crate}[[:space:]]*=[[:space:]]*)\"=?[0-9][^\"]*\"|\1\"=${cgp_version}\"|" "$STELLA_MANIFEST"
done

if grep -qF "$SENTINEL" "$STELLA_MANIFEST"; then
  echo "stella's Cargo.toml already carries the local-CGP patch — leaving it as-is."
else
  echo "Patching $STELLA_MANIFEST to pin contextgraph-* at $CGP_DIR (local checkout)"
  {
    echo ""
    echo "$SENTINEL"
    echo "[patch.\"$CGP_GIT_SOURCE\"]"
    for crate in "${crates[@]}"; do
      printf '%s = { path = "%s/%s" }\n' "$crate" "$CGP_DIR" "$crate"
    done
    echo ""
    echo '[patch.crates-io]'
    for crate in "${crates[@]}"; do
      printf '%s = { path = "%s/%s" }\n' "$crate" "$CGP_DIR" "$crate"
    done
  } >>"$STELLA_MANIFEST"
fi

echo "--- patched Cargo.toml tail ---"
tail -n "$(( 2 * ${#crates[@]} + 5 ))" "$STELLA_MANIFEST"
echo "-------------------------------"

# Discover which stella crates depend on a contextgraph-* crate at all, from
# their manifests, rather than hardcoding stella-graph/stella-context/
# stella-cli — so the canary keeps tracking the real dependency edge as
# stella's own crate graph changes.
dependents=()
while IFS= read -r manifest; do
  dependents+=("$(basename "$(dirname "$manifest")")")
done < <(cd "$STELLA_DIR" && find . -mindepth 2 -maxdepth 3 -name Cargo.toml \
  -exec grep -lE '^contextgraph-[a-z-]+([.]workspace)?[[:space:]]*=' {} \; | sort -u)

if [[ "${#dependents[@]}" -eq 0 ]]; then
  echo "::error::no stella crate depends on contextgraph-* — is STELLA_DIR stale, or did the dependency move?"
  exit 1
fi

echo "stella crates depending on contextgraph-*: ${dependents[*]}"

package_args=()
for pkg in "${dependents[@]}"; do
  package_args+=(-p "$pkg")
done

cd "$STELLA_DIR"
echo "--- cargo build (${dependents[*]}) against local CGP checkout ---"
cargo build "${package_args[@]}"

if [[ "${DOWNSTREAM_CANARY_BUILD_ONLY:-0}" == "1" ]]; then
  echo "DOWNSTREAM_CANARY_BUILD_ONLY=1 — skipping cargo test."
  exit 0
fi

echo "--- cargo test (${dependents[*]}) against local CGP checkout ---"
cargo test "${package_args[@]}"
