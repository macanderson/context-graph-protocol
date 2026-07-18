# Contributing to Open Context Protocol (OCP)

Thanks for wanting to make Open Context Protocol better. This document is the whole game:
how to set up, where your change goes, what "done" means here, and how to
get it merged. It's long because it's honest — but the short version is:

## The ground rules

**Commit format** — [Conventional Commits](https://www.conventionalcommits.org),
with the crate (or surface) as scope, matching the existing history:

```text
feat(stella-model): add mistral provider adapter
fix(stella-tui): restore terminal on panic in raw mode
docs(readme): correct provider table
ci(release): sign macOS binaries
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

## Issues and labels

- **[Bug report](https://github.com/macanderson/stella/issues/new?template=bug_report.yml)** — include `stella --version`, OS, provider/model, and a repro.
- **[Feature request](https://github.com/macanderson/stella/issues/new?template=feature_request.yml)** — say what you're trying to do, not just what to add.

Labels you'll see: `area:*` routes an issue to a crate; `P0`–`P2` is priority;
`good first issue` and `help wanted` mean what they say; `needs-witness` means
a PR is waiting on its witness test.

## License

OCP is dual-licensed **MIT OR Apache-2.0**. By contributing, you agree your
contributions are licensed under the same terms, as certified by your DCO
sign-off. No CLA, no copyright assignment.

