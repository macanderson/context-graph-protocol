# 26. Versions written in prose, and the Go SDK's tag

- Status: accepted
- Date: 2026-09-23
- Extends [ADR 0012](0012-sdk-version-pins-share-a-major.md), whose "What it
  does not cover" list named prose and the Go SDK as open. Tracking issue:
  [#108](https://github.com/macanderson/context-graph-protocol/issues/108).
  Repository policy; no wire or spec impact.

## Context

`check-sdk-version-pins.py` (ADR 0012) compares versions between manifests.
Versions written in prose were invisible to it, and some had already gone
stale:

- `sdk/PUBLISHING.md`'s acceptance bar pinned `go get …/contextgraph@v0.1.0`
  next to two unpinned siblings (`npm install …`, `pip install …`), with the
  repository on `2.0.0`. A reader could not tell whether `v0.1.0` was
  deliberate or left behind. `sdk/go/README.md` had the same pin in its
  install line.
- The same file's status table said Python was "published 0.1.0" while
  `sdk/python/pyproject.toml` shipped `2.0.0`. It was unclear whether the table
  recorded the first publish or tracked the current version.
- `docs/protocol-advantages.md` told adopters to pin
  `contextgraph-types = "=0.1.0"`, with the crates on `2.x`. `sdk/README.md`
  said Python and Go were "not yet installable outside a checkout" after both
  had been published.

The Go SDK is a harder case. A Go module is versioned by its git tag, so
`sdk/go/go.mod` has no version for a check to read. The only tag is
`sdk/go/v0.1.0`. `MIGRATION.md` §5.6 says "SDKs move in lockstep" but then
describes only the npm and PyPI packages, and says the Go SDK was unchanged in
`2.0.0`. So the question was whether the Go tag should be held to the crates'
major like the other SDKs.

## Decision

### 1. The status table is a historical record, and prose install commands are unpinned

`sdk/PUBLISHING.md`'s table records each SDK's **first** publish (version and
date) and does not change after that. All three rows follow this. The current
version lives where it is authoritative: the npm and PyPI manifests, and the
newest `sdk/go/v*` tag for Go. A table meant to track current versions would
be a copy of those, and a copy goes stale. The previous table already had.

The acceptance bar's install commands are unpinned (`@latest` for Go), so they
check the release that was just made. A runbook step that needs an exact
version writes the placeholder `vX.Y.Z`.

### 2. The Go SDK's tag is **not** held to the crates' major

The Go SDK has its own version line. Two reasons, the first decisive:

- **Go ties a module's major version to its import path.** From `v2` on, the
  module path must end in `/v2` (`…/sdk/go/v2`), and every consumer changes
  every import. Holding the Go tag to the crates' major would force that
  import-path change on Go users at every crate major, even when the Go API
  had not changed. `2.0.0` is the example. Its break was `FrameKind` widening,
  which the Go SDK did not have then (MIGRATION.md §5.6), so a Go `v2` would
  have renamed every import for nothing. The TypeScript and Python lockstep
  is cheap because a major in those ecosystems costs a version number, not a
  rename. In Go it costs the rename.
- **An offline check cannot read a tag.** CI checks out one commit and
  fetches no tags. The check would need `fetch-depth: 0` or `git fetch --tags`,
  and would then fail on any fork or shallow clone that lacks them. That is a
  poor trade for enforcing a rule the first reason already rejects.

The Go SDK's major moves when the Go API breaks. `MIGRATION.md` §5.6 now says
the lockstep covers the npm and PyPI packages only.

### 3. Versions in prose are checked, and the Go SDK's never appear there

`check-sdk-version-pins.py` now also reads tracked Markdown (`git ls-files
'*.md'`). It holds every version written for a package this repository
publishes to ADR 0012's rule: same major as the package's manifest, and not
ahead of it. That covers:

- npm: `@contextgraphprotocol/typescript-sdk@<v>`,
  `create-contextgraph-provider@<v>`, `npm create contextgraph-provider@<v>`
- PyPI: `contextgraph-sdk==|>=|~=<v>`
- crates: `<crate> = "<req>"`, `<crate> = { version = "<req>", … }`, and
  `cargo add|install <crate>@<v>`, for every workspace crate with
  `publish = true`

`@latest` and unpinned commands are never flagged. Neither is `vX.Y.Z`, or any
other text that does not start with a digit.

A concrete Go SDK version in prose (`go get …/sdk/go…@v1.2.3`) is always a
failure. There is no manifest to compare it with, so no check could keep it
true, and it would become the next stale `@v0.1.0`.

`CHANGELOG.md` and `docs/adr/` are exempt, as they are from
`check-deploy-hygiene.py`. They quote versions as history, and a changelog
that rewrote the version it reports would misstate the past.

## Consequences

- The stale pins found in #108 are gone. A new one fails CI in the commit
  that writes it, with the file, line and shipped version.
- A crate major bump that leaves an old major in a README now fails CI along
  with the manifests, so the bump is one change.
- The Go SDK's version is watched by nothing automatic. That is accepted: the
  only thing to watch it against would be a lockstep this ADR rejects.
  Publishing a Go tag stays a deliberate human act (sdk/PUBLISHING.md).
- `schema/reference-vectors.ndjson` is untouched. Its `"version": "1.0.0"`
  strings are provider versions inside fixtures, and the prose check reads
  only Markdown.
- The tag *naming* conventions (#103, #105) are separate. This ADR adds no
  tag convention; it uses Go's required `sdk/go/vX.Y.Z` shape as it is.
