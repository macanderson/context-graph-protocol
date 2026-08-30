# 0012 — A version pin names its manifest's major

**Status:** Accepted (repository policy; no wire or spec impact)

## Context

`sdk/create-contextgraph-provider/index.js` carries a `DEFAULT_SDK` table: the
dependency each scaffolded project resolves when the caller passes no `--sdk`.
It states a version of two packages it does not own —
`@contextgraphprotocol/typescript-sdk` and `contextgraph-sdk` — and those two
packages state their own versions in `sdk/typescript/package.json` and
`sdk/python/pyproject.toml`.

Those numbers went out of agreement and stayed that way. The TypeScript entry
sat at `^0.1.0` while its manifest shipped `1.0.0` — a full major behind, for
an unknown length of time, found only in passing during the `2.0.0` bump
(#98). Every project scaffolded in that window resolved an SDK from the wrong
major.

Nothing in CI could have caught it, and the job that looks closest is the one
that proves it. `create-contextgraph-provider scaffolds a conformant project`
runs the scaffolder with `--sdk "file:$GITHUB_WORKSPACE/sdk/typescript"` and
`pip install ./sdk/python`, so **both published pins are overridden by local
paths on every run**. That is the correct design for that job — resolving the
real pin would make CI depend on a publish, and a red CI after a registry
outage teaches nobody anything — but it means the job proves the *templates*
are conformant and says nothing about whether the versions they name exist.

## Decision

**A version pin naming a package this repository publishes must share that
package's major and must not run ahead of it.** Enforced offline by
`.github/scripts/check-sdk-version-pins.py`, run in CI as `sdk version pins`.

### Same major, not equality

The two pins are deliberately ranges, and differently shaped ones: a caret
(`^2.0.0`) for npm and a floor (`contextgraph-sdk>=2.0.0`) for pip. Equality
would turn every SDK patch release into an edit here that nobody would
remember to make, and the range exists precisely so a scaffold picks up a
patch without one.

A major is the thing a scaffold cannot survive. Majors are where the SDK's API
changes, and ADR 0011 is the worked example: `FrameKind` widened to an open
vocabulary in `2.0.0`, so a project scaffolded against a `1.x` pin generates
code against a vocabulary that release retired. Matching the major is
therefore the weakest rule that still catches every break, which is what a
guard should be — a stricter one would fail on changes that are correct.

### Not ahead of the manifest, either

Drift has a second direction: a pin flooring at `2.1.0` when only `2.0.0` has
ever shipped resolves to nothing at all. The check reads the manifest version
as the newest release that can exist, which holds because these manifests are
what `publish-sdks.yml` publishes.

### The lockstep is checked too, because the pins lean on it

The scaffolder's own comment and `MIGRATION.md` §5.4 both assert that an SDK
major and a crate major are one release. The pin rule borrows its meaning from
that: `^2.0.0` protects a scaffold from ADR 0011 only while SDK 2 and crate 2
name the same break. So the check also holds `Cargo.toml`'s
`[workspace.package] version`, `sdk/typescript/package.json` and
`sdk/python/pyproject.toml` to one major. Its practical effect is that a
version bump is one commit across the three files rather than three commits
with a window in between.

### What it does not cover

- **`sdk/go`** carries no package version. A Go module is versioned by its git
  tag (`sdk/go/v…`), not by a field in `go.mod`, and the scaffolder emits no Go
  template — so there is no in-tree pin to compare. Its `ProtocolVersion`
  constant is a *protocol* version, a separate axis governed by `SPEC.md` §3.1.
- **`schema/reference-vectors.ndjson`**'s `"version": "1.0.0"` strings are the
  *provider* versions of the fixtures in the vectors, not package versions of
  anything here. Rewriting them would change the bytes the reference vectors
  pin, so they stay put.
- **`create-contextgraph-provider`'s own `version`** is a version it owns and
  may move on its own cadence.
- **Prose.** A README's `npm install …@2` would not be caught. Extending the
  check there is possible and was left out as a separate judgement, not an
  oversight.

## Consequences

- A `DEFAULT_SDK` entry left behind by an SDK bump fails CI at the commit that
  bumps the SDK, which is the commit that can still fix it cheaply.
- Bumping an SDK major becomes one atomic change across four files: the two
  SDK manifests, `Cargo.toml`, and `DEFAULT_SDK`. A staged bump is refused.
  That is the intent — the staged state is exactly the drift #98 recorded.
- A template that hardcodes an SDK version instead of taking `{{SDK_SPEC}}`
  fails too, because it would make the pin this check reads stop being the pin
  that ships.
- The check reads no registry, so it cannot go red because npm or PyPI is
  down, and it cannot verify that a pinned version was actually published. The
  `sdk (…) is a conformant implementation` jobs and `publish-sdks.yml` own that
  half.
