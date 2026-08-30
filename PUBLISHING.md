# Publishing the Context Graph Protocol crates to crates.io

This documents the release process for the three **Context Graph Protocol**
crates — `contextgraph-types`, `contextgraph-host`, `contextgraph-conformance` — to crates.io. These
crates are published independently of any downstream consumer (such as the
`stella` binary), on their own cadence.

**Nobody has run these publish commands yet.** The workspace default is
`publish = false`; the three Context Graph Protocol crates override it explicitly (see their
`Cargo.toml`s). This file exists so the *first* real publish is a checklist,
not an improvisation.

## Preferred path: the tag-triggered workflow, not a laptop

[`.github/workflows/release.yml`](./.github/workflows/release.yml) automates
the exact sequence documented below, so a release is reproducible and doesn't
depend on whoever's laptop has a `cargo login` token on it. Pushing a
`contextgraph-vX.Y.Z` tag is what *starts* it — it does not publish anything
by itself:

1. The workflow's `publish` job targets the `crates-io` GitHub Environment. If
   that environment has required reviewers configured (Settings →
   Environments), the job pauses there until a human clicks "Approve and
   deploy." No approval, no publish.
2. It then runs `cargo publish` for each crate in dependency order, polling
   the sparse index between publishes (`.github/scripts/wait-for-crate.sh`)
   so the next crate's registry resolution never races the CDN — the same
   "wait for the index" step called out by hand below, just automated.
3. `CARGO_REGISTRY_TOKEN` must exist as a secret scoped to that same
   environment, holding a crates.io API token as described in "One-time
   prerequisites" below.

Both the `crates-io` environment and its secret are one-time, human,
repo-Settings setup — **neither exists yet** as of this writing. Until they
do, the workflow exists but cannot run: a tag push just sits there with the
job queued for an environment that has no approver configured, which is a
safe failure mode, not a silent one.

