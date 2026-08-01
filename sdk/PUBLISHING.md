# Publishing the Context Graph Protocol SDKs

This documents the release process for the three provider SDKs — `sdk/typescript`,
`sdk/python`, `sdk/go` — to their respective package registries. Each SDK is an
**independent implementation** of the same wire contract (that's the point —
see [`sdk/README.md`](./README.md)), so unlike the workspace crates
([`../PUBLISHING.md`](../PUBLISHING.md)) there is no dependency order between
them: any SDK can publish without the others being live. What they share is a
target version (`0.1.0` for the first release of each) and the same bar —
green on `.github/scripts/conformance-external.sh` — before anything goes out.

| SDK | Registry | Status |
| --- | --- | --- |
| TypeScript | npm, `@contextgraphprotocol/typescript-sdk` | ✅ published (PR #46) |
| Python | PyPI, [`contextgraph-sdk`](https://pypi.org/project/contextgraph-sdk/) | ✅ published 0.1.0 (2026-07-31) |
| Go | Go module proxy, `.../sdk/go/contextgraph` | ⬜ not yet published (tag-gated, see below) |

**Nobody has run the Go publish steps yet.** This file exists so the
*first* real publish of each is a checklist, not an improvisation — exactly
the role [`../PUBLISHING.md`](../PUBLISHING.md) plays for the crates.

## npm (already live — for the next bump)

The TypeScript SDK's first publish already happened (PR #46), so this is the
one registry where the "one-time prerequisites" are already satisfied for this
maintainer account. Recorded here so a *second* release doesn't require
relearning it:

1. An npm account with 2FA enabled and publish access to the
   `@contextgraphprotocol` org scope.
2. `npm login` locally (or an automation token for CI — see the workflow
   below).
3. Bump `version` in `sdk/typescript/package.json`, then from `sdk/typescript`:
   ```bash
   npm install
   npm run build
   npm pack --dry-run   # inspect the tarball contents before anything uploads
   npm publish --access public
   ```
   `files` in `package.json` is already scoped to `["dist/src", "README.md"]`,
   so `npm pack --dry-run` is the cheap way to confirm a source-map or stray
   test file hasn't crept into what ships.

## PyPI

### One-time prerequisites

1. A PyPI account with 2FA enabled.
2. Either:
   - An API token scoped to the `contextgraph-sdk` project (PyPI Account
     Settings → API tokens — a *project-scoped* token is only available after
     the first upload; the **first** publish necessarily uses an
     account-scoped token, which should be rotated to a project-scoped one
     immediately after), or
   - PyPI **Trusted Publishing** (OIDC from GitHub Actions, no stored secret
     at all) configured against this repository and the `publish-sdks.yml`
     workflow below — the preferred long-term setup, but it can only be
     configured for a project that already exists on PyPI, so it too follows
     the first manual publish rather than replacing it.
3. Confirm the name is still unclaimed:
   `https://pypi.org/pypi/contextgraph-sdk/json` — a 404 means free. (Checked
   2026-07-29: 404, unclaimed.)

### The publish sequence

Run from `sdk/python`:

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install build twine

# Build sdist + wheel into dist/
python -m build

# Validate metadata/README rendering with no network call and no upload —
# this is the pre-publish proof that belongs in a PR or a dry run.
twine check dist/*

# The real, one-way upload.
twine upload dist/*
```

`twine check` catches the two most common first-publish failures (malformed
`long_description`/README rendering, missing/invalid classifiers) before
anything reaches the index. It does **not** catch a name collision or a
duplicate version — PyPI itself rejects those at upload time, and rejects
re-uploading an existing version outright (no overwrite, ever; see below).

### Post-publish verification

In a scratch directory *outside* this workspace:

```bash
python3 -m venv /tmp/cgp-sdk-smoke && source /tmp/cgp-sdk-smoke/bin/activate
pip install contextgraph-sdk
python3 -c "import contextgraph_sdk; print(contextgraph_sdk.__file__)"
```

Then, from the repository root (with `cargo build --workspace --bins` run
once so the conformance binary exists), prove the *installed* package — not
the in-tree copy — still passes conformance by pointing the example provider's
shebang at the scratch venv's interpreter, or simpler, copy
`sdk/python/examples/example_docs.py` into the scratch dir and run it with the
scratch venv's `python3` (the example only imports `contextgraph_sdk`, so it
is agnostic to where that package physically resolves from). Copy the
example's sibling `fixtures/` directory along with it — the provider serves
file provenance whose digests are computed from those files, resolved
relative to the example's own location:

```bash
./.github/scripts/conformance-external.sh -- /tmp/cgp-sdk-smoke/bin/python3 /tmp/example_docs.py
```

A green run here is the acceptance criterion from #59: "`pip install
contextgraph-sdk` ... can build/run the example provider," checked against the
*published* package, not the workspace checkout.

## Go

Go modules don't have an upload step — **the tag is the publish.** Once a
tag matching the module's path exists on the public GitHub remote, the
module is immediately `go get`-able; there is no registry account, no token,
and no separate "release" action beyond `git tag` + `git push --tags`.

### Why the tag has to be `sdk/go/vX.Y.Z`, not `vX.Y.Z`

This repository is a Rust workspace with no root `go.mod` — `sdk/go/go.mod`
is a **nested module** whose module path is
`github.com/macanderson/context-graph-protocol/sdk/go`. Go's [multi-module
repository convention](https://go.dev/ref/mod#vcs-version) requires a nested
module's tags to be prefixed with its subdirectory path relative to the repo
root, so the first tag is:

```
sdk/go/v0.1.0
```

A bare `v0.1.0` tag would be ignored by the `sdk/go` module entirely (that
tag pattern is reserved for a module living at the repo root, which doesn't
exist here) — it's an easy mistake to make once and then have to explain why
`go get ...@v0.1.0` 404s while `go get ...@sdk/go/v0.1.0` works.

Note this is a distinct tag from the general repo release-tagging tracked in
#30 (a root-level `v0.0.2` for downstream git-pins to the Rust crates) — the
Go SDK's tag is independent of whatever prefix or cadence that one settles
on, but #30 is the first real tag this repository will have cut since the
pre-rename `ocp-v0.1.0`, so treat it as the dry run for the mechanics
(annotated tag, changelog cross-reference, pushing tags at all) that this
tag then repeats.

### The publish sequence

```bash
# From the repo root, after confirming sdk/go/go.mod's version is 0.1.0-ready
# (no in-flight breaking changes) and CI is green on the commit being tagged:
git tag -a sdk/go/v0.1.0 -m "sdk/go v0.1.0"
git push origin sdk/go/v0.1.0
```

This is the one command sequence in this document that is *also* explicitly
out of scope for any agent to run unattended (see "one-way door" below) —
unlike npm/PyPI where a stray dry-run is harmless, `git push` of a tag is
itself the irreversible act for Go.

### Pseudo-versions in the meantime

Until the tag exists, `sdk/go` is still technically fetchable by an exact
commit, via Go's **pseudo-version** mechanism — `go get
github.com/macanderson/context-graph-protocol/sdk/go/contextgraph@<commit-sha>`
resolves to a synthetic version string like `v0.0.0-<timestamp>-<sha12>`. This
is why `go vet` / `go build` against `sdk/go` works fine in CI and for anyone
pinning a commit today (see #30's note on stella/oxagen currently doing the
equivalent for the Rust crates) — what a tag adds is a stable, human-readable
version number and `@latest` resolution, not fetchability itself.

### Post-publish verification

```bash
mkdir -p /tmp/cgp-go-smoke && cd /tmp/cgp-go-smoke
go mod init cgp-go-smoke
go get github.com/macanderson/context-graph-protocol/sdk/go/contextgraph@v0.1.0
```

A resolving `go.sum` entry (rather than a "module not found" or "no matching
versions" error) is the acceptance criterion. Then copy
`sdk/go/examples/example-docs` into the scratch module (updating its import
path to the now-external `contextgraph` package), `go build` it, and run:

```bash
./.github/scripts/conformance-external.sh -- ./cgp-go-smoke-example
```

from the repository root, proving the externally-resolved module still
produces a conformant provider.

The Go module proxy (`proxy.golang.org`) also caches the first successful
fetch of a version forever, recorded in the public checksum database
(`sum.golang.org`) — so the first `go get` after the tag is pushed is worth
doing deliberately (e.g. from this verification step) rather than leaving it
to whoever happens to try first.

## After publishing

- **Record the version in `../CHANGELOG.md`** under `[Unreleased]`, same as a
  crate release — see the entry this issue (#59) already added as the
  template.
- **Update the status table at the top of this file and in
  [`sdk/README.md`](./README.md)** from ⬜ to ✅, and drop the "not yet
  published" notes from `sdk/python/README.md` / `sdk/go/README.md`.
- **Verify the full acceptance bar from #59 end to end**: all three of
  `npm install @contextgraphprotocol/typescript-sdk`, `pip install
  contextgraph-sdk`, and `go get .../sdk/go/contextgraph@v0.1.0` resolve from
  a clean environment, and each SDK's example provider passes
  `conformance-external.sh` when run from the installed package, not the
  in-tree copy.

## This is a one-way door

- **npm**: `npm unpublish` exists but is aggressively restricted (72-hour
  window, blocked entirely if any other package depends on the version) and
  is an anti-pattern for a public SDK regardless of policy — treat a bad
  publish as needing a corrected patch version, never a retraction.
- **PyPI**: uploads cannot be overwritten or deleted. A version can only be
  *yanked* via the web UI (equivalent to `cargo yank` — hidden from new
  installs' default resolution, but still explicitly installable via `pip
  install contextgraph-sdk==<bad-version>`, so existing lockfiles that
  already pinned it keep working). Same rule: fix forward with a new version.
- **Go**: a pushed tag is technically deletable
  (`git push --delete origin sdk/go/v0.1.0`), but once `proxy.golang.org` /
  `sum.golang.org` have cached and checksummed it — which can happen within
  seconds of the tag existing, by anyone's `go get`, not just this
  maintainer's — the module version is permanently retrievable from the
  proxy regardless of what happens to the tag in this repository. Treat the
  tag push as **more** irreversible than the other two registries, not less.

This is exactly why every command above that touches a real registry or the
real tag namespace is separated from its dry-run/verification counterpart,
and why no agent or script should run `twine upload`, `npm publish`, or
`git push` of a release tag without a human deliberately choosing to.
