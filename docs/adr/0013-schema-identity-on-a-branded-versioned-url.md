# 0013 — Schema identity: a branded URL, versioned by major family

**Status:** Accepted. Tracking issues:
[#79](https://github.com/macanderson/context-graph-protocol/issues/79),
[#58](https://github.com/macanderson/context-graph-protocol/issues/58).

**Amends [ADR 0008](./0008-deploy-topology-and-advertised-urls.md).** Rule (1)
of that ADR — advertise an artifact URL only on a host this repo serves —
stands unchanged and is what permits this move. Its *conclusion* for the
schemas ("GitHub-raw is the only host it can honestly advertise") rested on a
fact that [#78](https://github.com/macanderson/context-graph-protocol/pull/78)
retired.

Each schema's `$id` becomes:

```
https://contextgraphprotocol.org/schema/v1/contextgraph-envelope.schema.json
https://contextgraphprotocol.org/schema/v1/contextgraph-lifecycle-record.schema.json
```

## Context

`$id` is not a download link. It is the schema's identity — the base URI for
`$ref` resolution and the string a third party pins, caches, and quotes when it
says which contract it validates against. It gets to be wrong in ways an
ordinary URL does not.

This one has been wrong twice, in different ways, and the second fix has now
expired:

1. `context-graph-protocol.org` — hyphenated, never registered. Every consumer
   that tried to dereference it got a DNS failure (#58).
2. `raw.githubusercontent.com/macanderson/context-graph-protocol/main/schema/…`
   — correct when it was chosen (ADR 0008): the apex was served by a different
   repository that carried nothing under `/schema/`, so naming the apex would
   have swapped one unreachable URL for another.

#78 changed the fact underneath (2). `publish-spec.yml` now syncs `schema/` to
`contextgraphprotocol.org` on every merge to `main`, under a prefix this
repository owns and the microsite's deploy is denied write on. The apex is a
host this repo serves, for that prefix. ADR 0008's rule permits the move; ADR
0008's conclusion no longer follows from it.

Two defects remain in the raw URL beyond the host:

- **It is a code-hosting domain, not the protocol's name.** An identity a third
  party quotes for a decade should say what it identifies.
- **It pins `main`, a git branch.** A branch is unbounded change under a stable
  name. A `1.x` additive minor already changes what a resolver holding that URL
  sees, with nothing to signal it. The protocol has no concept of `main`; that
  segment names this repository's default branch, an implementation detail of
  where the bytes happen to live.

## Decision

**1. The host is `contextgraphprotocol.org`** — the protocol's own name, and a
prefix this repository publishes.

**2. The path is versioned by *major protocol family*, not by minor and not by
`main`.** `v1` denotes `contextgraph/1`, the wire family
(`contextgraph_types::PROTOCOL_VERSION`). It is emphatically **not** the crate
version, which is already `2.x` against that same `contextgraph/1` wire — the
two axes move independently and today they disagree
([docs/stability.md](../stability.md)). Anyone reading `v1` as "version 1 of the
crates" has misread it, which is why this paragraph exists.

`contextgraph/2` will be published at `/schema/v2/`, and `/schema/v1/` will
keep answering for as long as anyone resolves it.

**3. The two URLs already in the wild keep resolving, forever.**

| Path | Role |
| --- | --- |
| `/schema/v1/<name>` | the identity; what `$id` names and validators resolve |
| `/schema/<name>` | unversioned alias, published since #78 — a convenience, never an identity |
| `raw.githubusercontent.com/…/main/schema/<name>` | the former identity, still quoted |

The raw URL needs no publishing step: GitHub serves it as long as the file
stays at `schema/<name>`. That turns "keep the old identity alive" into a
single concrete constraint — **never move or rename `schema/*.schema.json`** —
which is easier to honour than a redirect and cannot silently lapse.

**4. One copy in the repository.** `schema/v1/` does not exist as a directory.
The publisher writes the same two files to both prefixes, and
`check-deploy-hygiene.py` maps both URL prefixes back to `schema/`. A second
checked-in copy would be a drift hazard for no gain — and this repository has
already paid for that once, in the three copies of the prose ADR 0008 retired.

## Why not per-minor paths (`/schema/1.2/…`)

This is the alternative #79 raises, and it is the wrong granularity.

Within `contextgraph/1`, evolution is **additive-only** — that is a governance
guarantee, not a habit ([GOVERNANCE.md](../../GOVERNANCE.md), SPEC §13 U1–U4).
A consumer holding a cached `1.0`-era copy of the schema is therefore never
*wrong*, only less complete: every field it knows still means what it meant, and
the members it has not heard of are ones U1 already requires it to ignore. The
staleness a per-minor path would protect against is staleness that cannot hurt.

Against that, per-minor costs: a new published path every minor, a new `$id`
every minor, and a consumer's pin going stale by design each time. Versioning
one notch coarser than the thing that actually breaks compatibility buys
churn.

Family granularity is the notch where compatibility genuinely changes, and it
is the axis the protocol already versions on — so the URL now agrees with
`versions_compatible`, which compares exactly this prefix.

## Why `$ref` resolution is unaffected

`$id` sets the base URI for relative `$ref`s, so moving it is the kind of change
that can break resolution silently. Here it cannot, and this was checked rather
than assumed: **every `$ref` in both schemas is a same-document JSON pointer**
(`#/$defs/…`), and **neither schema references the other**.

```
$ jq -r '[.. | objects | select(has("$ref")) | .["$ref"]] | unique[]' \
    schema/contextgraph-envelope.schema.json schema/contextgraph-lifecycle-record.schema.json
```

returns only `#/$defs/…` entries. A same-document pointer resolves against
whatever the document's base is, so it is correct under the old `$id`, the new
one, a `file://` path, and no `$id` at all. Both schemas remain resolvable fully
offline from a local copy, which is how the conformance suite and
`validate-examples.py` read them.

## The ordering constraint, and how it is met

An identity must not name a URL that 404s. That makes the order rigid: publish
the path, then flip `$id`. The two halves land in one PR, and the workflow's own
step order is what enforces it — the `publish` job syncs `/schema/v1/`,
invalidates the CDN, and only then dereferences each `$id` and asserts the
served body reports that same `$id`. A merge that fails to serve the identity
fails the job loudly.

The offline validator is deliberately **not** where that is checked.
`schema/validate-examples.py` runs on every PR, forks included, and against
commits whose publish has not happened; making it fetch would trade a real
guarantee for a flaky one. It pins the identity string; the publisher proves the
identity answers. Neither check is weakened to accommodate the other.

## Consequences

- Every validator that resolves `$id` now fetches from the protocol's own
  domain, under a path stable for the life of `contextgraph/1`.
- **No action is required of any implementer.** The bytes are identical at all
  three URLs, `$ref` resolution is unchanged, and the old URLs keep answering.
  A pinned local copy of either schema stays valid.
  ([MIGRATION.md](../../MIGRATION.md) says so where implementers look.)
- `schema/*.schema.json` may never be moved or renamed — that path *is* the
  old identity's hosting. A rename is a breaking change to a URL in the wild.
- `check-deploy-hygiene.py` gained the `/schema/v1/` prefix row, longest-prefix
  matching (two rows now nest), and a URL pattern that admits a version
  segment. The last of those was a live gap: the previous pattern required the
  filename to sit directly under `schema/`, so it matched no versioned URL at
  all and would have gone silently blind to the two `$id`s it exists to police.
- A future `contextgraph/2` adds `/schema/v2/` and one publisher line. It does
  not disturb v1.
- Should the lifecycle-record profile ever leave draft on a release cadence of
  its own, it earns its own path segment. It shares `v1` today because the
  profile is layered *on* the `contextgraph/1` base family rather than versioned
  against it; changing that is a decision made out loud, not a drift.
