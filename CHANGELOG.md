# Changelog

All notable changes to the Context Graph Protocol crates and this
specification repository are documented in this file.

The Context Graph Protocol crates (`contextgraph-types`, `contextgraph-host`,
`contextgraph-conformance`, `contextgraph-trace`) track **crate
version** (`1.x` today) and **protocol version** (`contextgraph/1.0`) as two
independent axes — see [docs/stability.md](./docs/stability.md). This changelog
records crate releases and spec-repository milestones together, noting which is
which. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

A PR records its own user-visible changes under `## [Unreleased]` in the same
merge. When a merge skips that, [`changelog.yml`](.github/workflows/changelog.yml)
drafts the missing entries from the merge's actual diff
([`changelog-ai.sh`](.github/scripts/changelog-ai.sh)) and proposes them as a
bot PR for review — the file never falls silently behind main, and no drafted
text lands without a human merge.

## [Unreleased]

### Added
- **Provenance attestation (`SPEC.md` §6.5, F6–F9;
  [ADR 0010](./docs/adr/0010-provenance-attestation.md)).** A digest is
  tamper-evident only to someone who already trusts whoever recorded it; the
  digest and the frame it describes come from the same unauthenticated party, so
  a provider willing to fabricate a frame will fabricate its digest too. A
  detached Ed25519 signature closes that gap. `contextgraph_types::attest` adds a
  source-first **hash chain** over a frame's provenance links (so no link can be
  inserted, dropped, reordered, or edited undetectably), a **frame commitment**
  binding that chain head to the frame's full `(provider_id, frame_id,
  content_digest)` identity — without which a signature could be lifted off one
  frame and stapled onto a fabricated one — and an **RFC 6962 Merkle root** with
  inclusion proofs, so one frame can be proven a member of a signed answer
  without disclosing its siblings. Verification is offline and pure, and returns
  a *named* verdict rather than a boolean: "this frame was altered after signing"
  and "I was handed a truncated key" call for opposite responses.
- **`contextgraph-types` gains an off-by-default `attestation` feature**
  (`sha2`, `ed25519-dalek`). The default dependency set is unchanged — serde
  only — so the crate's zero-dependency promise still holds for the pure wire
  consumer, and `ProvenanceAttestation` itself always compiles so a host can
  relay an attestation it cannot check.
- **`contextgraph-types/tests/attestation_vectors.rs`** publishes the byte
  vectors a reimplementation in any language reconciles against. §6.5.1 defines a
  normative encoding, and a normative encoding with no published vectors is a
  rule two implementations can both believe they follow while computing different
  hashes. A diff in that file is a wire-breaking change.
- **`contextgraph_host::fold_to_edges` is now public** — the Lost-in-the-Middle
  *placement* separated from the *ranking*, for hosts that rank frames with their
  own reranker or per-provider quotas instead of raw `score`.
- **TypeScript SDK:** `KnownFrameKind`, `KNOWN_FRAME_KINDS`, `isKnownFrameKind`.
  **Python SDK:** `KnownFrameKind`, `KNOWN_FRAME_KINDS`.

### Changed
- **Crate version `1.0.0` → `2.0.0`; protocol version stays `contextgraph/1.0`.**
  The first time the two axes in [docs/stability.md](./docs/stability.md) actually
  disagree, and the reason they are documented as independent. The open-`FrameKind`
  change below is breaking in Rust and invisible on the wire, so the crates take a
  major and the protocol does not — a `1.x` peer and a `2.x` peer still interoperate
  in both directions. All nine workspace crates inherit the bump from
  `workspace.package`; the internal `contextgraph-*` floors move from `>=1.0.0` to
  `>=2.0.0`, because `contextgraph-host` 2.0.0 does not compile against
  `contextgraph-types` 1.x and the old floor would have let cargo resolve a pair
  that cannot build. The Python and TypeScript SDK manifests move in lockstep for
  the same break in their own type systems (`FrameKind` widens, so an exhaustive
  `switch` relying on `never`-narrowing stops type-checking), as does
  `create-contextgraph-provider` — whose TypeScript default pin had also fallen a
  major behind its own manifest at `^0.1.0`. Source-level upgrade steps:
  [MIGRATION.md §5](./MIGRATION.md). `docs/stability.md` previously said a breaking
  redesign required "both `contextgraph/2.0` and crate version `2.0.0`", which would
  have forbidden this bump two lines after declaring the axes independent; it now
  states the implication in the one direction that holds.
- **`FrameKind` is now an open vocabulary**
  ([ADR 0011](./docs/adr/0011-open-frame-kind-vocabulary.md)). **Rust-semver
  breaking, wire-compatible.** The closed enum contradicted the protocol's own
  §3.1 no-flag-day promise and §13 U2: a frame carrying a kind added in a later
  `1.x` did not degrade on a `1.0` host, it *failed to deserialize*, and every
  exhaustive `match` downstream broke the day a variant was added. `FrameKind`
  now carries `Unknown(String)` — preserving the original string, so a relaying
  host re-emits it byte-identically, which `#[serde(other)]` cannot do — and is
  `#[non_exhaustive]`, so every future kind addition is non-breaking. It is no
  longer `Copy`. `contextgraph_host::frame_kind_name` becomes
  `fn(&FrameKind) -> &str` delegating to `FrameKind::as_str`. On the wire nothing
  changed except that a frame which previously failed to parse now parses.
