# Contributing to Context Graph Protocol

Thanks for wanting to make Context Graph Protocol (CGP) better. This document is the whole game:
how to set up, where your change goes, what "done" means here, and how to
get it merged. It's long because it's honest — but the short version is:

## The ground rules

**Commit format** — [Conventional Commits](https://www.conventionalcommits.org),
with the crate or surface as scope, matching the existing history:

```text
feat(contextgraph-host): add fan-out timeout for slow providers
fix(contextgraph-conformance): restore correct exit code on budget-honesty failure
docs(readme): clarify dual-license statement
ci(release): publish contextgraph-types to crates.io
```

**DCO, not CLA.** Sign every commit (`git commit -s`) to certify the
[Developer Certificate of Origin](https://developercertificate.org/). You keep
your copyright; no assignment, ever.

**PR checklist** (the template walks you through it):

1. One logical change per PR — smaller lands faster.
2. The gate is green locally (`fmt` / `clippy -D warnings` / `test`).
3. A witness test, or a stated reason there isn't one.
4. Docs updated in the same PR if behavior or flags changed (`README.md`,
   `--help` text, doc comments).
5. Commits signed off (`-s`).

Maintainers aim for a first response within a few days. "Needs work" is a
normal part of the loop here, not a rejection.

## Sites and URLs (read before you `vercel` anything)

**This repository deploys nothing, by decision.** `contextgraphprotocol.org`,
`cgp.oxagen.sh`, and `context-graph-protocol.vercel.app` are all served by one
Vercel project that is Git-connected to a *different* repository
(`macanderson/cgp-website`), which is the protocol's single published website.
This repo once carried a second, undeployed docs site under `site/`; it was
retired in favour of the canonical Markdown in `docs/` (#57,
[ADR 0008](./docs/adr/0008-deploy-topology-and-advertised-urls.md)). Don't add
another one — publish prose to `docs/`, and published assets to `assets/`,
`schema/`, or `registry/`.

Two rules follow, both enforced by
`python3 .github/scripts/check-deploy-hygiene.py` (a required CI check — run it
locally before you push):

1. **Never `vercel link` this checkout to the apex project.** `.vercel/` is
   gitignored, so a stray link is invisible in review — and one `vercel --prod`
   from a linked checkout replaces the public apex with this repo's docs site.
   That has already happened, in both directions.
2. **Advertise artifact URLs only on a host this repo serves.** Today that is
   `raw.githubusercontent.com/macanderson/context-graph-protocol/main/…` and
   nothing else. A `contextgraphprotocol.org/schema/…` or
   `cgp.oxagen.sh/badges/…` URL looks canonical and 404s. Prose links to the
   protocol homepage are fine on the apex — it serves those.

## Issues and labels

- **[Bug report](https://github.com/macanderson/context-graph-protocol/issues/new?template=bug_report.yml)** — include the CGP crate name and version, OS, and a repro.
- **[Feature request](https://github.com/macanderson/context-graph-protocol/issues/new?template=feature_request.yml)** — say what you're trying to do, not just what to add.

Labels you'll see: `area:*` routes an issue to a crate; `P0`–`P2` is priority;
`good first issue` and `help wanted` mean what they say; `needs-witness` means
a PR is waiting on its witness test.

## License

CGP is dual-licensed **MIT OR Apache-2.0**. By contributing, you agree your
contributions are licensed under the same terms, as certified by your DCO
sign-off. No CLA, no copyright assignment.

This project follows the [Code of Conduct](./CODE_OF_CONDUCT.md). By
participating you are expected to uphold it.

