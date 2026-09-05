# AGENTS.md

Guidance for AI agents (and humans) working in this repository. The
Context Graph Protocol is a Rust workspace: protocol types, host,
reference providers (ripgrep, tree-sitter, trace, refprov), an MCP
bridge and server, and a conformance suite. `README.md` and
`CONTRIBUTING.md` are the authoritative sources for the details.

## Standing decisions — apply without asking

Each directive below is a Steering Context Record in [`docs/scr/`](docs/scr/);
the SCR is canonical — it carries the rationale, exceptions, and enforcement
status. This block is the compiled summary that every agent — Claude Code
(via CLAUDE.md's `@AGENTS.md` import) and Stella (which reads AGENTS.md
directly) — loads at session start. The corpus is identical across the
macanderson org repos.

- **[SCR-001](docs/scr/SCR-001-no-full-suite-builds.md) — Tests/builds
  (inner loop):** Never compile or run the full test suite while developing.
  Build and test only the crates/packages/modules touched by the change
  (plus direct dependents on interface changes). The full suite is CI's job.
  Here: `cargo test -p <crate> [filter]`, never bare `cargo test` / `cargo test --workspace`.
- **[SCR-002](docs/scr/SCR-002-durability-first-architecture.md) —
  Architecture decisions:** Do not ask. Choose the most durable option — the
  one that can't be questioned in 10 years as the right move. Cheap-and-easy
  only wins when it is also the excellent durable choice. Record every such
  decision as an ADR in `docs/adr/`; the ADR replaces the question.
- **[SCR-003](docs/scr/SCR-003-dod-verified-close.md) — Definition of
  done:** An issue closes only when every DoD checklist item is satisfied
  and verified. Reference-grade includes tests, code comments, docs, and
  CI — not just the implementation. A PR that advances an issue without
  finishing it links it with `Refs #N` rather than `Closes #N`: `Refs`
  does not close, so the merge gate does not hold that PR against the
  issue's DoD. A PR may carry both, and is gated only on what it closes.
- **[SCR-004](docs/scr/SCR-004-residue-becomes-issues.md) — Residue:**
  Before declaring any task complete, file a GitHub issue for every
  follow-up, tech-debt item, or logical next step you noticed. Apply ONLY
  the `triage` label.
- **[SCR-005](docs/scr/SCR-005-triage-separation-of-duties.md) — Triage
  separation of duties:** Never apply priority (`P0`–`P3`) or size labels —
  a dedicated triage agent owns sizing and priority; a guard workflow
  strips creator-applied priorities.