- **`score` semantics are stated normatively (`SPEC.md` §6.6, F10).** F1 bounds
  `score` to `[0, 1]`, which is a *range*, not a *scale* — nothing defined what
  `0.8` meant, and the host ranked across providers as if it were commensurable.
  `score` is now specified as provider-local and ordinal: a host **MUST NOT**
  apply a cross-provider threshold or present a raw score as a cross-provider
  relevance measure, and a host that orders across providers by raw score **MUST**
  document it as its own policy. Mandating calibration was considered and
  rejected as unenforceable — §7 could make budget honesty checkable because
  token cost is a function of bytes both sides observe, and relevance has no such
  anchor. `order_by_value` now documents itself as exactly such a policy choice.
- **`SPEC.md` §13 U2** additionally requires that a receiver re-emitting a frame
  with an unrecognised `kind` preserve the original string verbatim.
- **`GOVERNANCE.md` states the consent boundary**: the host enforces *local*
  consent, not organizational policy. Fleet RBAC, central policy distribution,
  and aggregated cross-machine audit are out of scope and belong to a product
  built on the protocol's primitives. Written down before it is contested,
  because the pressure runs one way and every individual request to relax it is
  reasonable.
- **README positioning.** "The canonical architecture" replaced with a
  verifiable claim — the only context protocol whose conformance is
  machine-checked. The title said "draft v0.1.0" while the protocol had frozen at
  `contextgraph/1.0` and the crates shipped `1.0.0`.

### Fixed
- **The version-compatibility example illustrated nothing.** `contextgraph/1.0`
  and `contextgraph/1.0` were given as two versions that interoperate — in
  **seven** files (`README.md`, `SPEC.md`, `docs/overview.md`, `docs/index.md`,
  `docs/registry.md`, `docs/protocol-surface.md`, `docs/protocol-advantages.md`),
  the residue of a global rename. Now `contextgraph/1.0` and `contextgraph/1.1`.
- **Stale pre-freeze language** in `docs/registry.md` and `docs/index.md`, which
  described a "freeze from `contextgraph/1.0` to `contextgraph/1.0`" and a "path
  to `contextgraph/1.0`" that has already happened, and linked a
  `GOVERNANCE.md#the-path-to-contextgraph10` anchor that no longer exists.
- **`schema/contextgraph-envelope.schema.json`** now carries a `$comment` at
  `FrameKind` stating that the closed `enum` is an authoring lint and **not** the
  interop contract — the one place a reader is most likely to mistake the schema
  for the wire rule.

