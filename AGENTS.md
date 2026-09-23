# AGENTS.md

Guidance for AI agents (and humans) working in this repository. The
Context Graph Protocol is a Rust workspace: protocol types, host,
reference providers (ripgrep, tree-sitter, trace, refprov), an MCP
bridge and server, and a conformance suite. `README.md` and
`CONTRIBUTING.md` are the authoritative sources for the details.

## Standing decisions — apply without asking

The canonical records are context records in the oxagen workspace, under
[`.oxagen/rules/`](https://github.com/macanderson/oxagen/tree/main/.oxagen/rules).
Each bullet below is the compiled summary that every agent — Claude Code
(via CLAUDE.md's `@AGENTS.md` import) and Stella (which reads AGENTS.md
directly) — loads at session start. This repository is linked to that
workspace and does not carry a copy. Oxagen
[ADR-137](https://github.com/macanderson/oxagen/blob/main/docs/adr/ADR-137-standing-decisions-are-workspace-context-records.md)
retires `docs/scr/` in all five repositories.

- **[SCR-001](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.001-no-full-suite-builds.toml) — Tests/builds
  (inner loop):** Never compile or run the full test suite while developing.
  Build and test only the crates/packages/modules touched by the change
  (plus direct dependents on interface changes). The full suite is CI's job.
  Here: `cargo test -p <crate> [filter]`, never bare `cargo test` / `cargo test --workspace`.
- **[SCR-002](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.002-durability-first.toml) —
  Architecture decisions:** Do not ask. Choose the most durable option — the
  one that can't be questioned in 10 years as the right move. Cheap-and-easy
  only wins when it is also the excellent durable choice. Record every such
  decision as an ADR in `docs/adr/`; the ADR replaces the question.
- **[SCR-003](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.003-dod-verified-close.toml) — Definition of
  done:** An issue closes only when every DoD checklist item is satisfied
  and verified. Reference-grade includes tests, code comments, docs, and
  CI — not just the implementation. A PR that advances an issue without
  finishing it links it with `Refs #N` rather than `Closes #N`: `Refs`
  does not close, so the merge gate does not hold that PR against the
  issue's DoD. A PR may carry both, and is gated only on what it closes. A
  PR that closes nothing is waived by a label, and which one is a claim:
  `no-issue` for a trivial change, `closes-nothing` for a substantial one
  that closes no issue by design.
- **[SCR-004](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.004-fix-over-file.toml) — Fix over
  file:** Fix what you notice in the PR you are making; two unrelated fixes
  in one PR is fine. File an issue only when a fix cannot responsibly ride
  the PR (a maintainer decision, a rig or spend, or work larger than the
  session), and only when fixing it moves stability, reliability,
  maintainability, innovation, efficiency, or performance. Apply ONLY the
  `triage` label.
- **[SCR-005](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.005-triage-separation.toml) — Triage
  separation of duties:** Never apply priority (`P0`–`P4`) or size labels —
  a dedicated triage agent owns sizing and priority; a guard workflow
  strips creator-applied priority and size labels.
- **[SCR-006](https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.006-schema-change-labelled.toml) — Schema
  changes and migrations:** A pull request that changes a schema carries
  `migration-required`, and the migration reaches production before or with
  the deploy of that change, never after. Say in the PR which store changed
  and what must be applied. Do not add an automatic apply to a deploy
  pipeline under this record; that is a separate decision, made per repo.
  Here: this repository has no persistent store, so the record is inert and
  the label is never applied. A change to `schema/*.json` is a wire change
  and takes the protocol-stability section of the pull request template, not
  this label.
