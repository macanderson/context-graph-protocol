# Changelog

All notable changes to the Context Graph Protocol crates and this
specification repository are documented in this file.

The Context Graph Protocol crates (`contextgraph-types`, `contextgraph-host`, `contextgraph-conformance`) track **crate
version** (`0.x` today) and **protocol version** (`contextgraph/1.0-draft`) as two
independent axes — see [docs/stability.md](./docs/stability.md). This changelog
records crate releases and spec-repository milestones together, noting which is
which. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- **Frame representations** on `ContextFrame` — `full` | `compact` | `reference`
  (CGEP lifecycle phase 2). A frame now states *how* it carries its content:
  `reference` frames carry no inline content, only a `content_ref` resolver
  handle and a `canonical_content_hash`; `compact` frames inline a transformed
  rendering alongside both. Additive and backward-compatible — `representation`
  absent ⇒ `full`, and full/legacy frames are unchanged on the wire. Adds
  `content_ref`, `canonical_content_hash`, `content_fidelity`, `transform`,
  `minimum_content_fidelity`, `inline_content_requirement`, `canonical_token_cost`,
  and `tokenizer_ref`; `content` becomes optional (absent for references).
  Negotiated via `ContextQuery.representation_preferences` and
  `Capabilities.representations` + `Capabilities.resolve`. Enforced in Rust
  (`ContextFrame::representation_invariants`), the JSON Schema, and conformance
  tests. See
  [docs/adr/0005-frame-representations.md](./docs/adr/0005-frame-representations.md).
- `SPEC.md` — the single normative specification, self-contained and with stable
  requirement anchors (#3).
- `MIGRATION.md` — rename map, breaking-change list, and the GitHub
  redirect-hazard warning for downstreams pinning the old URL (#30).
- CI: fmt, clippy, test, MSRV, conformance green **and** `--misbehave` red,
  schema validation, examples/types round-trip (#2).
- `docs/adr/` — ADR 0002 (request correlation), 0003 (canonical token
  accounting), 0004 (dead capability surface).
- Canonical token accounting: `budget_tokens`, conformance requirement B3 (#8).
- Structured error codes with host-reaction guidance; open vocabulary (#9).
- Request correlation: `Capabilities.correlation`, envelope `id`, H4 (#4).
- Format validation: RFC 3339 UTC timestamp profile (F4), `sha256:` digest
  grammar (F5) (#10, #12).
- `max_frames` audit (B4) and graph relation `display_name` check (G1) (#7, #10).
- Recommended relation vocabulary `frame::rel` (#7).
- Embedding fingerprint format and exact-match rule (E1) (#11).

### Removed
- **Breaking:** `Capabilities.upsert`, `Capabilities.subscribe`, and
  `QueryCapability.filters` — negotiable at handshake but unreachable by any
  host. Wire-compatible; Rust API breaking (#5, #6, #11).

### Changed
- **Breaking:** `token_cost` MUST now equal the canonical count for its content.
  Providers that under-declared cost were previously green (#8).
- Withdrew the incorrect claim that CGP rides JSON-RPC 2.0 (#4).
- Code comments cite `SPEC.md` anchors instead of a private repository (#3).

### Added
- [`schema/contextgraph-envelope.schema.json`](./schema/contextgraph-envelope.schema.json) — a
  machine-readable JSON Schema (Draft 2020-12) for the Context Graph Protocol envelope and all wire
  types. Validates in any language (`ajv`, Python `jsonschema`, Rust
  `jsonschema`, Go `gojsonschema`). Includes `schema/validate-examples.py` to
  check the bundled examples and serve as a validator-usage reference.
- [`examples/`](./examples/) — diffable wire transcripts of a complete Context Graph Protocol
  session (NDJSON + pretty-printed reference messages), so an implementer in
  any language can diff their output against the exact shapes on the wire.
- `GOVERNANCE.md` — maintainer-led model, normative-change process, and the
  concrete criteria for the `contextgraph/1.0-draft` → `contextgraph/1.0` freeze.
- Repository governance files: `SECURITY.md`, `CODE_OF_CONDUCT.md`, and
  GitHub issue/PR templates.
- Prominent **License** section in the README clarifying the dual MIT OR
  Apache-2.0 licensing of all Context Graph Protocol crates.
- A consolidated **Conformance requirements** section in
  `docs/protocol-surface.md`, with RFC 2119 keywords and a formal ABNF grammar
  for the protocol version string.

### Changed
- `docs/protocol-advantages.md`: corrected "MIT licensed" to the accurate
  dual-license statement ("MIT OR Apache-2.0") to match the rest of the repo.
- `docs/protocol-advantages.md`: fixed a misspelling — "BTreive" → "Btrieve".
- `docs/protocol-advantages.md`, `docs/running-conformance.md`: removed leftover
  references to the unrelated `stella` project, replacing them with Context Graph Protocol-specific
  names (`contextgraph-graph`, `contextgraph-example-docs`).

### Fixed
- `contextgraph-host` and `contextgraph-conformance` did not compile from a
  half-applied merge of #37 (egress-scope + consent receipts): `host.rs` used
  `ConsentReceipt`/`EgressScope` without importing them and a `DataFlow` literal
  omitted `egress_scopes`; the conformance crate used `FrameId`/`DropReason`
  without importing them, a test omitted a `CHECK_VERIFY_HONESTY` import, and a
  check-count assertion was stale (6, now 7). Restored so the workspace builds
  and the full test suite passes. (Pre-existing on `main`; unrelated to frame
  representations but required to build the branch.)
- `docs/index.md`: removed dangling references to `PUBLISHING.md` and
  `RELEASING.md`, which do not exist in this repository.
- `CONTRIBUTING.md`: commit-message examples and issue-tracker links no longer
  reference the `stella` project; they now point at `context-graph-protocol` and
  use Context Graph Protocol crate scopes.

## [0.1.0] — 2026-07-17

The first published release of the Context Graph Protocol crates and the
specification repository. Protocol version: `contextgraph/1.0-draft`.

### Added — crates
- **`contextgraph-types`** — the wire types (`ContextFrame`, `ContextQuery`,
  `Capabilities`, `Provenance`, `DataFlow`, `FrameKind`), round-tripping
  through `serde_json` with zero dependencies beyond `serde`.
- **`contextgraph-host`** — the host runtime: the `ContextProvider` trait, fan-out
  router with budget-honesty auditing, the `ConsentStore` egress gate, the
  `wire::Envelope` NDJSON/HTTP framing, and `versions_compatible` major-family
  matching.
- **`contextgraph-conformance`** — the machine-checked conformance suite with five
  adversarial checks (`handshake`, `frame-validity`, `budget-honesty`,
  `shutdown-clean`, `malformed-input-tolerance`), the `contextgraph-inspect` CLI, and the
  `contextgraph-example-docs` reference provider with `--misbehave` failure modes.

### Added — specification & docs
- `README.md` — the one-read explanation: the blob-pipe problem, the seven
  guarantees, the wire surface, relation to MCP, and why you would build
  against it.
- `docs/overview.md` — the engineering-oriented technical overview.
- `docs/protocol-surface.md` — the normative wire types bound to `contextgraph-types`.
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

[Unreleased]: https://github.com/macanderson/context-graph-protocol/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/macanderson/context-graph-protocol/releases/tag/v0.1.0
