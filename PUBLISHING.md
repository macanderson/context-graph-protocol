# Publishing the Context Graph Protocol crates to crates.io

This is the release process for the four public **Context Graph Protocol**
crates: `contextgraph-types`, `contextgraph-host`, `contextgraph-conformance`
and `contextgraph-trace`. They publish together, at one version, on their own
cadence, independent of any downstream consumer (such as the `stella` binary).
The workspace default is `publish = false`; these four override it (see their
`Cargo.toml`s), and every other workspace crate stays unpublished.

Published so far: 0.1.0, 0.1.1 and 0.1.2 (by hand, 2026-08-01), and 2.0.0
(2026-09-13, from the tag `contextgraph-v2.0.0`). `contextgraph-trace` was
first published at 0.1.1. 1.0.0 was bumped in-tree and never published. The
[CHANGELOG](./CHANGELOG.md) has the detail for each.

## The release tag

A crate release is named by exactly one git tag:

```
contextgraph-vX.Y.Z
```

where `X.Y.Z` is the version all four crates carry. That is the only
workspace tag convention
([ADR 0034](./docs/adr/0034-one-release-tag-per-train.md)), and it is what
[`.github/workflows/release.yml`](./.github/workflows/release.yml) triggers
on. The SDKs have their own trains and their own tag shapes, described in
[`sdk/PUBLISHING.md`](./sdk/PUBLISHING.md): `sdk/go/vX.Y.Z` (the shape Go
requires for a nested module), `npm-vX.Y.Z` and `pypi-vX.Y.Z`. There is no bare
`vX.Y.Z` tag. `ocp-v0.1.0` is from before the rename; it stays, and no new
`ocp-*` tag is cut.

**The tag comes first, and it names a commit on `main`.** Merge the version
bump through a pull request, then tag the merge commit. Never publish from a
commit that has no tag, and never tag a commit that is not on `main`. (The
retroactive tags for the releases before this rule, below, are the one
exception: they record where a past release came from, and start nothing.)

## Preferred path: the tag-triggered workflow, not a laptop

`release.yml` automates the sequence documented below, so a release is
reproducible and does not depend on whose laptop has a `cargo login` token.
Pushing the tag is what *starts* it. It does not publish anything by itself:

1. **`preflight`** runs with no secrets and no approval. It first runs
   `.github/scripts/check-release-tag.py`, which refuses the run unless:
   - the tag is exactly `contextgraph-v` + the version all four crates carry
     in the tagged commit (and they do all carry one version);
   - the tagged commit is on `main`;
   - that version is not already on crates.io for any of the four.

   Then it proves `contextgraph-types` still packages
   (`cargo publish --dry-run --locked`). A failure here costs nobody an
   approval click.
2. **`publish`** targets the `crates-io` GitHub Environment. With required
   reviewers configured (Settings → Environments), the job pauses until a
   human clicks "Approve and deploy". No approval, no publish. It checks out
   the exact commit preflight checked, not the tag, so moving the tag while
   the job waits changes nothing.
3. It runs `cargo publish` for each crate in dependency order, polling the
   sparse index between publishes (`.github/scripts/wait-for-crate.sh`) so the
   next crate's registry resolution never races the CDN.
4. `CARGO_REGISTRY_TOKEN` must exist as a secret scoped to that same
   environment, holding a crates.io API token as described in "One-time
   prerequisites" below.

To cut a release:

```bash
# After the version bump has merged to main and CI is green on the merge commit.
git fetch origin main
git checkout --detach origin/main        # or the exact merge commit of the bump
grep -m1 '^version' Cargo.toml           # must print the version you are tagging
git tag -a contextgraph-vX.Y.Z -m "contextgraph crates X.Y.Z"
git push origin contextgraph-vX.Y.Z
```

Then approve the `publish` job once `preflight` is green.

The manual sequence in "The publish sequence" below remains the reference for
exactly what that workflow executes, and the fallback if a run fails partway
(see "This is a one-way door").

## Why the order matters

```
contextgraph-types  →  contextgraph-host  →  contextgraph-conformance
                    ↘  contextgraph-trace
```

`contextgraph-host`, `contextgraph-conformance` and `contextgraph-trace` depend
on `contextgraph-types` (and `contextgraph-conformance` on `contextgraph-host`)
through a `path` dependency that also carries a `version` requirement.
crates.io strips `path` from the published manifest and only `version`
survives, so **each crate can only be published once every crate it depends
on is already live on crates.io.** Publishing out of order fails outright,
not partially.

This is also why local pre-publish verification is asymmetric:

