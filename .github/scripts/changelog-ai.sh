#!/usr/bin/env bash
#
# Draft CHANGELOG.md entries for a range of merges from their actual diff.
#
# Usage: changelog-ai.sh <git-range>          e.g. changelog-ai.sh abc123..HEAD
#
# Prints Keep-a-Changelog bullets (grouped under ### Added / ### Changed /
# ### Fixed) for the user-visible changes in the range, in this repo's
# changelog voice, to stdout. Prints NOTHING and exits 0 when it cannot or
# should not draft — no API key, the call failed, the range holds nothing
# user-facing, or the response didn't look like changelog markdown — so the
# caller can treat empty output as "leave the changelog alone".
#
# Called from .github/workflows/changelog.yml, which runs it over the merges
# that landed since the last commit that touched CHANGELOG.md and proposes
# the result as a bot PR — a draft for human review, never a direct push.
#
#   AI_GATEWAY_API_KEY   required to produce output; unset -> warn, print nothing
#   CHANGELOG_AI_MODEL   optional, default anthropic/claude-sonnet-5
set -euo pipefail

range="${1:?usage: changelog-ai.sh <git-range>}"

if ! command -v jq >/dev/null 2>&1; then
  echo "changelog-ai: jq not found; printing nothing." >&2
  exit 0
fi
if [ -z "${AI_GATEWAY_API_KEY:-}" ]; then
  echo "changelog-ai: AI_GATEWAY_API_KEY not set; printing nothing." >&2
  exit 0
fi

model="${CHANGELOG_AI_MODEL:-anthropic/claude-sonnet-5}"

commits="$(git log --no-merges --format='%s' "$range" | head -n 100 || true)"
if [ -z "$commits" ]; then
  echo "changelog-ai: no commits in range '${range}'; printing nothing." >&2
  exit 0
fi

bodies="$(git log --no-merges --format='--- %s%n%b' "$range" | head -n 400 || true)"
stat="$(git diff --stat "$range" -- ':(exclude)Cargo.lock' ':(exclude)site/pnpm-lock.yaml' | tail -n 100 || true)"
diff="$(git diff "$range" -- ':(exclude)Cargo.lock' ':(exclude)site/pnpm-lock.yaml' 2>/dev/null | head -c 100000 || true)"

# shellcheck disable=SC2016  # the format string's backticks are literal markdown
prompt="$(printf '%s\n\n## Commit subjects\n%s\n\n## Commit bodies (squash-merge PR descriptions)\n%s\n\n## Files changed\n%s\n\n## Diff (truncated)\n```diff\n%s\n```\n' \
  'Write CHANGELOG.md bullets for the merges below, from the Context Graph Protocol repository: a protocol specification plus Rust crates (contextgraph-types, contextgraph-host, contextgraph-conformance), TypeScript/Python/Go SDKs, and a docs site. Keep a Changelog format: "### Added" / "### Changed" / "### Fixed" / "### Removed" headings (only the ones that apply, in that order), one bullet per distinct change, in this repository'"'"'s established voice: a bold lead phrase, then the PR reference in parens, then an em-dash and two to four lines of concrete prose wrapped at 80 columns — for example: "- **Frame representations — full/compact/reference** (#41) — a provider can now answer with ...". Cover only what a protocol implementer, SDK user, or spec reader would notice: wire types, host runtime behavior, conformance checks, CLI, SDKs, published schemas, normative spec text, docs pages. Skip repo chores, lockfiles, logos, CI plumbing, and site build internals entirely. Base every claim on the diff below — commit messages describe intent, the diff is what shipped. If NOTHING in the range is user-visible, output exactly the single word NOTHING. Output ONLY the bullet markdown - no version heading, no preamble, no code fences.' \
  "$commits" "$bodies" "$stat" "$diff")"

payload="$(jq -n --arg m "$model" --arg c "$prompt" \
  '{model:$m, messages:[{role:"user",content:$c}], temperature:0.2}')"

resp="$(curl -sS --max-time 150 \
  -H "Authorization: Bearer ${AI_GATEWAY_API_KEY}" \
  -H "Content-Type: application/json" \
  -d "$payload" \
  https://ai-gateway.vercel.sh/v1/chat/completions || true)"

entries="$(printf '%s' "$resp" | jq -r '.choices[0].message.content // empty' 2>/dev/null || true)"

entries="$(printf '%s\n' "$entries" | sed -e '/^```/d')"
entries="$(printf '%s' "$entries" | sed -e 's/[[:space:]]*$//')"
if [ "$entries" = "NOTHING" ]; then
  echo "changelog-ai: model judged the range internal-only; printing nothing." >&2
  exit 0
fi
case "$entries" in
  '### '* | '- '*) ;;
  *)
    echo "changelog-ai: response did not look like changelog markdown; printing nothing." >&2
    exit 0
    ;;
esac

printf '%s\n' "$entries"
