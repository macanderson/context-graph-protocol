# ADR 0020: Standing decisions live in the oxagen workspace, not a local copy

- Status: accepted
- Date: 2026-09-22
- Supersedes: the replicated-corpus placement in [ADR 0009](0009-adopt-standing-decisions-scr-corpus.md)
- Related: oxagen ADR-137

## Context

ADR 0009 replicated `docs/scr/` into this repository so five org repos would
carry one byte-identical corpus. Oxagen ADR-137 retired that placement on
2026-09-22. The six standing decisions are context records under
`.oxagen/rules/` in the oxagen repository, with `sharing_scope = workspace`.
This repository's `.oxagen/workspace.toml` declares `role = "linked"` to the
`oxagen/product` workspace. `scr-corpus-check` no longer compares copies. It
fails when any of the five repositories still has a file under `docs/scr/`.

Keeping the directory, or adding SCR-006 to it, makes that check worse. The
records are not documents to copy. A connected repository is steered from the
workspace.

## Decision

`docs/scr/` is removed from this repository. The standing-decisions block in
`AGENTS.md` stays, because the harnesses that work here read that file, and
each bullet links the oxagen record it summarizes. This repository does not
keep a second copy of the record.

The SCR-006 summary states the fact that applies here: this repository has
no persistent store, so the record is inert and `migration-required` is never
applied. A change to `schema/*.json` is a wire change and takes the
protocol-stability section of the pull request template.

The `AGENTS.md` / `CLAUDE.md` split from ADR 0009 stands. Enforcement that
was never the corpus stays where it is: the DoD workflows, the triage guard,
and the scoped-test hook.

## Why durable

One record per directive, in the repository the workspace already reads, is
the source. A later change is a new revision of that file, not a
five-repository rollout. The compiled summary remains local because a harness
opened on this repository reads `AGENTS.md` and does not read oxagen's.