- `contextgraph-types` has no workspace-internal deps, so
  `cargo publish --dry-run --locked -p contextgraph-types` runs the **full**
  verify (packages, resolves, compiles the packaged tarball in isolation, then
  aborts before upload). That is complete proof it is ready, and it is what CI
  and `preflight` run.
- The other three resolve `contextgraph-types` from the registry, so their
  full `--dry-run` for a *new* version can only succeed once that version of
  `contextgraph-types` is live. `cargo package -p <crate> --no-verify
  --allow-dirty --exclude-lockfile` checks their manifest shape before then.
  The one-shot co-publish below avoids the problem entirely.

## One-time prerequisites

1. A crates.io account with a verified email, holding owner rights on the four
   crates.
2. For the workflow: a crates.io API token scoped to `publish-update`, stored
   as the `CARGO_REGISTRY_TOKEN` secret on a `crates-io` GitHub Environment
   (Settings → Environments → New environment → add required reviewers, then
   add the secret scoped to it). For a by-hand run: `cargo login <token>`
   locally. Do not commit the token; no file in this repository reads it.

## The publish sequence

This is what `release.yml` runs. Run it by hand only to finish a workflow run
that failed partway, and only **from a checkout of the release tag**, never
from a branch:

```bash
git fetch origin tag contextgraph-vX.Y.Z
git checkout --detach contextgraph-vX.Y.Z
git describe --exact-match --tags HEAD   # must print contextgraph-vX.Y.Z
```

Then, from the repo root, in this order, skipping any crate already live at
`X.Y.Z`. Do not parallelize: each step's success gates the next.

```bash
# 1. contextgraph-types — the leaf, no workspace-internal deps.
cargo publish -p contextgraph-types --locked
./.github/scripts/wait-for-crate.sh contextgraph-types X.Y.Z

# 2. contextgraph-host — now resolvable, since contextgraph-types is live.
cargo publish -p contextgraph-host --locked
./.github/scripts/wait-for-crate.sh contextgraph-host X.Y.Z

# 3. contextgraph-conformance — now resolvable, since both its deps are live.
cargo publish -p contextgraph-conformance --locked

# 4. contextgraph-trace — depends only on contextgraph-types.
cargo publish -p contextgraph-trace --locked
```

`cargo publish` runs its own full verify (packages, builds in an isolated
temp dir, then uploads) before it touches the registry, so each step is
self-checking. It is still a one-way action (see below). It also records the
commit it was run from in the `.crate` file's `.cargo_vcs_info.json`, which is
how anyone can check a published version against its tag.

### One-shot alternative (cargo ≥ 1.90)

Cargo can co-publish an interdependent set in one command, computing the
dependency order and resolving the siblings through a temporary local
registry, with no manual index wait between steps:

```bash
cargo publish --locked -p contextgraph-types -p contextgraph-host -p contextgraph-conformance -p contextgraph-trace
```

Add `--dry-run` to rehearse the whole set without uploading; that packages,
resolves each sibling, and compiles all four in order.

## After publishing

- **docs.rs builds automatically** on a successful publish, typically within
  a few minutes. Check that `https://docs.rs/contextgraph-types`,
  `https://docs.rs/contextgraph-host`, `https://docs.rs/contextgraph-conformance`
  and `https://docs.rs/contextgraph-trace` render.
- **Verify from outside the workspace**: in a scratch directory,
  `cargo new /tmp/contextgraph-smoke && cd /tmp/contextgraph-smoke && cargo add
  contextgraph-types contextgraph-conformance` should resolve from the real
  registry with no path override, and a trivial conformance-suite invocation
  should pass under `cargo test`.
- **Check the provenance**: the commit in any of the new `.crate` files'
  `.cargo_vcs_info.json` must equal `git rev-parse contextgraph-vX.Y.Z^{commit}`.

## Retroactive tags for the releases before this rule

0.1.0, 0.1.1 and 0.1.2 were published by hand before
[ADR 0034](./docs/adr/0034-one-release-tag-per-train.md), and have no tag.
ADR 0034 decides to tag each one retroactively, on **the commit its `.crate`
file records** (not a commit judged equivalent), with an annotated message
that says the tag was made afterwards. `contextgraph-v2.0.0` already exists
and already names the commit its `.crate` file records.

| Tag | Commit, from `.cargo_vcs_info.json` | Note |
| --- | --- | --- |
| `contextgraph-v0.1.0` | `6392a22978874c7939d314224e353a4449185c16` | predates the history re-root; on no branch |
| `contextgraph-v0.1.1` | `53c5afeca9b62393637ef2319a6d3fc45c852608` | predates the history re-root; on no branch |
| `contextgraph-v0.1.2` | `9ce2a83b8cf8a0bdbd62dfdca8ff93a785948d6d` | head of #74; same tree as `main`'s squash-merge `00377abca163225a7efe0e2a1425fef34c95230e` |