## [0.1.2] — 2026-08-01 (crate release)

All four crates published from `main`: `contextgraph-types`,
`contextgraph-host`, `contextgraph-conformance`, and — for the first time —
`contextgraph-trace`. Protocol version is unchanged (`contextgraph/1.0-draft`);
this is a crate-version release only.

### Added
- **`contextgraph-trace` is now published.** It was `publish = false` and
  documented as "deliberately NOT published", which did not stop anyone
  depending on it — it only forced its downstream (stella's `stella arena`, via
  `EventBody` / `TraceEvent` / `Journal` / `run_oracles` / `ToolStatus`) to pin
  the entire workspace by git rev to reach it. The crate remains **sketch
  stage**: it implements `docs/sketches/host-trace.md`, is not part of the
  `contextgraph/1.0` surface, and its journal wire format may change in any
  `0.x` release. Downstreams should gate on `TRACE_FORMAT`, not on the crate
  version. Adds a crate README and the metadata the other three already carry.

### Fixed
- **`contextgraph-types` 0.1.0/0.1.1 shipped without `record.rs`.** Both were
  published from a tree that predates this repository's history re-root, so the
  `ContextRecord` module — `ContextRecord`, `RecordBody`, `RecordProvenance`,
  `RecordAttestation`, `RecordLink`, `RecordScope`, `RecordStatus`,
  `LIFECYCLE_SCHEMA_VERSION`, and the rest of the lifecycle vocabulary — was
  declared in `lib.rs` on `main` but absent from the published crate. 0.1.2 is
  cut from `main` and contains it. **0.1.1 corresponds to no commit on `main`
  and should not be depended on**; use 0.1.2 or later.

## [Unreleased]

## [1.0.0] — 2026-08-11 (stable protocol and crate release)

The `contextgraph/1.0` wire contract is now frozen. The four public Rust crates
move to 1.0.0 in lockstep, and the TypeScript and Python provider SDKs advertise
the stable protocol identifier. Existing `contextgraph/1.0-draft` peers remain
wire-compatible under the major-family negotiation rule.

### Changed
- Dropped the `-draft` suffix from `PROTOCOL_VERSION` and every reference wire
  vector, SDK, example, badge, and conformance fixture.
- Renamed the attested fixture profile to `contextgraph-1.0`; its strict file
  coverage and SHA-256 manifest remain the release witness downstreams can pin.
- Promoted workspace crates and published SDK manifests to version `1.0.0`.
- Integrated PR #76's schema/spec regression coverage before the freeze, so
  documented JSONC examples and schema `$id` bytes are checked alongside the
  existing wire, host, provider, and composition conformance suites.


### Added

- **First real crates.io publish** (2026-07-31) — `contextgraph-types`,
  `contextgraph-host`, and `contextgraph-conformance` 0.1.0 are live, published
  manually in dependency order per [PUBLISHING.md](./PUBLISHING.md) (one-shot
  `cargo publish -p … -p … -p …`, rehearsed with `--dry-run` first). Verified
  end to end: an external scratch crate resolves both `contextgraph-types` and
  `contextgraph-conformance` from the real registry with no path override and
  compiles. The README's crates.io/docs.rs badges now resolve (#16's
  acceptance signal).