The manual sequence in "The publish sequence" below remains the documented
reference for exactly what that workflow executes step-by-step, and is the
fallback if a release needs manual intervention partway through (see "This is
a one-way door").

## Why the order matters

```
contextgraph-types  →  contextgraph-host  →  contextgraph-conformance
```

`contextgraph-host` depends on `contextgraph-types` via `{ path = "../contextgraph-types", version =
"0.1.0" }`; `contextgraph-conformance` depends on both `contextgraph-types` and `contextgraph-host` the
same way. crates.io rejects a publish whose dependencies aren't already
resolvable from the registry — `path` is stripped from the published
manifest and only `version` survives, so **each crate can only be published
once every crate below it in the chain is already live on crates.io.**
Publishing out of order fails outright, not partially.

This is also why local pre-publish verification is asymmetric:

- `contextgraph-types` has no workspace-internal deps, so
  `cargo publish --dry-run -p contextgraph-types` runs the **full** verify (packages,
  resolves, compiles the packaged tarball in isolation, then aborts before
  upload) — this is complete proof it's ready.
- `contextgraph-host` and `contextgraph-conformance` depend on a crate (`contextgraph-types`) that
  genuinely isn't on crates.io yet, so `cargo package`/`cargo publish
  --dry-run` for them cannot resolve the registry entry for `contextgraph-types`
  locally — that's not a bug in this checklist, it's crates.io index
  resolution working as designed. The correct pre-publish proof for those
  two is `cargo package -p <crate> --no-verify --allow-dirty
  --exclude-lockfile` (packages and validates the manifest shape without
  needing the registry lockfile) plus manual inspection of the generated
  `Cargo.toml` inside the `.crate` tarball to confirm the `version` fields
  landed. Full `--dry-run` verification for `contextgraph-host` and `contextgraph-conformance`
  only becomes possible *after* their dependencies are actually published.

## One-time prerequisites

1. A crates.io account with a verified email, linked to a GitHub account with
   write access to `macanderson/context-graph-protocol` (or another account
   willing to transfer ownership to the `macanderson` GitHub org's crates.io
   team once one exists).
2. `cargo login <token>` locally, using a crates.io API token scoped to
   `publish-new` + `publish-update` (crates.io Account Settings → API
   Tokens). Do not commit this token; it's not an env var this repo reads.
   For the tag-triggered workflow instead of a laptop, the same kind of
   token is stored as the `CARGO_REGISTRY_TOKEN` secret on a `crates-io`
   GitHub Environment (Settings → Environments → New environment → add
   required reviewers, then add the secret scoped to it) rather than run
   through `cargo login` anywhere.
3. Confirm the crate names are still unclaimed: check
   `https://crates.io/crates/contextgraph-types`, `.../contextgraph-host`, `.../contextgraph-conformance`
   — a 404 on each means the name is free. (As of writing, all three are
   unclaimed.)

## The publish sequence

Run every command from the repo root, in this exact order. Do not
parallelize — each step's success gates the next.

```bash
# 1. contextgraph-types — the leaf, no workspace-internal deps.
cd contextgraph-types
cargo publish
cd ..

# Wait for the crates.io index to pick it up. Usually seconds, occasionally
# a minute or two behind the sparse index CDN. Confirm before proceeding:
cargo search contextgraph-types   # or just check https://crates.io/crates/contextgraph-types

# 2. contextgraph-host — now resolvable, since contextgraph-types is live.
cd contextgraph-host
cargo publish
cd ..
cargo search contextgraph-host

# 3. contextgraph-conformance — now resolvable, since both its deps are live.
cd contextgraph-conformance
cargo publish
cd ..
cargo search contextgraph-conformance
```

`cargo publish` runs its own full verify (packages, builds in an isolated
temp dir, then uploads) before it ever touches the registry, so each step is
self-checking — but it's still a one-way action (see below).

### One-shot alternative (cargo ≥ 1.90)

Modern cargo can co-publish an interdependent set in one command, computing
the dependency order and resolving the siblings through a temporary local
registry — no manual index-wait between steps:

```bash
cargo publish -p contextgraph-types -p contextgraph-host -p contextgraph-conformance
```

Add `--dry-run` to rehearse the whole set without uploading; that dry-run is
the definitive publishability proof used to validate this checklist (it
packages, resolves each sibling, and compiles all three in order). Prefer the
explicit three-step sequence above if you want to eyeball each crate landing
on crates.io before the next goes up.

## After publishing

- **docs.rs builds automatically** on a successful publish, typically within
  a few minutes. Check `https://docs.rs/contextgraph-types`,
  `https://docs.rs/contextgraph-host`, `https://docs.rs/contextgraph-conformance` render
  cleanly — the `documentation` field in each `Cargo.toml` already points
  there.
- **Verify the acceptance criterion end to end**: in a scratch directory
  *outside* this workspace, `cargo new /tmp/contextgraph-smoke && cd /tmp/contextgraph-smoke
  && cargo add contextgraph-types contextgraph-conformance` should resolve from the real
  registry with no path override, and `cargo test` (after writing a trivial
  conformance-suite invocation) should pass — proving "an external crate can
  depend on `contextgraph-types` and pass `contextgraph-conformance` without
  vendoring any downstream code" (the issue's acceptance bar) against the
  *published* crates, not just the workspace.
- Tag the release in this repo for traceability, e.g. `contextgraph-v0.1.0`. Use
  the `contextgraph-` tag prefix so the crate release train never collides with a
  downstream consumer's own version tags in the tag namespace. **If publishing
  by hand, this happens last** — after the fact, for traceability. If using
  `release.yml` instead, the order inverts: pushing this same tag is what
  starts the workflow, so it happens *first*, before any crate is live.

## This is a one-way door

crates.io does not support deleting a published version. A mistake after
publish is fixed with `cargo yank --version 0.1.0 -p contextgraph-types` (hides it
from new dependency resolution without breaking existing lockfiles that
already reference it) followed by publishing a corrected patch version —
never by trying to overwrite or delete what's already there. This is exactly
why every command above was verified with `--dry-run` / `--no-verify
--exclude-lockfile` first, and why no agent or script should run the real
`cargo publish` without a human deliberately choosing to.

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
| `schema/reference-vectors.ndjson` | `https://contextgraphprotocol.org/schema/reference-vectors.ndjson` |
| `SPEC.md` | `https://contextgraphprotocol.org/spec/SPEC.md` |
| `docs/**` | `https://contextgraphprotocol.org/spec/docs/…` |

`schema/validate-examples.py` runs first, in this workflow rather than only in
`ci.yml`. Reading another workflow's result would need a `workflow_run` trigger,
whose failure mode is publishing anyway when the dependency is skipped — and the
distinction that matters is between "the schema validated somewhere" and "the
bytes about to be published validated".

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
  stored AWS key; without the environment the credential exchange fails.
- **`SITE_DISPATCH_TOKEN` secret.** The last step asks the microsite to rebuild,
  because its rendered documentation quotes this specification. Writing to
  another repository is something the job's own `GITHUB_TOKEN` cannot do by
  design, so this needs a fine-grained PAT with `Contents: read-write` on
  `macanderson/cgp-website`. **Until it exists the step warns and the job still
  succeeds** — the schemas and spec are published either way; only the site
  rebuild waits for `cgp-website`'s next own merge.