A maintainer runs this once. Each fetch pulls a commit no branch holds, which
GitHub serves by its full id:

```bash
git fetch origin 6392a22978874c7939d314224e353a4449185c16 \
                 53c5afeca9b62393637ef2319a6d3fc45c852608 \
                 9ce2a83b8cf8a0bdbd62dfdca8ff93a785948d6d

git tag -a contextgraph-v0.1.0 6392a22978874c7939d314224e353a4449185c16 \
  -m "contextgraph crates 0.1.0 (published 2026-08-01)" \
  -m "Tagged retroactively on 2026-09-28 (ADR 0034, #103): the commit recorded in the published .crate files' .cargo_vcs_info.json. It predates the history re-root and is on no branch."
git tag -a contextgraph-v0.1.1 53c5afeca9b62393637ef2319a6d3fc45c852608 \
  -m "contextgraph crates 0.1.1 (published 2026-08-01)" \
  -m "Tagged retroactively on 2026-09-28 (ADR 0034, #103): the commit recorded in the published .crate files' .cargo_vcs_info.json. It predates the history re-root and is on no branch. Superseded by 0.1.2; do not depend on it."
git tag -a contextgraph-v0.1.2 9ce2a83b8cf8a0bdbd62dfdca8ff93a785948d6d \
  -m "contextgraph crates 0.1.2 (published 2026-08-01)" \
  -m "Tagged retroactively on 2026-09-28 (ADR 0034, #103): the commit recorded in the published .crate files' .cargo_vcs_info.json, the head of #74. Its tree is identical to main's squash-merge 00377abca163225a7efe0e2a1425fef34c95230e."

git push origin contextgraph-v0.1.0 contextgraph-v0.1.1 contextgraph-v0.1.2
```

Change the date in the messages to the day the tags are actually made.

Pushing the tags starts `release.yml` once for each, and those runs use **the
workflow file in each tagged commit**, not today's. All three commits predate
`check-release-tag.py`, so their `preflight` only dry-runs a package, and
their `publish` job then waits for approval on the `crates-io` environment.
Nothing can be published either way: crates.io refuses a version it already
has, so an approved run would fail at its first `cargo publish`. Still, do
not approve them. Cancel all three:

```bash
gh run list --workflow release.yml --limit 3 --json databaseId,headBranch \
  --jq '.[] | select(.headBranch | startswith("contextgraph-v0.1.")) | .databaseId' \
  | xargs -r -n1 gh run cancel
```

Afterwards, `git ls-remote --tags origin 'contextgraph-v*'` lists a tag for
every version on crates.io.

## This is a one-way door

crates.io does not support deleting a published version. A mistake after
publish is fixed with `cargo yank --version X.Y.Z -p <crate>` (hides it from
new dependency resolution without breaking existing lockfiles that already
reference it) followed by publishing a corrected patch version, never by
trying to overwrite what is already there. A version, once published, keeps
its tag: do not move or delete a `contextgraph-v*` tag after its release.
This is why every step above is rehearsed with `--dry-run` first, and why no
agent or script runs the real `cargo publish` or pushes a release tag without
a human deliberately choosing to.

---

# Publishing the specification and schemas to contextgraphprotocol.org

Separate from the crates above, and automatic:
[`.github/workflows/publish-spec.yml`](./.github/workflows/publish-spec.yml)
runs on every push to `main` that touches `schema/`, `docs/` or `SPEC.md`, and
puts them on the microsite's CDN.

| Source | URL |
| --- | --- |
| `schema/*.json` | `https://contextgraphprotocol.org/schema/v1/…` — **the identity** |
| `schema/*.json` | `https://contextgraphprotocol.org/schema/…` — unversioned alias |
| `schema/reference-vectors.ndjson` | `https://contextgraphprotocol.org/schema/v1/reference-vectors.ndjson` — beside the identity |
| `schema/reference-vectors.ndjson` | `https://contextgraphprotocol.org/schema/reference-vectors.ndjson` — unversioned alias |
| `SPEC.md` | `https://contextgraphprotocol.org/spec/SPEC.md` |
| `docs/**` | `https://contextgraphprotocol.org/spec/docs/…` |

## This job carries the schemas' identity

It did not until #79. Each schema's `$id` names
`https://contextgraphprotocol.org/schema/v1/<name>`
([ADR 0013](./docs/adr/0013-schema-identity-on-a-branded-versioned-url.md)), so
what this job publishes is what every validator resolves — not a mirror. A
merge that fails to serve it fails the job.

