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
- **Conformance registry + provider badge** (`site/content/docs/registry.mdx`,
  `docs/registry.md`, #20) — a page listing providers that are green on
  `contextgraph-conformance`'s suite, each backed by a reproducible
  `contextgraph-inspect --json` report (not a self-attested claim), seeded with
  the bundled `contextgraph-example-docs` reference fixture and its captured
  12/12 report. This is where the governance "two independent implementations"
  freeze criterion becomes checkable. Adds a static `conformant.svg` badge and a
  PR-template submission checklist requiring the exact reproducing invocation.
- **Release prep** (`.github/workflows/release.yml`, #16) — a tag-triggered
  (`contextgraph-v*`) workflow that publishes `contextgraph-types` →
  `contextgraph-host` → `contextgraph-conformance` to crates.io in dependency
  order, polling the sparse index between publishes
  (`.github/scripts/wait-for-crate.sh`). A tag push alone can never publish: an
  unconditional credential-free `publish-dry-run` CI job packages
  `contextgraph-types` on every PR, and the real publish is gated behind a
  `crates-io` GitHub Environment requiring reviewer approval. Adds crates.io +
  docs.rs badges to the root and per-crate READMEs (they read "not found" until
  the first real publish). Version cut and the environment/secret are the
  owner's call (see #16).
- **SDK publish prep** (`sdk/PUBLISHING.md`, `.github/workflows/publish-sdks.yml`,
  #59) — a per-registry release checklist (npm already live via #46; PyPI and Go
  pending) plus a tag-gated, secret-guarded publish workflow. The TypeScript SDK
  is published to npm as `@contextgraphprotocol/typescript-sdk` 0.1.0; the PyPI
  (`contextgraph-sdk`) and Go module publishes stay human-only (registry upload
  and an addressable git tag). SDK READMEs now say "not yet published" so the
  install snippets aren't misleading.
- **Downstream canary CI** (`.github/workflows/downstream-canary.yml`, #29) —
  the code-side half of the #27 boundary. Builds stella's `contextgraph-*`
  consumers (`stella-graph`, `stella-context`, `stella-cli`) against this repo's
  HEAD via a local `[patch]` override (`.github/scripts/downstream-canary-stella.sh`),
  on a daily schedule, `workflow_dispatch`, and PRs touching the wire crates.
  Deliberately advisory (`continue-on-error` + a `::warning::` flag) — a
  downstream break is a pre-freeze signal, not a reason to fail this repo's gate
  on a foreign project's state. A guarded `oxagen-canary` job activates once a
  human wires `OXAGEN_PLATFORM_TOKEN`.
- **Schema `$id` now names a URL that actually resolves** (#58). `$id` pointed at
  `contextgraphprotocol.org/schema/…`, which 404s until the Vercel project is
  Git-linked to this repo's `site/` (#57); it now names this repo's GitHub-raw
  URL, which resolves today regardless of how #57 is decided, as an interim
  measure until the domain can serve the file for real.
- **Structured error codes now survive the transport boundary** (#9) —
  `HostError::Provider` carries the provider's `ErrorCode`, the `ErrorCode`
  vocabulary gains `unsupported_representation` (§P5) and `incompatible_version`
  (§H3, non-retryable — a new `HostReaction::DropProvider`), and the
  `malformed-input-tolerance` conformance check now requires a `bad_request` code
  rather than passing on any error (with a `--misbehave mislabel-malformed` mode
  that exercises it).
- **Reference HTTP transport now enforces C7/C8** (#13) —
  `HttpProvider::connect_with_auth` / `Host::add_http` accept an optional bearer
  `Credential`; plaintext `http://` to a non-loopback host is refused with
  `HostError::InsecureTransport` before any bytes leave (loopback exempt);
  credentials attach via `bearer_auth` and render only as `Credential(<redacted>)`;
  a 401 surfaces as `HostError::Unauthorized`.
- **Host conformance: H3 version-rejection + crash-isolation scenarios** (#14) —
  the host-side harness now drives the reference `Host` at a provider declaring a
  mismatched major family (asserting a named `HostError::VersionMismatch` under an
  explicit timeout, so "never a hang" is load-bearing) and at a `query_all`
  fan-out where one provider dies mid-query (asserting the fan-out completes with
  the healthy frames and the crash is reported + excluded). `run_host_conformance`
  now exposes 8 checks.
- **Provider SDK HTTP adapters + scaffold generator** (#17) — host a provider
  behind one HTTP POST endpoint: `createHttpHandler` (TypeScript), `make_wsgi_app`
  (Python), `Handler` (Go), each with a runnable `example-docs-http` that goes
  green under `contextgraph-inspect http`. `create-contextgraph-provider`
  scaffolds a provider (TypeScript + Python) wired to both transports plus a CI
  workflow running `contextgraph-inspect` in the generated project's own CI from
  the first commit. TS + Python quick-starts and an HTTP-transport section added
  to `docs/implementing-a-provider.md`.
- **`SPEC.md` normative completeness pass** — folds every shipped wire surface
  into the single normative home ahead of the freeze (#49, #50, #48, #13). Adds
  §9 **Verification** (`verify`/`verified`, V1–V4), §6.3 **Frame identity**
  (D1–D4), §6.4 **Representations** (`full`/`compact`/`reference`, P1–P5) with an
  explicit **1.0 scope boundary for `context/resolve`** — the operation is
  deferred to a `1.x` additive minor (sketch: [docs/sketches/resolve.md](./docs/sketches/resolve.md)),
  so a remote provider should not emit un-rehydratable `reference` frames (#50).
  Adds §4.1 **egress scopes and consent receipts** (C5–C6) and §4.2 **transport
  security** (C7 TLS-for-non-loopback, C8 credentials-never-logged, #13). Adds
  §13 **Extensibility** (U1 ignore-unknown-members, U2 closed `FrameKind` /
  open vocabularies, U3 reserved `:` namespaces, U4 no-repurpose/deprecation) —
  the rules that make the additive-only freeze real, distinguishing the
  authoring-strict JSON Schema from the U1 interop contract (#48). Adds the
  `unsupported_representation` and `incompatible_version` error codes (#9). No
  wire-shape change: all of this documents surfaces already carried by the
  schema and reference types.
- **Restored `docs/context-reuse.md` §3** (Consent scopes and receipts), whose
  normative text was dropped by the PR #38 merge — recovered from `d229ed9` and
  reconnected to the C5/C6 requirements the schema and `consent-scope` check
  already cite.
- **Host execution trace + replay oracles** (`contextgraph-trace`, sketch stage,
  unpublished) — the host-side dual of the provider conformance suite. An
  append-only NDJSON journal a harness (or a Harbor-adapter-style shim
  observing one) emits while it works — turns, prompt assemblies, tool-call
  pairing, `context/verify` observations, side effects, crashes and resumes —
  plus eight pure replay oracles that hold the recording to the loop
  invariants: `sequence-integrity`, `turn-loop-pairing`,
  `assembly-budget-honesty`, `staleness-at-use`, `citation-at-use`,
  `deterministic-composition`, `effect-exactly-once`, `resume-integrity`. The
  journal reuses the protocol's identity spine (`FrameId`, wire `Verdict`) and
  carries no frame bodies; the crate depends on `contextgraph-types` + serde
  only. Ships golden journals plus one adversarial fixture per check that
  trips exactly that check. No wire shape or `SPEC.md` change — see
  [docs/sketches/host-trace.md](./docs/sketches/host-trace.md).
- **Prompt ingestion as a local provider** (`contextgraph_host::ingest`) — the
  ingestion-side dual of `compose_context`. Turns a user's paste into an ordinary
  `ContextProvider`: intent passes through verbatim as `query.goal`, directory
  references become `query.anchors`, and pasted evidence (logs, tables, code,
  notes) becomes content-addressed frames served `compact` by default with the
  full bytes rehydratable via a `[full]` re-query. Deterministic segmentation,
  honest `token_cost`/`content_digest` per representation (§B3), `derivation`
  (not `file`) provenance, and exact `verify` on immutable content. Local-only
  and egress-free — no consent friction. Host-side reference behavior; no wire
  shape or `SPEC.md` change. Ships a wire-conformance test that validates real
  ingested frames (full/compact/reference) against the frame, budget, and JSON
  Schema contracts. See
  [docs/adr/0006-prompt-ingestion-as-a-local-provider.md](./docs/adr/0006-prompt-ingestion-as-a-local-provider.md).
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

### Fixed
- **`SPEC.md` §9's `verify` example no longer fails the schema `SPEC.md` ships.**
  Both envelopes carried `"id": "v1"`, but §3.2 grants an `id` only to
  `query`/`frames`/`error`, the reference `Envelope::Verify`/`Verified` have no
  such field, and the schema is `additionalProperties: false` — the example was
  invalid against the protocol's own definition. The `id`s are removed;
  `verify` correlates by full frame identity, not by envelope id. Root cause:
  `schema/validate-examples.py` checked `examples/` but never `SPEC.md`, so the
  one example surface with no machine check was the one that drifted. It now
  validates every fenced `jsonc` block in `SPEC.md` too (comments and documented
  placeholders normalized away, structure checked), and CI's existing `schema`
  job therefore catches this class of drift.
- **Regression guard for the `ContextQuery` `required` fix.** The schema change
  itself landed independently in #63; this adds the test that keeps it fixed —
  an ordinary unfiltered, unanchored query must satisfy the schema's *own*
  `required` array (read from the schema, so it cannot drift into a stale
  snapshot). A cross-audit of all 16 shared types confirms no other type demands
  a field its serializer elides — this bug class has now recurred twice
  (`ContextFrame` in PR #44, `ContextQuery` in #63), so it is worth a standing
  check rather than another one-off fix.
- **§G2 and §D1 are now actually verified, not merely asserted.** Both named
  `frame-validity` as their verifier while neither `target_uri` nor the frame's
  own `content_digest` was read by any check — the self-attestation §11.1
  exists to rule out. `check_frames` now rejects a relation with an empty
  `target_uri` (§G2) and a present-but-malformed `content_digest` (§D1); its
  evidence string had claimed "well-formed digests" while accepting
  `sha256:abc`. §D1 was found by auditing the other ten rules that cite
  `frame-validity` after §G2 turned out to be unenforced; the remaining nine
  were confirmed enforced.
- **`contextgraph-host::wire` docs no longer invert a MUST NOT.** The module
  said concurrency is "negotiated by observation, not by a capability flag",
  contradicting `SPEC.md` §3.2 and the shipped `Capabilities::correlation`: a
  host **MUST NOT** send an `id` to a provider that did not declare correlation.
- JSON Schema: a `ContextFrame`'s `required` is now exactly what the reference
  serializer always emits (`id`, `kind`, `title`, `score`, `token_cost`).
  `provenance` and `relations` were listed as globally required but are
  `skip_serializing_if = Vec::is_empty` in the reference type and required by no
  frame-validity check, so a Rust-serialized frame with no edges failed schema
  validation. Surfaced by ADR 0006's wire-conformance test — the first to
  validate serialized frames (not just hand-authored examples) against the
  schema. `content` remains governed per-representation by the existing `allOf`.

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
