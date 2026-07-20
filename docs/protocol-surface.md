# Context Graph Protocol protocol surface

This is the normative shape of the Context Graph Protocol as bound to
Rust types by [`contextgraph-types`](https://crates.io/crates/contextgraph-types). Every type
below lives in that crate, round-trips through `serde_json`, and *is* the
protocol — there is no separate IDL. Field-level doc comments in the crate
are the ultimate source of truth; this page is a guided tour.

Protocol version: `PROTOCOL_VERSION = "contextgraph/1.0-draft"` (`contextgraph-types/src/lib.rs`).
See [`stability.md`](./stability.md) for what "draft" means and when it
freezes.

> **Conformance language.** The key words **MUST**, **MUST NOT**, **SHOULD**,
> and **MAY** in this document, the overview, and the build guide are to be
> interpreted as described in [BCP 14 / RFC 2119](https://www.rfc-editor.org/rfc/rfc2119)
> when they appear in **bold**. Lowercase "must" / "should" are used in the
> ordinary sense. The consolidated, authoritative list of conformance
> requirements is the [§ Conformance requirements](#conformance-requirements)
> section at the end of this page.


## The three modules

`contextgraph-types` is organized into three modules, re-exported from the crate root:

- [`capability`](#handshake--capability) — what a provider is and does,
  negotiated at the handshake.
- [`query`](#context-query) — the retrieval request/response shape.
- [`frame`](#context-frame) — the unit of exchange a provider returns.

## Handshake / capability

A provider identifies itself and what it does with data before a host ever
sends it a query.

```rust
pub struct DataFlow {
    pub reads: bool,   // can see workspace content via query payloads
    pub writes: bool,  // persists context/upsert writes
    pub egress: bool,  // sends anything off the local machine
}

pub struct ProviderInfo {
    pub name: String,
    pub version: String,
    pub data_flow: DataFlow,
}

pub struct Capabilities {
    pub query: QueryCapability,
    pub upsert: bool,
    pub graph: bool,
    pub embeddings_fingerprint: Option<String>,
    pub subscribe: bool,
}

pub struct QueryCapability {
    pub kinds: Vec<String>,   // e.g. ["doc", "snippet"] — see FrameKind below
    pub filters: Vec<String>,
}
```

`DataFlow.egress` is the security-critical field. **A conforming host MUST
NOT auto-enable a provider that declares `egress: true`** — it must gate that
provider behind an explicit, one-time consent that names what leaves
(enforced by `contextgraph-host`'s `ConsentStore`; see
[Implementing a provider](./implementing-a-provider.md)).

## Context query

A request to a provider for context frames relevant to a goal. Every query
carries a token budget; a conforming provider never returns more than it and
never lies about the cost.

```rust
pub struct ContextQuery {
    pub goal: String,                    // the task/turn goal driving retrieval
    pub query_text: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub kinds: Vec<FrameKind>,           // empty = "give me your best frames of any kind"
    pub anchors: Vec<String>,            // open files / mentioned symbols, for proximity scoring
    pub max_frames: u32,
    pub max_tokens: u32,
    pub as_of: Option<String>,           // pin retrieval to a point in time (bi-temporal facts)
}

pub struct ContextQueryResult {
    pub frames: Vec<ContextFrame>,
    pub truncated: bool,                 // true if more candidates existed than fit the budget
    pub dropped_estimate: Option<u32>,
}
```

`ContextQueryResult` carries two helper methods any host can use:

- `total_token_cost() -> u64` — sum of `token_cost` across returned frames.
- `respects_budget(max_tokens: u32) -> bool` — whether that sum stayed within
  the query's budget. `contextgraph-host`'s fan-out router calls this on every
  response and drops (with a loud report) any provider whose frames fail it —
  a provider that returns more tokens than it claimed is exhibiting
  **budget dishonesty**, and its frames are never trusted into a prompt.

## Context frame

The unit of exchange returned from a query. Frames, never blobs, carry
relevance, cost, and provenance so a budgeting, citing host can compose
sources honestly.

```rust
pub enum FrameKind {
    Snippet, Symbol, Fact, Doc, Memory, Episode, Graph,
}

pub struct ContextFrame {
    pub id: String,                      // provider-scoped, stable for dedup across queries
    pub kind: FrameKind,
    pub title: String,                   // human label — never a bare uuid
    pub content: String,                 // untrusted data — host must delimit as quoted material
    pub uri: Option<String>,
    pub score: f32,                      // provider-normalized relevance, [0, 1]
    pub token_cost: u32,                 // honest, conformance-audited token cost
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub recorded_at: Option<String>,
    pub provenance: Vec<Provenance>,
    pub citation_label: Option<String>,
    pub embedding: Option<FrameEmbedding>,
    pub relations: Vec<Relation>,
}

pub struct Provenance {
    pub kind: String,           // e.g. "file", "derivation", "episode" (serialized as "type")
    pub uri: Option<String>,
    pub range: Option<String>,
    pub digest: Option<String>,
    pub method: Option<String>,
    pub by: Option<String>,
}

pub struct Relation {
    pub rel: String,
    pub target_uri: String,
    pub display_name: Option<String>,    // a graph edge is surfaced by human label, never a raw id
}

pub struct FrameEmbedding {
    pub fingerprint: String,             // the vector payload itself is elidable
    pub vector: Option<Vec<f32>>,
}
```

Two contract points worth calling out explicitly, because the conformance
suite checks both:

- **`score` must be in `[0, 1]`.** `ContextFrame::has_valid_score()` is the
  cheap self-check any provider or host can run; `contextgraph-conformance`'s
  `frame-validity` check enforces it against real providers.
- **`title` and `citation_label` must never be empty.** A host must be able
  to cite a frame by a human label — an empty or missing citation label is a
  conformance failure, not a cosmetic gap. (Whole-platform convention: raw
  ids are never the primary on-screen identifier.)

## Wire framing (defined in `contextgraph-host`, not `contextgraph-types`)

`contextgraph-types` defines the payload shapes above; `contextgraph-host::wire::Envelope`
defines how they're framed on the wire — newline-delimited JSON (NDJSON), one
`serde_json` value per line over stdio, or one JSON body per streamable-HTTP
request/response. See [Implementing a provider](./implementing-a-provider.md)
for the full envelope vocabulary (`handshake` / `handshake_ack` / `query` /
`frames` / `shutdown` / `error`) and the version-compatibility rule. See
[`examples/`](../examples/) for diffable wire transcripts of a complete session,
or the [machine-readable JSON Schema](../schema/contextgraph-envelope.schema.json) to
validate messages in any language.

## Machine-readable schema

The wire shapes above are captured as a [JSON Schema (Draft 2020-12)](../schema/contextgraph-envelope.schema.json).
The root schema validates a single envelope (one message per NDJSON line / per
HTTP body); every payload type is also exposed under `$defs` for granular
validation. Because the wire format is JSON, any standard JSON Schema validator
works — `ajv` (JS/TS), Python `jsonschema`, the Rust `jsonschema` crate, or Go
`gojsonschema` — with no IDL compiler or language lock-in.

The schema encodes the structural conformance rules: required fields, `score ∈
[0,1]`, non-empty `title`/`citation_label`/`id`, the `FrameKind` enum, the
`contextgraph/MAJOR.MINOR(-draft)?` version-string pattern, u32 ranges, and the
`Provenance.kind` → `"type"` serialization rename. A message that validates
against the schema is structurally conformant; the `contextgraph-conformance` suite adds
the behavioral checks (budget honesty, malformed-input tolerance, clean
shutdown) that a schema alone cannot express.

Run `python3 schema/validate-examples.py` to check the bundled examples against
the schema — it doubles as a usage reference for wiring your own validator.

---

## Conformance requirements

This section is the consolidated, authoritative list of what a conforming
provider and host **MUST** do. The `contextgraph-conformance` suite checks the
provider-side requirements; a host built on `contextgraph-host` enforces the
host-side requirements. Bold keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

### Handshake and versioning

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| H1 | A provider **MUST** reply to `handshake` with a `handshake_ack` whose `protocol_version` is in the same major family as the host's. | `handshake` conformance check |
| H2 | The `provider.name` and `provider.version` fields **MUST NOT** be empty. | `handshake` conformance check |
| H3 | A version-family mismatch **MUST** be reported to the host as a named error, not left to hang. | `contextgraph-host::wire::versions_compatible` |

### Data flow and consent

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| C1 | A conforming host **MUST NOT** auto-enable a provider that declares `data_flow.egress: true`. It **MUST** gate that provider behind explicit, named, revocable consent. | `ConsentStore` gate in `contextgraph-host` |
| C2 | The host **MUST NOT** transmit a query payload to an `egress` provider before consent is recorded. | `Host::query_provider` gate |
| C3 | A provider **SHOULD** declare `egress: true` honestly if it sends data off the local machine, directly or indirectly. | advisory; the host cannot rely solely on the claim — see C4 |
| C4 | `contextgraph-host`'s HTTP transport **MUST** treat every remote provider as `egress` regardless of its handshake claim. | `contextgraph-host` HTTP transport |

### Frame validity

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| F1 | Every frame's `score` **MUST** be in the range `[0, 1]`. | `frame-validity` conformance check; `ContextFrame::has_valid_score()` |
| F2 | Every frame's `title` **MUST** be non-empty. | `frame-validity` conformance check |
| F3 | Every frame's `citation_label` **MUST** be non-empty. | `frame-validity` conformance check |

### Budget honesty

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| B1 | The sum of `token_cost` across a provider's returned frames **MUST NOT** exceed the query's `max_tokens`. | `budget-honesty` conformance check; `ContextQueryResult::respects_budget` |
| B2 | A host **MUST** drop (with a loud report) the frames of any provider that violates B1, rather than silently truncating them. | `contextgraph-host::Host` budget audit |

### Robustness

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| R1 | A provider **MUST NOT** crash on a malformed line or a bad request. It **SHOULD** reply `error` instead. | `malformed-input-tolerance` conformance check (stdio) |
| R2 | A provider **MUST** tear down cleanly on `shutdown` (stdio: exit; HTTP: no further requests expected). | `shutdown-clean` conformance check |
| R3 | Frame `content` **MUST** be treated as untrusted data by the host — delimited as quoted material, never executed as instructions. | `contextgraph-host` host contract |

## Version strings

The protocol version string has the grammar:

```abnf
version-string = "contextgraph/" major "." minor [ "-draft" ]
major          = 1*DIGIT
minor          = 1*DIGIT
```

The **major family** is the substring up to (but not including) the first `.`
— e.g. the family of `contextgraph/1.0-draft` is `contextgraph/1`.

Two version strings interoperate if and only if they share a major family.
`contextgraph/1.0-draft` and `contextgraph/1.0` both belong to family `contextgraph/1` and interoperate;
`contextgraph/2.0` does not interoperate with either. The `-draft` suffix marks a
not-yet-frozen version within a family and does not affect interoperability.
This rule is implemented by `contextgraph-host::wire::versions_compatible`; an
out-of-Rust implementation **SHOULD** compare major families directly rather
than hardcoding a specific version string.

See [`stability.md`](./stability.md) for the crate-version vs. protocol-version
distinction and the draft-to-freeze model, and [`GOVERNANCE.md`](../GOVERNANCE.md)
for who decides the freeze.