`v1` is the `contextgraph/1` **major protocol family**, not the crate version
(already `2.x` against that same wire — see
[docs/stability.md](./docs/stability.md)). `contextgraph/2` would be published
at `/schema/v2/`, and `/schema/v1/` would keep answering.

Three paths serve the same two files, and all three must keep working:

- `/schema/v1/<name>` — the identity.
- `/schema/<name>` — the unversioned path published since #78. In the wild, so
  it keeps being published. It is a convenience alias, never an identity.
- `raw.githubusercontent.com/…/main/schema/<name>` — the former identity, still
  quoted. Nothing publishes it: GitHub serves it as long as the file stays put.
  **So `schema/*.schema.json` must never be moved or renamed.**

Only one copy of each schema exists in the repository. There is no `schema/v1/`
directory; the publisher writes the same bytes to both prefixes, so there is
nothing to keep in sync.

The reference vectors follow the schemas onto both prefixes (#111). They are
what the reference Rust types serialize for the `contextgraph/1` wire, so they
are part of that family's contract. Someone who fetches the schema from
`/schema/v1/` finds the vectors in the same place. They carry no `$id`, so
the workflow checks them differently: each path must return the committed
bytes as `application/x-ndjson`. That type is set explicitly, because S3
would otherwise serve the file as `application/octet-stream` and browsers
would download it. See the amendment to
[ADR 0013](./docs/adr/0013-schema-identity-on-a-branded-versioned-url.md).

## How identity is checked, in two halves

`schema/validate-examples.py` runs first, in this workflow rather than only in
`ci.yml`. Reading another workflow's result would need a `workflow_run` trigger,
whose failure mode is publishing anyway when the dependency is skipped — and the
distinction that matters is between "the schema validated somewhere" and "the
bytes about to be published validated".

That script is **offline**: it pins the `$id` string and does not fetch it. It
runs on every PR, forks included, against commits whose publish has not
happened, so a fetch there would be flaky rather than informative.

The dereference is this workflow's last check instead, after the sync and the
CDN invalidation: it fetches every published path and asserts the served body
reports the identity as its own `$id`. A 200 is not enough on its own — a static
site answers its 404 page with one — and neither is "the body parses as the
schema", which would accept a stale object left at an alias path.

## Two things the site's own repository depends on

The microsite (`macanderson/cgp-website`) is built from a different repository
into the *same* bucket. Two arrangements keep them from overwriting each other,
and both are load-bearing:

- Its deploy excludes `schema/` and `spec/` from a `--delete` sync. Without
  that, it would remove everything this workflow publishes, silently — deleting
  a file the sync did not expect is not an error.
- This repository's AWS role can write **only** those two prefixes, and the
  site's role is explicitly denied them. Either arrangement being dropped fails
  a deploy loudly rather than corrupting the other repository's output.

## Retiring a schema is manual, on purpose

The schema upload carries no `--delete`. A published schema URL is a contract
other implementations resolve, so removing one is a breaking change and must not
be something a rename in this repository does silently on merge. Retiring one is
an `aws s3 rm` with the argument made out loud. The `spec/` upload *does* use
`--delete`, because those are documents and renaming them is ordinary editing.

## One-time setup

- **`production` environment.** The AWS role trusts exactly the subject
  `repo:macanderson/context-graph-protocol:environment:production`. There is no
  stored AWS key; without the environment the credential exchange fails. That
  subject names the environment, not a branch, so the environment restricts who
  can publish only together with a **deployment-branch policy limiting
  `production` to `main`** (Settings → Environments → production → Deployment
  branches and tags → Selected branches → add `main`). Configure it; it was
  absent when checked on 2026-09-22
  ([#202](https://github.com/macanderson/context-graph-protocol/issues/202)).
  Confirm with `gh api repos/macanderson/context-graph-protocol/environments/production`:
  `deployment_branch_policy` is set and the only custom branch is `main`. The
  publish job also runs only when `github.ref` is `refs/heads/main`, so a
  dispatch from another branch that carries the workflow as reviewed skips it
  even without the policy; a branch that edits that condition away is what the
  policy is for.
- **`SITE_DISPATCH_TOKEN` secret.** The last step asks the microsite to rebuild,
  because its rendered documentation quotes this specification. Writing to
  another repository is something the job's own `GITHUB_TOKEN` cannot do by
  design, so this needs a fine-grained PAT with `Contents: read-write` on
  `macanderson/cgp-website`. **Until it exists the step warns and the job still
  succeeds** — the schemas and spec are published either way; only the site
  rebuild waits for `cgp-website`'s next own merge.
