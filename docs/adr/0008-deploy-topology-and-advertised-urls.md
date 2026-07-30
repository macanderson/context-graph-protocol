# 0008 — Deploy topology: which host serves what, and the URLs this repo may advertise

**Status:** Accepted for the mechanical half (the invariant and its guard);
the site disposition in [Open decision](#open-decision-what-happens-to-site)
awaits the maintainer's ratification. Tracking issue:
[#57](https://github.com/macanderson/context-graph-protocol/issues/57).

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

So the apex is intentionally the microsite. This repo's `site/` — a fumadocs
app with twelve MDX pages, built on every CI run as a **hard gate** since
[#62](https://github.com/macanderson/context-graph-protocol/pull/62) — is
deployed **nowhere**.

### The consequence nobody wrote down

`site/` is not only prose. `site/public/` carries three machine-readable
artifacts that this repo hands to third parties by absolute URL:

- `schema/contextgraph-envelope.schema.json` — the schema's public identity
- `registry/contextgraph-example-docs.report.json` — the conformance report
  backing a registry row
- `badges/conformant.svg` — the badge a conformant provider pastes into its
  own README

Because `site/` deploys nowhere, every absolute URL pointing at those
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

**4. Guard, don't just document.** `scripts/check-deploy-hygiene.py` enforces
(1) and (3) offline, and runs in CI. A rule with no gate is decoration — the
same reasoning that promoted the docs-site build to a hard gate in #62.

## Open decision: what happens to `site/`

`#57`'s third acceptance criterion — "decide whether `cgp-website` content is
ported into `site/` or retired" — is a maintainer call, and this ADR does not
pre-empt it. The relevant facts, so the call can be made on evidence:

- There are **three** copies of the protocol's prose docs: `docs/*.md`
  (canonical), `site/content/docs/*.mdx` (hand-maintained copies —
  `source.config.ts` records the provenance as `../docs` but the collection is
  local), and `cgp-website/app/docs/**` (hand-written React, the one the
  public actually reads at the apex).
- Three sources of truth for the same normative prose, in two repositories, is
  the exact failure mode [ADR 0007](./0007-protocol-product-boundary.md) was
  written to end.
- `site/` is a CI hard gate with no deployment: the repo pays the build cost
  and gets no serving surface.

The two coherent resolutions:

- **Fold** — give `site/` its own Vercel project (Git-connected to this repo,
  root directory `site/`) on a stable host, and have the microsite link to it
  rather than restate it. This also restores an apex-family home for the three
  artifacts, at which point rule (1)'s URLs move off GitHub-raw and the guard's
  allowlist gains that host.
- **Retire** — delete `site/`, drop the hard gate, and let the microsite be the
  single published surface. The artifacts then need a home in `cgp-website`
  (served, or rewritten to GitHub-raw), and this repo keeps rule (1) forever.

Either is defensible; both are cheap to enact. What is *not* defensible is the
status quo, in which `site/` is built on every PR, serves nobody, and its
absence quietly breaks the URLs the spec hands out.

## Consequences

- The badge and report URLs in `docs/` and `site/content/docs/` now resolve.
- `scripts/check-deploy-hygiene.py` fails CI if a new `cgp.oxagen.sh/badges/…`
  or `contextgraphprotocol.org/schema/…` style artifact URL is introduced, and
  fails locally if the checkout is linked to the apex Vercel project.
- The `$id` interim in `schema/validate-examples.py` (#58) is no longer a
  one-off: it is the general rule, and that comment can point here.
- When the open decision lands, exactly one place changes: the served-hosts
  allowlist in `scripts/check-deploy-hygiene.py`, plus the URLs it then
  permits.