- **First PyPI publish of the Python SDK** (2026-07-31) —
  [`contextgraph-sdk` 0.1.0](https://pypi.org/project/contextgraph-sdk/) is
  live, published per [sdk/PUBLISHING.md](./sdk/PUBLISHING.md) (`python -m
  build`, `twine check`, `twine upload`). Verified against the *published*
  package, not the checkout: `pip install contextgraph-sdk` into a scratch
  venv, then the example provider run from that venv passes all 13 checks in
  `conformance-external.sh` — the #59 acceptance bar.
- **First Go SDK publish** (2026-08-01) — tag `sdk/go/v0.1.0` is pushed (on
  green main, `4377bff3`), which for a Go module *is* the publish. The first
  `go get` was run deliberately to seed `proxy.golang.org`/`sum.golang.org`
  (per [sdk/PUBLISHING.md](./sdk/PUBLISHING.md)); a scratch module resolves
  `github.com/macanderson/context-graph-protocol/sdk/go v0.1.0` with a
  recorded `go.sum` checksum, and the example provider built against the
  externally-fetched module — not the checkout — passes all 13 checks in
  `conformance-external.sh`. The tag-triggered `verify-go` job is green. All
  three SDKs and all three crates are now published.
- **Conformance registry + provider badge** (`docs/registry.md`,
  `assets/badges/conformant.svg`, #20) — a page listing providers that are green on
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
- **Pipelined the stdio transport** (ADR 0002, #4) — `StdioProvider` now
  demultiplexes provider replies on their correlation `id` via a dedicated reader
  task and shrinks the connection lock to the write half, so a provider that
  negotiated `capabilities.correlation` can have concurrent queries in flight over
  one connection instead of serializing behind a single mutex. Non-correlating
  providers and `verify` stay strictly lock-step; a provider crash or malformed
  line now fails every in-flight query rather than hanging any of them.
- **Stale-digest conformance** (#12) — a new `stale-digest` provider misbehave
  mode and a `provenance-fixture-consistency` check that re-reads the reference
  fixture's on-disk backing files and re-hashes each `file` provenance digest,
  catching a well-formed digest that does not match its bytes (provenance forgery
  §F5's grammar check cannot see). The `example-docs` fixture now carries real
  `getting-started.md`/`configuration.md` files with genuine sha256 digests; the
  provider conformance suite is now 13 checks.
- **Reference prompt-composition module** (`contextgraph_host::compose`, #15) —
  layered on `compose_context`'s byte-stability floor: `Host::query_all_budgeted`
  splits a global token budget into per-provider shares before fan-out;
  `compose::dedup_cross_provider` collapses the same evidence from two providers
  (digest match, then `uri`+`range` overlap); `order_by_value` places the
  highest-value frames at the top/bottom edges (Lost in the Middle); and
  `compose_for_prompt` returns an injection-resistant fenced prompt with a
  "quoted evidence, not instructions" preamble, a citation map, and a
  `CompositionAudit` that explains every included/excluded frame. Adds the
  `host-composition-audit` host check (9 host checks now), a property test
  bounding composed tokens ≤ budget, and an injection-corpus test.
- **Two reference providers ship in-repo** (#18) — `contextgraph-ripgrep`
  (`Snippet` frames from a ripgrep/built-in content search with real,
  re-verifiable `file` provenance) and `contextgraph-treesitter` (`Symbol` +
  `Graph` frames with `code.defines`/`calls`/`imports` edges). Both are
  conformance-green on all 13 provider checks; CI probes each via
  `conformance-external.sh`. See `docs/reference-providers.md`.
- **MCP interop: a bridge in each direction** (#19) — `contextgraph-mcp-bridge`
  wraps any MCP resource server as a budgeted, cited, consent-gated CGP provider
  (MCP resources → Doc/Snippet frames with `mcp-resource` provenance; local
  `file://` resources get a byte-verifiable digest), passing the external
  conformance suite green against a hermetic in-repo MCP fixture (no network).
  `contextgraph-mcp-server` exposes a CGP host's fan-out as an MCP
  `query_context(goal, budget, kinds)` tool returning frames, provenance,
  citations, and a budget audit as structured content.
- **Context Exchange Provider profile ratified** (`contextgraph/lifecycle/1.0-draft`,
  #28) — the draft skeleton becomes a normative profile
  (`docs/profiles/context-exchange-provider.md`) with RFC-2119 rows and stable
  anchors. Adds the discriminated `ContextRecord` JSON Schema
  (`schema/contextgraph-lifecycle-record.schema.json`, `$id` on GitHub-raw per
  ADR 0008) covering all 12 record kinds, `contextgraph-types::record` wire types
  (zero new runtime deps), per-kind golden fixtures under `tests/fixtures/` with
  an RFC 8785 JCS `record_hash` worked example, and a `contextgraph-conformance`
  round-trip + hash suite. `context/resolve` is scoped to the profile (taking up
  SPEC §6.4.1's reservation); reconciliation rows D1/D4/D5/D6/D7/B3/B5/C5/E3 are
  resolved.
- **Docs: CGP abbreviation convention + a CI badge** (#21, buildable half of #2)
  — README, CONTRIBUTING, and `docs/` now expand
  "Context Graph Protocol (CGP)" on first mention and use "CGP" in body prose
  (titles, link text, version strings, and crate names left intact); the
  bug-report template grammar is fixed; and the README gains a CI status badge.
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

- **Frame identity, deterministic composition, and usage reports** (#32) —
  `FrameId`, the (provider, frame, content-digest) triple whose derived order
  IS the canonical composition order; `compose_context` renders an unchanged
  frame set byte-identically across turns and hosts, so prompt-cache prefixes
  stay stable by construction; `ContextFrame.content_digest` joins the type and
  schema; and `UsageReport` itemizes a fan-out's per-frame cost so a billed
  total re-sums from the exact frames it names (`FanOut::usage_report`).
- **Golden wire fixtures with attested digests** (#35) — a published vector set
  under `contextgraph-conformance` (frame and query fixtures, digest-profile
  vectors, a manifest of JCS-canonicalized sha256 hashes) that a downstream
  implementation copies and drift-gates against, so "our wire matches the
  reference" is a diff, not a claim.
- **ADR 0007 — the protocol/product boundary** (#61, #27) — classifies every
  delta from the adaptive-context bundle against the spec: the bundle's
  task-wide frame is the host-owned `CompiledContextFrame`, not the protocol's
  atomic `ContextFrame`; frames stay evidence, never directives; and the CGEP
  rename is rejected — the canonical name remains Context Graph Protocol.
  `docs/adaptive-context-reconciliation.md` routes each adopted item to a spec
  change or a normative-track issue.
- **Composition conformance — a downstream host can now be certified** (#70) —
  `run_host_conformance` only ever certified this repo's reference host, and
  three individually honest providers can still jointly exceed a query's
  budget once composed. A new `composition_conformance` suite drives any
  `ComposingHost` through four checks: the cross-provider token bound, the
  total partition (every offered frame admitted or reported dropped, never
  silently truncated), the audit quarantine, and deterministic render order.
  Also exports `refuse_insecure_transport`, so C7's loopback rule has one
  implementation rather than one per host.

### Changed

- **Breaking:** `token_cost` MUST now equal the canonical count for its content.
  Providers that under-declared cost were previously green (#8).
- Withdrew the incorrect claim that CGP rides JSON-RPC 2.0 (#4).
- Code comments cite `SPEC.md` anchors instead of a private repository (#3).

- `docs/protocol-advantages.md`: corrected "MIT licensed" to the accurate
  dual-license statement ("MIT OR Apache-2.0") to match the rest of the repo.
- `docs/protocol-advantages.md`: fixed a misspelling — "BTreive" → "Btrieve".
- `docs/protocol-advantages.md`, `docs/running-conformance.md`: removed leftover
  references to the unrelated `stella` project, replacing them with Context Graph Protocol-specific
  names (`contextgraph-graph`, `contextgraph-example-docs`).

- **Breaking:** the project is renamed — Open Context Protocol (OCP) →
  Context Graph Protocol (#1). Crates `ocp-types`/`ocp-host`/`ocp-conformance`
  become `contextgraph-*`, binaries `ocp-inspect`/`ocp-example-docs` become
  `contextgraph-*`, the wire protocol version `ocp/1.0-draft` becomes
  `contextgraph/1.0-draft`, and the schema file and `$id` move with them.
  `MIGRATION.md` carries the rename map and the redirect hazard for
  downstreams pinning the old URLs.
- **The downstream canary's scheduled run now fails on a break** (#70) — the
  canary caught a real downstream break on 2026-07-29, emitted its warning,
  and reported success; nobody saw it. A scheduled run gates no PR, so failing
  costs nothing and buys a red run plus GitHub's failure notification. The
  `pull_request` path stays advisory — a downstream repo still cannot block a
  merge here.

### Removed

- **The `site/` documentation app is retired** (#57,
  [ADR 0008](./docs/adr/0008-deploy-topology-and-advertised-urls.md)). It was a
  fumadocs/Next app built on every PR as a hard gate and deployed nowhere,
  holding a second, hand-maintained copy of the prose in `docs/` — a copy that
  had drifted **stale**: `protocol-surface.mdx` still documented
  `capabilities.upsert`/`.subscribe`/`.filters`/`.writes`, removed below, and
  `changelog.mdx` was 87 lines against this file's 353. The protocol's website
  is `macanderson/cgp-website`; the protocol's prose is `docs/*.md`, and now
  only that. Removes the `docs site builds` CI job and
  `tests/docs_site_witness_test.py`. The two genuinely published artifacts move
  out of `site/public/` to durable homes — `assets/badges/conformant.svg` and
  `registry/contextgraph-example-docs.report.json`; the third was a
  byte-identical mirror of `schema/contextgraph-envelope.schema.json` and is
  simply gone, along with the copy-sync check that guarded it.
- **Breaking:** `Capabilities.upsert`, `Capabilities.subscribe`, and
  `QueryCapability.filters` — negotiable at handshake but unreachable by any
  host. Wire-compatible; Rust API breaking (#5, #6, #11).

### Fixed

- **The three SDK example providers now serve verifiable file provenance.**
  The Python, TypeScript, and Go `example-docs` fixtures cited
  `file:///docs/…` paths that exist on no machine, with placeholder digests
  (`sha256:1111…`) that hash nothing. That was fine until
  `provenance-fixture-consistency` began re-reading the bytes every `file`
  provenance names: a provider serving no locally-readable provenance is
  skipped, and `conformance-external.sh` requires every check green, none
  skipped — so all three "sdk is a conformant implementation" CI jobs went
  red. Each example now ships the same two fixture files the Rust reference
  provider uses, resolves them to absolute `file://` URIs from its own
  location, and computes the real sha256 over the on-disk bytes at startup —
  the digest a host re-derives when it re-reads the file, so the check passes
  end to end (§6.2, §F5) and cannot drift: the digest is computed from the
  same bytes the URI names.
- **The conformance badge this repo hands to providers now resolves** (#57,
  [ADR 0008](./docs/adr/0008-deploy-topology-and-advertised-urls.md)).
  `docs/registry.md` and `docs/implementing-a-provider.md` told every
  conformant provider to paste `https://cgp.oxagen.sh/badges/conformant.svg`
  into its README. That host is served by a different repository and carries
  nothing under `/badges/`, so the badge 404ed for everyone who followed the
  instruction — on a page whose own copy claims the badge "never depends on
  this site's uptime." Both references now name this repo's GitHub-raw path,
  which GitHub serves as `image/svg+xml`. The same wall #58 hit from the schema
  side, in the places that fix did not reach.
- **The rule behind it is written down and gated.** ADR 0008 records the deploy
  topology — one Vercel project owns `contextgraphprotocol.org`,
  `cgp.oxagen.sh`, and `context-graph-protocol.vercel.app`, and it is
  Git-connected to `macanderson/cgp-website`, not here — and states the
  boundary it implies: advertise an artifact URL only on a host this repo
  serves, and never `vercel link` this checkout to the apex project (one
  `vercel --prod` from a linked checkout replaces the public apex, which has
  already happened in both directions).
  `.github/scripts/check-deploy-hygiene.py` enforces both offline, as a
  required CI check — nothing else in the build dereferences a URL, which is
  why the dead badge shipped silently.
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
