# 34. One release tag per release train, and every published crate has one

- Status: accepted
- Date: 2026-09-28
- Tracking issue: [#103](https://github.com/macanderson/context-graph-protocol/issues/103).
  Extends [ADR 0026](0026-versions-in-prose-and-the-go-sdk-tag.md), whose
  Consequences left tag *naming* open. Repository policy; no wire or spec
  impact.

## Context

Four crate versions reached crates.io with no git tag naming them:
`0.1.0`, `0.1.1` and `0.1.2`, all published by hand on 2026-08-01. Only
`2.0.0` (2026-09-13) has one, `contextgraph-v2.0.0`. The one record of which
commit each untagged release came from is the `.cargo_vcs_info.json` that
`cargo publish` writes into every `.crate` file:

| Version | `sha1` recorded in the `.crate` | On `main`? |
| --- | --- | --- |
| 0.1.0 | `6392a22978874c7939d314224e353a4449185c16` | no |
| 0.1.1 | `53c5afeca9b62393637ef2319a6d3fc45c852608` | no |
| 0.1.2 | `9ce2a83b8cf8a0bdbd62dfdca8ff93a785948d6d` | no, but squash-merged as `00377abca163225a7efe0e2a1425fef34c95230e`, with the same tree |
| 2.0.0 | `be5edef4856d1a3387f76a7f7f64de1f691472cc` | yes, tagged `contextgraph-v2.0.0` |

(`contextgraph-trace` was first published at 0.1.1; the other three at 0.1.0.
All four crates of one version record the same commit. `1.0.0` was bumped
in-tree and never published.)

The 0.1.0 and 0.1.1 commits predate this repository's history re-root, so
nothing on `main` contains them. GitHub still serves both, but no branch or
tag holds them, so nothing guarantees it will keep doing so. `9ce2a83` is the
head of pull request #74, held only by `refs/pull/74/head`.

Meanwhile three tag conventions were in play, and none agreed:

- `ocp-v0.1.0` and `sdk/go/v0.1.0`, the only tags that existed, from before
  the rename;
- `v0.0.2`, which `MIGRATION.md` §2 once told downstreams to pin, and which
  was never cut (#30);
- `contextgraph-v*`, the one `release.yml` triggers on, which no tag matched
  until `contextgraph-v2.0.0`.

For a protocol whose pitch is "enforced by contract, not convention", a
registry artifact that cannot be traced to a reviewed commit is the gap it
exists to close, pointed at itself.

## Decision

### 1. One tag shape per release train

Each train that ships from this repository has exactly one tag shape. The
workspace convention is `contextgraph-v<semver>`.

| Train | Tag | Why this shape |
| --- | --- | --- |
| The four Rust crates, in lockstep | `contextgraph-v<X.Y.Z>` | It is the one tied to automation: `release.yml` triggers on it. The prefix keeps the crate train out of any downstream consumer's `v*` namespace, and it names the product rather than the repository, so a rename of the repository does not orphan it. |
| Go SDK (`sdk/go`) | `sdk/go/v<X.Y.Z>` | Go requires it. A nested module's tags must carry the module's directory as a prefix, and a bare `v*` tag would be read as a version of a root module that does not exist. The version is the Go SDK's own line (ADR 0026). |
| TypeScript SDK (npm) | `npm-v<X.Y.Z>` | Already what `publish-sdks.yml` verifies on. The tag starts verification only; publishing is a separate manual dispatch. |
| Python SDK (PyPI) | `pypi-v<X.Y.Z>` | As npm. |

Nothing else is a release tag. In particular there is no bare `v*` tag, and
there will not be one: it would claim the whole repository's version, which
has no single version, and Go would misread it. `ocp-v0.1.0` stays as it is.
Deleting a tag someone may have pinned breaks their build and fixes nothing;
it is history, from before the rename, and no new `ocp-*` tag is cut.

The crate train's version is the one version every `publish = true` crate
carries (they release in lockstep). The tag is `contextgraph-v` followed by
exactly that string, with no `v` inside the version and nothing after it.

### 2. A tag names the commit the registry recorded, and it is on `main`

For a release from now on, the tagged commit must be on `main`, which takes
changes only through review, and the tag must exist before the publish, since
pushing it is what starts `release.yml`. The version released is the one in
the tagged commit's manifests.

`release.yml`'s `preflight` job enforces all of this before any approval is
requested (`.github/scripts/check-release-tag.py`):

1. The tag is `contextgraph-v` + the publishing crates' shared version, and
   the crates do share one.
2. The tagged commit is an ancestor of `origin/main`.
3. That version is not already on the crates.io sparse index for any
   publishing crate. A version is published once, so a tag for a live
   version records history and starts nothing. This is what makes the
   retroactive tags in §3 safe to push, and it turns crates.io's upload-time
   rejection into a failure before a human is asked to approve. The check
   fails closed: an index it cannot read is a failure.

The publish job then checks out `github.sha`, the commit preflight checked,
rather than the tag, which could be moved while the job waits for approval.
It publishes the version preflight wrote as a job output, not one re-derived
from the tag name.

A publish by hand (PUBLISHING.md's fallback, for finishing a run that failed
partway) is done from a checkout of the tag, never from a branch.

### 3. Every version already on crates.io gets a retroactive tag

Leaving a published artifact permanently unattributed is worse than a tag
whose date is later than its name. The tag's date is when the tag was made.
The version's date is on crates.io and in the changelog. An annotated tag
says in its message that it was made afterwards, and from what evidence.

Each retroactive tag names **the commit the `.crate` file records**, not a
commit judged equivalent. That is the only choice anyone can check without
trusting this ADR: download the `.crate`, read `.cargo_vcs_info.json`, and
compare it with `git rev-parse <tag>^{commit}`. It also gives the three
commits no branch holds a ref that keeps them.

| Tag | Commit |
| --- | --- |
| `contextgraph-v0.1.0` | `6392a22978874c7939d314224e353a4449185c16` |
| `contextgraph-v0.1.1` | `53c5afeca9b62393637ef2319a6d3fc45c852608` |
| `contextgraph-v0.1.2` | `9ce2a83b8cf8a0bdbd62dfdca8ff93a785948d6d` (its tree is identical to `main`'s `00377ab`) |
| `contextgraph-v2.0.0` | exists; `be5edef4856d1a3387f76a7f7f64de1f691472cc`, as the `.crate` records |

Rule 2 of §2 does not apply to these tags, because they do not start a
release. It applies to a tag that is meant to publish. Pushing one of them
still starts `release.yml`, and preflight stops each one: 0.1.0 and 0.1.1 at
the `main` check, and all three at the registry check. That is the intended
outcome, not an error to fix.

Creating and pushing the tags is a maintainer action. The exact commands are
in `PUBLISHING.md` ("Retroactive tags for the releases before this rule").

## Consequences

- `PUBLISHING.md`, `MIGRATION.md` §2 and `sdk/PUBLISHING.md` name exactly
  the conventions in §1, and `release.yml`'s trigger is the crate train's
  tag. A self-test holds `check-release-tag.py`'s prefix and `release.yml`'s
  trigger to the same string.
- An untagged publish through the workflow is impossible: the workflow
  starts only from a tag, and preflight refuses a tag that misnames the
  version, points off `main`, or repeats a release. A publish by hand is
  still possible, as it must be for recovery. The runbook makes it start
  from the tag and says why.
- A version bump and its tag are two steps: merge the bump, then tag the
  merge commit. A tag pushed before the bump merges fails preflight, because
  the tagged commit is not on `main` yet or its manifests carry the old
  version.
- `git tag -l 'contextgraph-v*'` lists every crate version on crates.io once
  the §3 tags are pushed. Until then it lists only `contextgraph-v2.0.0`.
- The preflight registry check needs the network. It is the only rule that
  does, and it fails closed, so an outage at crates.io delays a release
  rather than letting a duplicate through to an approval click.
