# Changelog

All notable changes to the Open Context Protocol (OCP) crates and this
specification repository are documented in this file.

The OCP crates (`ocp-types`, `ocp-host`, `ocp-conformance`) track **crate
version** (`0.x` today) and **protocol version** (`ocp/1.0-draft`) as two
independent axes — see [docs/stability.md](./docs/stability.md). This changelog
records crate releases and spec-repository milestones together, noting which is
which. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- [`schema/ocp-envelope.schema.json`](./schema/ocp-envelope.schema.json) — a
  machine-readable JSON Schema (Draft 2020-12) for the OCP envelope and all wire
  types. Validates in any language (`ajv`, Python `jsonschema`, Rust
  `jsonschema`, Go `gojsonschema`). Includes `schema/validate-examples.py` to
  check the bundled examples and serve as a validator-usage reference.
- [`examples/`](./examples/) — diffable wire transcripts of a complete OCP
  session (NDJSON + pretty-printed reference messages), so an implementer in
  any language can diff their output against the exact shapes on the wire.
- `GOVERNANCE.md` — maintainer-led model, normative-change process, and the
  concrete criteria for the `ocp/1.0-draft` → `ocp/1.0` freeze.
- Repository governance files: `SECURITY.md`, `CODE_OF_CONDUCT.md`, and
  GitHub issue/PR templates.
- Prominent **License** section in the README clarifying the dual MIT OR
  Apache-2.0 licensing of all OCP crates.
- A consolidated **Conformance requirements** section in
  `docs/protocol-surface.md`, with RFC 2119 keywords and a formal ABNF grammar
  for the protocol version string.

### Changed
- `docs/protocol-advantages.md`: corrected "MIT licensed" to the accurate
  dual-license statement ("MIT OR Apache-2.0") to match the rest of the repo.
- `docs/protocol-advantages.md`: fixed a misspelling — "BTreive" → "Btrieve".
- `docs/protocol-advantages.md`, `docs/running-conformance.md`: removed leftover
  references to the unrelated `stella` project, replacing them with OCP-specific
  names (`ocp-graph`, `ocp-example-docs`).

### Fixed
- `docs/index.md`: removed dangling references to `PUBLISHING.md` and
  `RELEASING.md`, which do not exist in this repository.
- `CONTRIBUTING.md`: commit-message examples and issue-tracker links no longer
  reference the `stella` project; they now point at `opencontextprotocol` and
  use OCP crate scopes.

## [0.1.0] — 2026-07-17

The first published release of the Open Context Protocol crates and the
specification repository. Protocol version: `ocp/1.0-draft`.

### Added — crates
- **`ocp-types`** — the wire types (`ContextFrame`, `ContextQuery`,
  `Capabilities`, `Provenance`, `DataFlow`, `FrameKind`), round-tripping
  through `serde_json` with zero dependencies beyond `serde`.
- **`ocp-host`** — the host runtime: the `ContextProvider` trait, fan-out
  router with budget-honesty auditing, the `ConsentStore` egress gate, the
  `wire::Envelope` NDJSON/HTTP framing, and `versions_compatible` major-family
  matching.
- **`ocp-conformance`** — the machine-checked conformance suite with five
  adversarial checks (`handshake`, `frame-validity`, `budget-honesty`,
  `shutdown-clean`, `malformed-input-tolerance`), the `ocp-inspect` CLI, and the
  `ocp-example-docs` reference provider with `--misbehave` failure modes.

### Added — specification & docs
- `README.md` — the one-read explanation: the blob-pipe problem, the seven
  guarantees, the wire surface, relation to MCP, and why you would build
  against it.
- `docs/overview.md` — the engineering-oriented technical overview.
- `docs/protocol-surface.md` — the normative wire types bound to `ocp-types`.
- `docs/protocol-advantages.md` — standalone research analysis of the seven
  advantages, with grounding in primary research.
- `docs/implementing-a-provider.md` — the provider build guide (in-process
  Rust trait and out-of-process wire protocol, any language).
- `docs/running-conformance.md` — how to run the conformance suite via CLI or
  library.
- `docs/stability.md` — the crate-semver vs. protocol-version model.
- `CONTRIBUTING.md` — contribution guidelines (Conventional Commits, DCO, PR
  checklist).
- Dual license files: `LICENSE-MIT`, `LICENSE-APACHE`.

[Unreleased]: https://github.com/macanderson/opencontextprotocol/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/macanderson/opencontextprotocol/releases/tag/v0.1.0
