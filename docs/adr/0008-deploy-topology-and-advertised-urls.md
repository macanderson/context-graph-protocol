# 0008 — Deploy topology: which host serves what, and the URLs this repo may advertise

**Status:** Accepted. Ratified 2026-07-30 — the maintainer's call on
[the `site/` question](#the-site-question-retire) was **retire**, and this ADR
records the topology that results. Tracking issue:
[#57](https://github.com/macanderson/context-graph-protocol/issues/57).

**Amended 2026-08-22 by
[#78](https://github.com/macanderson/context-graph-protocol/pull/78).** Rule
(1) stands unchanged; the fact it rested on does not. "Today it deploys none"
was true when this was ratified and is no longer: `publish-spec.yml` syncs
`schema/` to `contextgraphprotocol.org/schema/` and `SPEC.md`/`docs/**` to
`/spec/` on every merge to `main`. Those two prefixes are now hosts-this-repo-
serves for the purpose of rule (1), and nothing else on that apex is — the
badge and registry URLs stay on `raw.githubusercontent.com`.

**Amended again 2026-08-29 by
[ADR 0013](./0013-schema-identity-on-a-branded-versioned-url.md)
([#79](https://github.com/macanderson/context-graph-protocol/issues/79)).**
Rule (1) still stands, and is what *permits* the change rather than resisting
it. What no longer holds is this ADR's conclusion for the schemas — that
GitHub-raw is the only host it can honestly advertise, called permanent in the
Consequences below. That was sound while this repository published nothing;
after the amendment above it is not. Each schema's `$id` now names
`contextgraphprotocol.org/schema/v1/…`, a prefix this repository serves, and
the raw URL keeps resolving as the former identity. The badge and the registry
report are unaffected and stay on raw.

The guard is
keyed on the prefix, not the host, so it still rejects
`contextgraphprotocol.org/badges/conformant.svg`, which is the 404 this ADR
was written about.

## Context

This repository has, at various times, believed it deploys the domains it
advertises. It does not, and the gap has been shipping broken URLs.

### What is actually true (verified 2026-07-30)

One Vercel project, `context-graph-protocol`
(`prj_s3lfCDvK9H9PwgvkpXiho1juiR63`, team `oxagen`), owns every host in the
family:

| Host | Served by |
| --- | --- |
| `contextgraphprotocol.org` (+ `www.` 308) | `macanderson/cgp-website` |
| `cgp.oxagen.sh` | `macanderson/cgp-website` |
| `context-graph-protocol.vercel.app` (+ team aliases) | `macanderson/cgp-website` |

That project's Git integration points at **`macanderson/cgp-website`**, root
directory `.`, production branch `main`. It is **not** connected to this
repository. The changeover is visible in the project's deploy history:

- `2026-07-23T21:41:03Z` — last production deploy sourced from
  `context-graph-protocol` (`abaebc7`).
- `2026-07-23T21:51:53Z` — production deploy from `cgp-website@main`
  (`8958d2e`). **Still the live production deployment.**
- Everything since is a `cgp-website` preview. Pushes to this repository
  produce no deployments on the project at all.

So the apex is intentionally the microsite. This repo also carried `site/` — a
fumadocs app with twelve MDX pages, built on every CI run as a **hard gate**
since [#62](https://github.com/macanderson/context-graph-protocol/pull/62) —
deployed **nowhere**. (It is retired by this ADR; the sections below describe
the state that prompted the decision.)

### The consequence nobody wrote down

`site/` was not only prose. `site/public/` carried three machine-readable
artifacts that this repo hands to third parties by absolute URL:

- `schema/contextgraph-envelope.schema.json` — the schema's public identity
- `registry/contextgraph-example-docs.report.json` — the conformance report
  backing a registry row
- `badges/conformant.svg` — the badge a conformant provider pastes into its
  own README

Because `site/` deployed nowhere, every absolute URL pointing at those
artifacts 404s. Verified 2026-07-30:

```
https://contextgraphprotocol.org/schema/contextgraph-envelope.schema.json  404
https://contextgraphprotocol.org/registry/…report.json                     404
https://cgp.oxagen.sh/badges/conformant.svg                                404
```

The last one is the sharpest: `docs/registry.md` and
`docs/implementing-a-provider.md` (and their `site/content/docs/` mirrors)
instruct provider authors to paste that exact URL into their READMEs. Every
author who followed the instruction got a broken image, on a page whose own
copy claims the badge "never depends on this site's uptime."

[#58](https://github.com/macanderson/context-graph-protocol/issues/58) already
hit the same wall from the schema side and worked around it by pointing `$id`
at this repo's GitHub-raw URL. That workaround was correct, and it was applied
to exactly one of the three artifacts.

## Decision

**1. A host boundary, stated once.** This repository may advertise an
absolute URL for one of its own static artifacts *only* on a host it
deploys. Today it deploys none, so every artifact URL resolves through
`raw.githubusercontent.com/macanderson/context-graph-protocol/main/…` — the
one host that serves this repo's bytes by construction, regardless of how the
Vercel topology is settled.

`raw.githubusercontent.com` serves the badge as `image/svg+xml`, so it renders
in a third-party README exactly as the `cgp.oxagen.sh` URL was supposed to.

This is generalising #58's fix from the schema to all three artifacts, and
writing down the rule that made it necessary.

**2. Prose links are not artifact links.** A human-facing link to the
*protocol homepage* is fine on the apex — the microsite serves it and returns
200. Those links should name the canonical apex (`contextgraphprotocol.org`,
matching `Cargo.toml`'s `homepage` and the README), not `cgp.oxagen.sh`.

**3. This repository must never be `vercel link`ed to the apex project.**
`.vercel/` is gitignored, so a stray local link is invisible to review — but a
`vercel --prod` from such a checkout deploys *this* repo over the microsite's
production and takes the apex down to a fumadocs site with no warning. That is
precisely how the 2026-07-23 flip-flop happened, in both directions.

**4. Guard, don't just document.** `.github/scripts/check-deploy-hygiene.py` enforces
(1) and (3) offline, and runs in CI. A rule with no gate is decoration — the
same reasoning that promoted the docs-site build to a hard gate in #62.

## The `site/` question: retire

`#57`'s third acceptance criterion — "decide whether `cgp-website` content is
ported into `site/` or retired" — was put to the maintainer with the evidence
below. **The decision is retire**, and this PR enacts it.

The evidence:

- There were **three** copies of the protocol's prose: `docs/*.md`
  (canonical), `site/content/docs/*.mdx` (hand-maintained copies —
  `source.config.ts` recorded the provenance as `../docs` but the collection
  was local), and `cgp-website/app/docs/**` (hand-written React, the one the
  public actually reads at the apex). Three sources of truth for the same
  normative prose, across two repositories, is the exact failure mode
  [ADR 0007](./0007-protocol-product-boundary.md) was written to end.
- The copies had already drifted, and drifted **stale**, not richer:
  `site/content/docs/protocol-surface.mdx` still documented
  `capabilities.upsert`, `.subscribe`, `.filters`, and `.writes` — the dead
  surface [ADR 0004](./0004-dead-capability-surface.md) removed —
  and `changelog.mdx` was 87 lines against `CHANGELOG.md`'s 353. Four more
  pages (`changelog`, `contributing`, `governance`, `security`) were partial
  copies of root files with no `docs/` counterpart at all. Deleting the app
  therefore removed *wrong* documentation, not unique documentation.
- `site/` was a CI **hard gate** with no deployment: every PR paid a
  `pnpm install && next build` and got no serving surface in return.

Enacting it meant relocating the two artifacts that were genuinely published
out of `site/public/` before deleting the app — `assets/badges/conformant.svg`
and `registry/contextgraph-example-docs.report.json`. The third,
`site/public/schema/…`, was a byte-identical mirror of `schema/…` and simply
went away, along with the copy-sync check that guarded it.

The alternative was **fold** — give `site/` its own Vercel project on a stable
host and have the microsite link out rather than restate. It was rejected as
the more expensive way to end up with two websites and the same drift risk,
when the microsite already serves the audience.

## Consequences

- The badge and report URLs in `docs/` resolve, and now point at paths
  (`assets/`, `registry/`) that no longer depend on an app that might be
  deleted.
- `.github/scripts/check-deploy-hygiene.py` fails CI if a new
  `cgp.oxagen.sh/badges/…` or `contextgraphprotocol.org/schema/…` style
  artifact URL is introduced, or if an advertised artifact does not exist at
  the path its URL names, and fails locally if the checkout is linked to the
  apex Vercel project.
- Rule (1) is **permanent**, not interim. Which host it *resolves to* for a
  given artifact is not: the rule turns on the set of hosts this repo serves,
  and that set grew in #78. So the clause that used to close this bullet —
  GitHub-raw is the only host it can honestly advertise — is superseded for the
  schemas by [ADR 0013](./0013-schema-identity-on-a-branded-versioned-url.md).
  It still holds for the badge and the registry report, which this repository
  publishes nowhere else.
- CI loses the `docs site builds` job and `tests/docs_site_witness_test.py`.
  The protocol's prose is now single-sourced in `docs/*.md`, read on GitHub.
- `cgp-website` is the protocol's only website. Serving one of these artifacts
  from the apex takes a publishing path plus one row in `SERVED_PREFIXES` in
  the hygiene check. (This bullet said `SERVED_HOSTS`; #78 replaced it with a
  prefix map, because a bare host entry would re-bless the `/badges/` URL this
  guard exists to catch.)
