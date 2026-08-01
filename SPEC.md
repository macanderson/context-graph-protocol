# Context Graph Protocol (CGP) — normative specification

**Version:** `contextgraph/1.0-draft`

This document is the **single normative home** of the Context Graph Protocol.
A provider or host can be implemented from this document, the
[JSON Schema](./schema/contextgraph-envelope.schema.json), and the
[examples](./examples/) alone, without reading the reference Rust source.

> **Conformance language.** The key words **MUST**, **MUST NOT**, **SHOULD**,
> **SHOULD NOT**, and **MAY** are to be interpreted as described in
> [BCP 14 / RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) when, and only
> when, they appear in **bold**.

> **Normative vs informative.** Wire shapes, the requirement tables, the version
> rule, and the counting and format grammars are **normative**. Reference-host
> behaviour (timeouts, the safety factor, composition strategy) is
> **informative** and explicitly marked.

Every requirement has a stable anchor (`H1`, `B3`, `F5`, …). Cite them from code
comments and bug reports; they will not be renumbered within the
`contextgraph/1` family.

---

## 1. What CGP is

CGP specifies **context retrieval**: typed, budgeted, provenance-carrying,
consent-gated, conformance-verified frames that a host composes into a prompt.

It does **not** specify tool invocation — that is
[MCP](https://modelcontextprotocol.io)'s scope, and CGP will not absorb it. An
agent needing both composes them: CGP frames feed the prompt, MCP tools do the
work.

The unit of exchange is a **frame**, never a blob. A frame states what it is,
where it came from, what it costs, when it was true, and how to cite it — so a
host can budget, attribute, and verify rather than accept on faith.

---

## 2. Transport bindings

The **semantic layer** (frames, queries, capabilities) is defined independently
of its **transport binding**. One binding is defined in this revision.

### 2.1 NDJSON binding (normative)

Every message is a single JSON object — an *envelope* — tagged by a `type`
member.

- **stdio:** exactly one envelope per line, newline-delimited (NDJSON). An
  envelope **MUST NOT** contain a literal newline.
- **HTTP:** one envelope as the request body, one as the response body.

Envelope vocabulary: `handshake`, `handshake_ack`, `query`, `frames`,
`verify`, `verified`, `shutdown`, `error`.

A receiver **MUST** ignore an envelope member it does not recognise rather than
rejecting the message; a receiver **MUST NOT** reject an envelope solely because
its `type` is one it does not implement — it replies `error` with code
`bad_request` for a payload-bearing request it cannot serve, and ignores an
unrecognised notification. This is what lets the vocabulary grow additively
within the `contextgraph/1` family (§13).

**CGP is not JSON-RPC.** There is no `jsonrpc` member and no `method`/`params`
split. Its lifecycle is *informed* by MCP — a handshake negotiating version and
capabilities before any payload moves — but the framing is its own. A JSON-RPC
binding **MAY** be specified later as an alternate encoding of this same
semantic layer, without a new protocol family. See
[ADR 0002](./docs/adr/0002-request-correlation-and-the-json-rpc-question.md).

---

## 3. Handshake, versioning, and correlation

The host opens with `handshake`; the provider replies `handshake_ack` carrying
its protocol version, identity, and capabilities. **No query payload moves
before this exchange completes.**

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **H1** | A provider **MUST** reply to `handshake` with a `handshake_ack` whose `protocol_version` is in the same major family as the host's. | `handshake` check |
| **H2** | `provider.name` and `provider.version` **MUST NOT** be empty. | `handshake` check |
| **H3** | A version-family mismatch **MUST** be reported as a named error, never left to hang. | `versions_compatible`; `handshake` check (provider-facing); `host-version-reject` host-side scenario (§11.1) |
| **H4** | A provider declaring `capabilities.correlation` **MUST** echo a request's `id` verbatim on the corresponding `frames` or `error`. | `CorrelationMismatch`; `drop-correlation-id` witness |

### 3.1 Version strings

```abnf
version-string = "contextgraph/" major "." minor [ "-draft" ]
major          = 1*DIGIT
minor          = 1*DIGIT
```

The **major family** is the substring up to (not including) the first `.`. Two
versions interoperate **if and only if** they share a major family.
`contextgraph/1.0-draft` and `contextgraph/1.0` both belong to `contextgraph/1`
and interoperate; `contextgraph/2.0` does not.

This is what lets the freeze drop `-draft` without a flag day. An
implementation **SHOULD** compare major families rather than hardcoding a
version string.

### 3.2 Correlation

`query`, `frames`, and `error` **MAY** carry an `id`: an opaque host-generated
string, unique among the exchanges in flight on one connection.

- A host **MUST NOT** send an `id` to a provider that did not declare
  `capabilities.correlation`; such a provider is queried in lock-step and is
  fully conformant.
- An envelope with no `id` is a **notification** — it expects no reply. This is
  the shape a future push extension needs; no notification is defined in this
  revision.
- Correlation is negotiated **explicitly**, not by observation. A reply carrying
  no `id` would otherwise be ambiguous between "does not implement correlation"
  and "implements it incorrectly", and a guarantee whose violation is
  indistinguishable from legitimate behaviour cannot be checked.

---

## 4. Data flow and consent

`DataFlow` is the security-critical declaration, surfaced to the user at
install/consent time.

- **`reads`** — can see workspace content via query payloads.
- **`writes`** — durably persists data derived from what it receives (indexing
  payloads, retaining logs). A *consent-surface declaration*; it does not imply
  a host-callable write method, and none exists in this revision.
- **`egress`** — sends anything off the local machine.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **C1** | A host **MUST NOT** auto-enable a provider declaring `egress: true`. It **MUST** gate it behind explicit, named, revocable consent. | `ConsentStore` |
| **C2** | A host **MUST NOT** transmit a query payload to an egress provider before consent is recorded. | `Host::query_provider` |
| **C3** | A provider **SHOULD** declare `egress: true` honestly if data leaves the machine, directly or indirectly. | advisory — see C4 |
| **C4** | A host's HTTP transport **MUST** treat every non-loopback provider as egress regardless of its handshake claim. | HTTP transport |
| **C5** | A provider **MUST NOT** declare an off-machine `egress_scope` alongside `egress: false` — a local posture that names a destination content leaves is a contradiction a host rejects at the handshake. | `DataFlow::scopes_consistent` |
| **C6** | A host **MUST** refuse a query, with a typed error naming the scopes, when a provider declares off-machine egress scopes and any such scope has no recorded consent receipt; the payload **MUST NOT** be transmitted. | `ConsentStore::evaluate`; `scope-lie` witness |
| **C7** | A host's HTTP transport **MUST** use TLS for every non-loopback provider, and **MUST** refuse to transmit a query payload to a non-loopback provider over an unencrypted connection. | HTTP transport |
| **C8** | A host **MUST NOT** log, or place in an error surfaced off-machine, any bearer token, credential, or authorization header used to reach a provider. | HTTP transport |

C4 is the load-bearing one: C3 is a claim, and a protocol that trusted claims
about egress would have no security story at all. The transport overrides the
declaration because the transport *knows*.

### 4.1 Egress scopes and consent receipts

A provider **MAY** declare, alongside the boolean `egress`, the *egress scopes*
its served content falls under — a closed vocabulary that classes *where*
content goes, so consent can be recorded per destination rather than as one
undifferentiated bit. The four normative base classes are `local-only`,
`org-tenant`, `third-party-index`, and `third-party-model`; everything but
`local-only` is off-machine. The vocabulary is extensible by a namespaced custom
scope (`vendor:name`, a `:` with non-empty sides), and an unrecognised custom
scope is treated as off-machine — the conservative default is that an unknown
destination *leaves*, so a host never under-gates. A scope is declared at the
provider level and governs every frame that provider serves; there is no
per-frame scope.

When a host grants consent it records an append-only *consent receipt* pinning
the provider identity, the exact scope, the grantor, and the grant time — turning
"is this allowed?" into a durable "what left, to whom, who agreed, and when?".
The full model, the receipt shape, and the audit rationale are in
[`docs/context-reuse.md` §3](./docs/context-reuse.md). A receipt is a host-side
artifact, not a wire message: a provider implements nothing to make one possible.

### 4.2 Transport security (C7–C8)

The NDJSON binding over stdio is a local pipe with no network exposure. Over
HTTP, a non-loopback provider is reached across a network the host does not
control, so C7 requires TLS and C8 forbids leaking the credentials used to
authenticate to it. These bind the *host's* transport, not the provider, and
join C4 as rules the transport enforces regardless of what a provider claims:
a host that would send workspace content to a remote provider in cleartext, or
spill its bearer token into a log, has no egress-security story at all. A
provider **MAY** require a bearer credential; how a host obtains and stores one
is host machinery and outside this revision.

---

## 5. Query

```jsonc
{
  "type": "query",
  "id": "q1",
  "query": {
    "goal": "why does the retry loop give up",
    "query_text": "retry loop",          // optional
    "embedding": [0.01, -0.2],           // optional; see E1
    "kinds": ["snippet"],                // empty = any kind
    "anchors": ["file:///repo/src/net.rs"],
    "max_frames": 8,
    "max_tokens": 2000,
    "as_of": "2026-07-01T00:00:00Z"      // optional; see F4
  }
}
```

`anchors` are URIs the host considers focal (open files, mentioned symbols). A
graph-capable provider **SHOULD** boost frames within a small number of relation
hops of an anchor. The ranking algorithm stays provider-private; the *contract*
is only that anchors bias relevance.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **Q1** | When `kinds` is non-empty, a provider **MUST NOT** return a frame whose `kind` is outside it. Empty `kinds` means any kind. A provider serving none of the requested kinds returns zero frames, or replies `unsupported_kind`. | `kinds-filter` |

### 5.1 Why `kinds` binds (Q1)

`kinds` shipped as a request field with documented syntax and no stated
semantics, and not one implementation honored it — the reference provider and
all three SDKs declared `capabilities.query.kinds` and then ignored the filter,
returning whatever they had. That is the dead-capability surface
[ADR 0004](docs/adr/0004-dead-capability-surface.md) purged elsewhere, in its
subtler form: not an unreachable field, but a reachable one that silently does
nothing.

Specifying it rather than dropping it, because unlike `upsert`/`subscribe` the
surface is already load-bearing: `unsupported_kind` (§10) exists precisely to
answer "you asked for kinds I don't serve", which presupposes the filter binds.
A host that narrows to `["snippet"]` to keep prose out of a code-reasoning
prompt, and silently receives `doc` frames anyway, has had its budget spent on
content it explicitly excluded.

Q1 is a filter, not a ranking rule: it says which frames are *eligible*, and
leaves ordering provider-private like the rest of §5.

### 5.1 Embedding space (E1)

| # | Requirement |
| - | ----------- |
| **E1** | A host **MUST NOT** populate `query.embedding` unless its own embedding fingerprint is **exactly equal** to the provider's declared `capabilities.embeddings_fingerprint`. A provider receiving a vector whose length contradicts its declared dimension **SHOULD** reply `bad_request`. |

Fingerprint grammar: `<model-id>/<dimensions>[/<normalization>]`, e.g.
`bge-small-en-v1.5/384/l2`. Equality is exact rather than model-id-only, because
dimension and normalization both change what a vector *means*: a 384-dim
unnormalized vector sent to an index of 384-dim L2-normalized vectors yields
plausible-looking, meaningless scores — the silent wrongness CGP exists to make
loud.

---

## 6. Frames

```jsonc
{
  "id": "frm_retry",
  "kind": "snippet",
  "title": "net.rs L120-160",
  "content": "…",
  "uri": "file:///repo/src/net.rs",
  "score": 0.83,
  "token_cost": 42,
  "valid_from": "2026-01-01T00:00:00Z",
  "recorded_at": "2026-07-20T18:00:00Z",
  "provenance": [{ "type": "file", "uri": "…", "range": "L120-160",
                   "digest": "sha256:<64 hex>" }],
  "citation_label": "net.rs L120-160",
  "relations": [{ "rel": "code.calls", "target_uri": "…",
                  "display_name": "net::retry" }]
}
```

`kind` is one of `snippet`, `symbol`, `fact`, `doc`, `memory`, `episode`,
`graph`.

**Frame `content` is untrusted data.** It is evidence, not instruction.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **F1** | `score` **MUST** be in `[0, 1]`. | `frame-validity` |
| **F2** | `title` **MUST** be non-empty. | `frame-validity` |
| **F3** | `citation_label` **MUST** be non-empty — a host must be able to cite a frame by a human label, never a bare id. | `frame-validity` |
| **F4** | `valid_from`, `valid_to`, `recorded_at`, and `as_of` **MUST** match `YYYY-MM-DDTHH:MM:SS(.f+)?Z`. | `frame-validity` |
| **F5** | Provenance of kind `file` **MUST** carry a digest matching `sha256:<64 lowercase hex>`. | `frame-validity` |

### 6.1 Temporal profile (F4)

The profile is a **strict subset** of RFC 3339: uppercase `T`, uppercase `Z`,
UTC only. RFC 3339 also permits lowercase `t`, a space separator, and numeric
offsets; those are **not** conformant here. One spelling per instant means two
frames with the same instant compare equal as strings, which the dedup and
cache-key properties depend on. Naming it a subset rather than "RFC 3339" is
deliberate accuracy.

Semantics: `valid_from`/`valid_to` bound when the content was *true in the
world*; `recorded_at` is when the provider *learned* it. `as_of` pins retrieval
to an instant.

### 6.2 Digests (F5)

Grammar: `sha256:<64 lowercase hex>`. Lowercase is mandated, not conventional —
digests are compared byte-for-byte, and a case disagreement is indistinguishable
from tampering.

**Digested bytes:** the exact UTF-8 source bytes addressed by `uri` + `range` at
retrieval time, with **no normalization** (no line-ending translation, no
trailing-newline adjustment). Provenance without a `range` digests the whole
resource.

Only `file` provenance is held to F5: a `derivation` or `episode` link has no
addressable bytes, so requiring a digest of it would be theatre.

### 6.3 Frame identity (D1–D4)

A frame's stable identity is the triple *(provider id, frame id,
`content_digest`)*. `content_digest` is the provider-declared SHA-256 over the
frame's exact **inline** content bytes; it is opaque to the protocol
(`sha256:<hex>`) and is the spine shared by deterministic composition, usage
reports, and verification (§9). It is distinct from `canonical_content_hash`,
the SHA-256 over the *complete source* content that a `compact`/`reference`
frame carries so a resolved rehydration can be checked (§6.4).

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **D1** | `content_digest`, when present, **MUST** match `sha256:<64 lowercase hex>`. | `frame-validity` |
| **D2** | Two frames with the same *(provider id, frame id, `content_digest`)* **MUST** be treated as the same content; a host **MAY** dedup or reuse across queries on that basis. | host composition |
| **D3** | A frame whose `content_digest` is absent **MUST NOT** be reused unchecked across queries — a host re-queries or re-verifies it rather than trusting a stored copy. | host composition |
| **D4** | A `content_digest` is a claim about the *inline* bytes only; a host that reuses a frame's body across queries **SHOULD** confirm the identity still holds via `verify` (§9) before trusting it. | `verify` |

The identity rules and the reuse discipline they enable are developed in full in
[`docs/context-reuse.md` §1](./docs/context-reuse.md).

### 6.4 Representations (P1–P5)

A frame declares **how** it carries its content through `representation`, one of
`full`, `compact`, `reference`. Absent means `full`, so a frame emitted before
this field existed round-trips unchanged.

- **`full`** — the content is inline. The legacy default; the `representation`
  field is omitted on the wire.
- **`compact`** — an inline *transformed* rendering (a distillation, a
  truncation) travels with the frame, alongside the metadata to fetch or verify
  the original: `content`, `content_digest` (of the inline bytes),
  `canonical_content_hash` (of the full source), a `transform` identity, and a
  `content_ref`.
- **`reference`** — no inline content at all: only a `content_ref` handle and the
  `canonical_content_hash`, for a host that will rehydrate the full source.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **P1** | A `full` frame **MUST** carry `content` and **MUST NOT** carry `content_ref`, `transform`, or `canonical_content_hash`. | `frame-validity` |
| **P2** | A `compact` frame **MUST** carry all of `content`, `content_digest`, `canonical_content_hash`, `transform`, and `content_ref`. | `frame-validity` |
| **P3** | A `reference` frame **MUST** carry `content_ref` and `canonical_content_hash`, and **MUST NOT** carry `content` (not even `""`), `content_digest`, or `transform`. | `frame-validity` |
| **P4** | `token_cost` is the honest cost of the **inline** rendering only (B3, §7): a `reference` frame therefore declares `token_cost: 0`, and a `compact` frame declares the cost of its *distilled* inline bytes — never the full-source cost, which belongs in the separate optional `canonical_token_cost`. | `budget-honesty` |
| **P5** | A host **MUST NOT** populate `query.representation_preferences` with a representation the provider did not advertise in `capabilities.representations`; a provider asked for an unadvertised representation **SHOULD** reply `error` with code `unsupported_representation`, or fall back to `full`. | capability negotiation |

### 6.4.1 `content_ref`, resolve, and the 1.0 scope boundary

A `content_ref` is an **opaque resolver handle** — a `provider_id` naming the
provider that returned the frame, a handle `uri` distinct from the frame's own
`uri`, and an optional `expires_at`. It is the coordinate a host would hand back
to obtain the full source of a `compact` or `reference` frame.

**`context/resolve` is not defined in `contextgraph/1.0`.** There is no resolve
envelope, and a host has no protocol-defined operation that turns a `content_ref`
into bytes. Resolution is reserved for a `1.x` additive minor (§13); a design
sketch lives under [`docs/sketches/`](./docs/sketches/). The **Context Exchange
Provider profile** (issue #28,
[`docs/profiles/context-exchange-provider.md`](docs/profiles/context-exchange-provider.md))
takes that reservation up: it defines `context/resolve` as a **profile-scoped**
operation layered on the `contextgraph/1` family — *outside* the frozen `1.0`
core, which still ships no resolve operation — turning `capabilities.resolve`
from a forward-declaration into a callable contract within that profile's
capability envelope. This has three consequences a 1.0 implementer **MUST**
understand:

- A provider communicating over a transport binding (stdio, HTTP) **SHOULD NOT**
  return `reference` frames, because the host cannot rehydrate them over the wire
  in 1.0. It **SHOULD** return `compact` (which self-carries a usable inline
  rendering) or `full` instead. An **in-process** provider sharing the host's
  address space **MAY** use `reference`, since rehydration is then a host-internal
  concern outside this protocol.
- `capabilities.resolve` is a **forward-declaration**. A provider advertising
  `compact` or `reference` **MUST** set `resolve: true` — a promise it can
  re-serve the full content of what it references — but no `1.0` wire operation
  exercises that promise. The consistency rule (`compact`/`reference` ⇒
  `resolve`) is a shape check on the handshake, not an obligation a host can call.
- A host composing a `reference` frame it cannot rehydrate **MUST** treat its
  contribution as empty rather than fabricating content.

Freezing the representation *fields* now — they already travel on the wire — while
deferring the resolve *operation* keeps 1.0 honest: it ships no capability a host
cannot use, and the operation arrives later as a clean additive minor rather than
a breaking change.

---

## 7. Budget honesty

The flagship guarantee, and the one most easily faked.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **B1** | The sum of `token_cost` across returned frames **MUST NOT** exceed the query's `max_tokens`. | `budget-honesty` |
| **B2** | A host **MUST** drop, with a loud report, the frames of any provider violating B1 — never silently truncate them. | host budget audit |
| **B3** | `token_cost` **MUST** equal `ceil(utf8_byte_length(content) / 4)`. | `budget-honesty` |
| **B4** | The number of returned frames **MUST NOT** exceed `max_frames`. | `budget-honesty` |

### 7.1 Why B3 exists

Without it, B1 verified *arithmetic, not truth*: a provider declaring
`token_cost: 1` on a ten-thousand-token frame satisfied B1 perfectly while
destroying the host's real budget. B3 anchors each summand to bytes both parties
observe.

Equality is exact, with no tolerance band. Any band wide enough to absorb
genuine tokenizer disagreement is also wide enough to hide meaningful
under-reporting. A provider cannot "disagree" with a byte count.

### 7.2 Budget tokens are an accounting unit, not a tokenizer

A budget token is **not** a prediction of any model's tokenizer, and a host
**MUST NOT** treat one as one model token. The unit exists to make claims
comparable and verifiable across implementations, which no real tokenizer can do
without being mandated in every language.

It is honest about its bias: at ~4 bytes/token it tracks English prose, and it
**under-estimates** dense source code (~3–3.5 bytes/token) and CJK (~3
bytes/token). A host therefore maps its real model budget into budget tokens
with a safety factor. *(Informative: the reference host suggests 1.35.)*

**Scope:** the count covers `content` only — not `title`, `citation_label`,
provenance, or the host's own fences and labels. `content` is the one field the
provider controls whose exact bytes both sides observe, which is what makes a
byte-exact check possible. The host's rendering chrome is the host's cost to
budget.

*(Informative: exact tokenizer agreement may return as an additive 1.x
refinement — an optional handshake tokenizer id plus an optional exact count. It
does not disturb the floor established here.)*

### 7.3 Usage reports

Budget honesty (B1–B4) stops at the individual frame. A host that meters context
into a billing system — the usage-events → warehouse → invoice loop platforms
reselling agents run — needs the per-request roll-up, and every host inventing
that shape independently leaves context cost unauditable one level up from the
wire. A **usage report** is that roll-up: a host-side artifact, not a wire
envelope, whose total is pinned to the same byte-exact `token_cost` (B3) the
frames already carry, so the number a customer is billed is the number the
frames actually cost.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **UR1** | A host **MUST** be able to produce a usage report for any query it executed, whose `budget_consumed` equals the summed `token_cost` of the served frames it reports. The report **MUST** reference those frames by their `FrameId` (§6.3), so a billed total is walkable back to the exact `(provider id, frame id, content_digest)` triples behind it. | `contextgraph-host::FanOut::usage_report` |

The full report shape and its warehouse/billing metering path are described in
the companion [`docs/context-reuse.md` §2](./docs/context-reuse.md). `UR1` is a
distinct rule from the extensibility `U1` of §13 (ignore-unknown-members); the
two share no anchor.

---

## 8. Graph

CGP is named for the graph, and the graph is carried in `relations`: a graph
frame is **a node with its labelled edges**, not an ad-hoc serialization format.
`content` remains human-readable prose, consistent with every other kind, because
content is what goes into a prompt.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **G1** | Every `Relation` **MUST** carry a non-empty `display_name` — an edge is surfaced by human label, never a raw id. | `frame-validity` |
| **G2** | `target_uri` **MUST** be a non-empty URI. | `frame-validity` |
| **G3** | A provider declaring `capabilities.graph` **SHOULD** boost frames within a small number of relation hops of a query `anchor`. | `anchor-relevance` |
| **G4** | A frame is **anchored** by an anchor URI when its own `uri` equals that anchor (zero hops), or any of its `relations[].target_uri` does (one hop). A provider declaring `capabilities.graph` and given a non-empty `anchors` **MUST** return at least one anchored frame when it has one to serve, and **SHOULD** rank anchored frames above unanchored ones. | `anchor-relevance` |

### 8.2 Why anchoring needed a definition (G4)

G3 said providers should "boost frames within a small number of relation hops of
an anchor" and stopped there — it never said what an anchor is compared
*against*. Two conformant providers could reasonably match anchors against the
frame `uri`, against `relations[].target_uri`, or against neither, and no test
could distinguish a provider doing sophisticated graph traversal from one
ignoring `anchors` entirely. The reference fixture did the latter: it declared
`graph: false`, served frames with no relations at all, and every graph
requirement passed vacuously.

G4 gives "anchored" a decidable predicate — string equality on URIs, at zero or
one hop — so the SHOULD in G3 becomes something a suite can actually witness.
Deeper traversal stays provider-private: G4 is a floor on what must be *found*,
not a ceiling on how hard a provider may look.

### 8.1 Relation vocabulary (SHOULD)

The `rel` vocabulary is **open** — a host **MUST NOT** reject an unknown value.
These names are published so independent providers converge instead of each
inventing `calls` / `call` / `code.call`:

`code.calls` · `code.imports` · `code.defines` · `code.references` ·
`doc.documents` · `episode.follows`

Provider-specific edges belong under their own namespace (`myindex.owns`), which
keeps the shared namespace meaningful.

### 8.3 Multi-hop traversal is deferred

**A wire operation for walking edges beyond one hop is not defined in
`contextgraph/1.0`.** G4 pins the one traversal semantics a suite can witness —
the zero-or-one-hop *anchored* predicate — and stops there. There is no
`neighbors` request in 1.0: a host receives frames with their edges from a
`query` and composes them; it never asks a provider to return a node's
neighborhood to a given depth. Freezing that operation now, with no host
emitting it, would reintroduce the dead-capability surface §8.2 and
[ADR 0004](./docs/adr/0004-dead-capability-surface.md) work to avoid. When a
concrete traversal consumer forces its design it can land as an additive minor,
gated on a new `capabilities.neighbors`, with `depth: 1` defined to return
exactly the G4 anchored set so nothing this freeze witnessed is invalidated. A
design sketch lives under
[`docs/sketches/context-neighbors.md`](./docs/sketches/context-neighbors.md).

---

## 9. Verification

A host that holds frames from an earlier query can ask the provider whether they
are still current, instead of blindly re-querying. This is the pull half of
staleness handling; a push extension (a provider volunteering invalidations) is
a notification-shaped 1.x addition (§13) and is not defined here.

```jsonc
// host → provider
{ "type": "verify",
  "request": { "frames": [
    { "provider_id": "code-graph", "frame_id": "frm_retry",
      "content_digest": "sha256:<64 hex>" }
  ] } }

// provider → host
{ "type": "verified",
  "response": { "verdicts": [
    { "frame": { "provider_id": "code-graph", "frame_id": "frm_retry",
                 "content_digest": "sha256:<64 hex>" },
      "status": "stale",
      "replacement_digest": "sha256:<64 hex>" }
  ] } }
```

A verify request carries frame **identities** (§6.3), never bodies. Each verdict
echoes the identity it answers *in full*, so a host correlates by matching rather
than by position and a provider that reorders or omits entries cannot shift a
`valid` onto the wrong frame.

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **V1** | A `verify` request **MUST** carry frame identities only — no frame bodies. A host **SHOULD** include only identities carrying a `content_digest`; a digest-less frame cannot be revalidated and is re-queried instead. | `verify-honesty` |
| **V2** | A provider declaring `capabilities.verify` **MUST** answer a `verify` with a `verified` reply. A requested identity that comes back with no verdict **MUST** be treated by the host as `unknown`. | `verify-honesty`; `rubber-stamp-verify`, `hollow-verify` witnesses |
| **V3** | A verdict is one of `valid`, `stale`, `gone`, `unknown`. A host **MUST** reuse a held frame body **only** on `valid`; `unknown` **MUST NOT** be read as validity. Reuse requires a positive answer, never the absence of a negative one. | `verify-honesty` |
| **V4** | A `stale` verdict **MAY** carry a `replacement_digest` — the provider's current digest for the frame, a digest never a body. A host **MUST NOT** keep serving its stored copy of a `stale` or `gone` frame. | `verify-honesty` |

A provider that does not declare `capabilities.verify` is queried afresh each
time and stays fully conformant — verification is an optimisation a host earns by
handshake, never an assumption. When a provider declares `capabilities.correlation`,
a `verify`/`verified` pair is correlated by `id` exactly as `query`/`frames` are
(H4). The verdict semantics and the reuse discipline are developed in
[`docs/context-reuse.md` §4](./docs/context-reuse.md).

---

## 10. Errors

```jsonc
{ "type": "error", "id": "q1", "code": "unsupported_kind",
  "message": "this provider serves only 'doc' frames" }
```

`code` is for the machine; `message` is for whoever reads the log. Both are
carried — neither replaces the other.

| code | meaning | host reaction |
| --- | --- | --- |
| `bad_request` | malformed or unintelligible query | do not retry |
| `unsupported_kind` | requested kinds not served | narrow or skip |
| `unsupported_representation` | requested representation not offered | re-request `full` or skip |
| `incompatible_version` | handshake version families do not share a major (H3) | do not retry; the provider is unusable |
| `budget_unsatisfiable` | budget too small for any meaningful frame | raise budget or skip |
| `unavailable` | transient overload, backing store down | retry with backoff |
| `shutting_down` | provider is tearing down | re-spawn or drop |
| `internal` | provider fault | report, count against health |

`incompatible_version` is the named error H3 requires — a version-family mismatch
is permanent, so a host **MUST NOT** read it as retryable. `unsupported_representation`
is what a provider replies when a host requests a representation it did not
advertise (§6.4).

| # | Requirement |
| - | ----------- |
| **X1** | The `code` vocabulary is **open**. An unrecognised code **MUST** be treated as `internal`. |
| **X2** | An absent `code` **MUST** be treated as `internal`. |

X1 and X2 both default to the conservative reading: a host must never infer
"safe to retry" from a code it does not understand, or from silence. This is
also what lets the vocabulary grow in a 1.x minor without breaking deployed
hosts.

---

## 11. Robustness

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **R1** | A provider **MUST NOT** crash on a malformed line or bad request. It **SHOULD** reply `error` with code `bad_request`. | `malformed-input-tolerance` |
| **R2** | A provider **MUST** tear down cleanly on `shutdown`. | `shutdown-clean` |
| **R3** | A host **MUST** treat frame `content` as untrusted data — delimited as quoted material, never executed as instructions. | `host-content-quoting` + `host-composition-audit`; reference [`compose_for_prompt`](docs/composing-frames-into-a-prompt.md) |

A host realizing R3 **SHOULD** follow the reference prompt-composition module
(global-budget split, cross-provider dedup, value-aware placement, fenced
injection-resistant rendering, and an audit record explaining every drop) —
[Composing frames into a prompt](docs/composing-frames-into-a-prompt.md).

### 11.1 Known enforcement gaps

Listing these is deliberate. A conformance suite that quietly omitted the rules
it cannot check would be exactly the self-attestation this project rejects.

The **host-side harness** (`contextgraph-conformance`'s `host_conformance`
module, issue #14) closes most of the host-binding gaps that once lived here. It
drives the reference host against adversarial providers — in-process ones, plus
short-lived stdio child fixtures for the transport-level scenarios — the
host-side equivalent of the provider fixture's `--misbehave` modes, and asserts
the host: **H3** rejects a `handshake_ack` from a mismatched major family with a
named `VersionMismatch`, never a hang (the host-side dual of §3's provider-facing
`handshake` check — that check asserts a provider *replies* with a well-formed
ack; this asserts the *host* *refuses* a wrong-family one, and promptly, driving
the handshake under an explicit timeout so a stall is a distinct failure);
**B2** drops an over-budget provider with a report; **B4** drops a frame-flooding
one; **C1/C2** never queries, nor transmits a payload to, an unconsented egress
provider; **C6** refuses an unreceipted off-machine scope with a typed error;
**F5-bytes** verifies a `file`-provenance digest against the re-read source over a
trusted local fixture (via `contextgraph_host::verify`, issue #12); **R3**
delimits frame `content` as quoted material inside a fence; and **crash
isolation** — a provider that dies mid-query surfaces as `ProviderCrashed` and is
excluded while a healthy provider fanned out concurrently beside it still returns
its frames, so one leg's crash never poisons a `query_all`. Run it:
`contextgraph-inspect host` (CI: `host-conformance.sh`).

What remains genuinely unchecked:

- **C4, C7, C8 — the HTTP transport rules.** These bind the host's HTTP client.
  **C7 (TLS for non-loopback) and C8 (credentials never logged) are now enforced
  and unit-tested in the reference host** (issue #13): the transport refuses a
  plaintext `http://` connection to a non-loopback provider with a typed
  `HostError::InsecureTransport` *before any bytes leave the host*, keeps the
  loopback `http://` exception, attaches a bearer credential via reqwest's
  `bearer_auth` rather than a format string, and renders every `Credential` as a
  fixed `Credential(<redacted>)` placeholder in both `Debug` and `Display` so it
  cannot spill into a log or a panic — each covered by a `contextgraph-host` unit
  test. What remains genuinely unchecked is full *live-TLS-peer* conformance:
  exercising the handshake, TLS negotiation, and credential exchange end-to-end
  against a real non-loopback TLS peer — and witnessing C4's treat-as-egress
  override over that same peer — needs a network peer the in-process harness
  cannot stand up, and stays the host-side harness's next increment.
- **R3 breakout-resistance is escaping, not an unguessable fence — a design
  choice, no longer a gap.** The reference `compose_context` neutralizes a
  content-embedded `<frame`/`</frame>` token and escapes fence attributes, so
  content cannot terminate the block that quotes it or forge a sibling frame
  (issue #63). Escaping rather than a random delimiter is deliberate:
  composition's contract is a byte-stable prompt prefix (§1 of
  `docs/context-reuse.md`), and a per-turn nonce would forfeit the provider
  prompt cache to buy a property escaping already provides. The *rest* of the
  composition module — global-budget split, cross-provider dedup, value-aware
  placement, and an audit record — is now implemented
  (`contextgraph_host::compose::compose_for_prompt`) and checked by the
  `host-composition-audit` host-conformance check (issue #15), so R3 is covered
  end to end rather than residual.
- **F5-bytes verifies a host-trusted source, not any provider-named `uri`.** The
  verifier re-reads a path the host chooses to trust; automatically re-reading an
  arbitrary `uri` a provider supplies is a capability decision (path confinement,
  consent) that stays future work.

---

## 12. Conformance

"CGP conformant" means **green on `contextgraph-conformance` for your declared
capability set** — a checkable claim, not a self-attestation.

Run it:

```bash
contextgraph-inspect stdio -- ./your-provider
contextgraph-inspect stdio --json -- ./your-provider   # machine-readable
```

The suite is adversarial by construction: the bundled reference provider has
`--misbehave` modes that each break exactly one guarantee, and CI asserts every
mode is **caught**. A suite that only ever passes proves nothing about its
ability to catch a broken provider.

---

## 13. Extensibility and forward compatibility

The freeze drops `-draft` without a flag day (§3.1) only if a `contextgraph/1.0`
implementation can safely receive a message a later `1.x` peer emits. That
requires a stated rule for what "receive" does with surface the receiver was not
built to know about. These rules are normative; they are what make the additive
bias of §15 real rather than aspirational.

| # | Requirement |
| - | ----------- |
| **U1** | A receiver **MUST** ignore an object member it does not recognise, in any envelope, capability set, frame, or nested object — it **MUST NOT** reject the message on that basis. This is what lets a `1.x` minor add an optional field that a `1.0` peer harmlessly drops. |
| **U2** | The `FrameKind` set (`snippet`, `symbol`, `fact`, `doc`, `memory`, `episode`, `graph`) is **closed within a major family**; a new kind is a `1.x` addition. A host that receives an unrecognised `kind` **MUST** treat the frame as opaque evidence — it **MAY** ignore it, but **MUST NOT** crash. New *open* vocabularies (`rel`, error `code`, `egress_scope`) grow without a version bump; a receiver **MUST NOT** reject an unknown value in any of them (§8.1, §10 X1, §4.1). |
| **U3** | Names containing a `:` are **reserved for namespacing**: a vendor-specific `rel`, `egress_scope`, or error `code` **MUST** be namespaced (`vendor:name`, non-empty on both sides) so it can never collide with a base value this spec defines or later reserves. Unprefixed names in these vocabularies belong to the protocol. |
| **U4** | A field this spec defines is never repurposed within `contextgraph/1`: its name, type, and meaning are stable. A field that is superseded is **deprecated** — kept parseable and documented as deprecated for the life of the major family — never deleted or redefined. Deletion or redefinition requires a new major family (§3.1). |

**Unknown-field handling is load-bearing, not a courtesy.** The reference types
ignore unknown members on deserialization; a stricter validator (for authoring or
CI) **MAY** reject them, but a validator on the *interop* path — deciding whether
to accept a peer's message — **MUST** follow U1. The JSON Schema in this
repository is published in an authoring-strict profile (`additionalProperties:
false`) to catch typos in fixtures; that strictness is a lint, not the interop
contract, and U1 governs the wire.

Together U1–U4 are the mechanism behind the one-line promise that the freeze
"drops `-draft` without a flag day": a `1.0` peer and a `1.5` peer interoperate
because the `1.0` peer ignores what it does not know, the vocabularies it does
know only ever grew, and nothing it relied on was moved out from under it.

The **Context Exchange Provider profile** (issue #28,
[`docs/profiles/context-exchange-provider.md`](docs/profiles/context-exchange-provider.md))
applies these same rules to its record layer:
[`schema/contextgraph-lifecycle-record.schema.json`](schema/contextgraph-lifecycle-record.schema.json)
is a second authoring-strict schema (`unevaluatedProperties: false`) that is a
lint, not the interop contract; `record_kind` is closed within `lifecycle/1.0`
(a new kind is a `lifecycle/1.x` addition, the U2 discipline); and record
`extensions` and `record_links.rel` follow the U3 namespacing rule.

---

## 14. Attribution

Provenance (§6.2) answers *where an item came from*. Attribution answers the
other half of the same question — *what it did* — so that including a frame is
an evaluable decision rather than an act of faith. Cost without outcome prompts
no decision ("this frame cost 400 tokens"), and outcome without cost prompts the
wrong one ("this frame was never cited" — it cost four).

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **A1** | A frame's attribution handle **is** its `FrameId` (§6.3) — the same `(provider id, frame id, content_digest)` triple used for composition, dedup, usage reports (§7.3, UR1), and `verify` (§9). An implementation **MUST NOT** mint a separate attribution id. | `contextgraph-types::attribution` |
| **A2** | A host reporting attribution **MUST** report `selected`, `rendered`, and `cited` as independent observations, not a single score. `cited` **MUST** mean the model's output referred to the frame, an observable fact — never an inference that the frame *influenced* the output. | `contextgraph-types::attribution` |
| **A3** | An attribution record **MUST** be reconcilable: coherent (`cited` ⇒ `rendered` ⇒ `selected`) and naming a frame the paired usage report actually billed. | `AttributionReport::is_reconcilable` |

### 14.1 Why one id, and three booleans

**One id (A1).** A second identity would be free to disagree with the first, and
a disagreement between *the frame that was billed* and *the frame that was
cited* is precisely the confusion attribution exists to remove.

**Three booleans (A2).** They are separately observable and collapse badly. The
case that matters most is a frame that was `selected` and `rendered` but never
`cited`: the host paid its tokens, the model read it, and it changed nothing.
A `used`/`unused` flag cannot express that, and a 0–1 usefulness score would
invent a precision nobody measured. `selected` without `rendered` is a third
distinct state — ranked in, then dropped by budget packing — and it is neither
credit nor debit, because it was never shown.

Attribution is a **host self-report**. Unlike `token_cost`, which §B3 anchors to
a canonical rule anyone can recompute, there is no way to check a host's claim
that a frame was cited; the guarantee is scoped to hosts that want honest
measurement, not enforced against ones that don't.

**Not on the wire.** There is no `context/feedback` method and no
`Capabilities.feedback` in this revision. The vocabulary is specified because it
has to be shared for scores to be comparable across implementations; the
transport is deferred to a 1.x additive minor
(`docs/sketches/attribution-feedback.md`). Shipping a negotiated feedback method
with no provider consuming it would recreate exactly the dead capability surface
[ADR 0004](docs/adr/0004-dead-capability-surface.md) removed — and the asymmetry
favors waiting: adding the method later is family-safe, removing a dead one is
not.

---

## 15. Changing this specification

See [GOVERNANCE.md](./GOVERNANCE.md). A normative change needs an issue, a PR
updating this document and `CHANGELOG.md`, and a **witness** — a conformance
check or a wire example. The bias is additive: a new optional field is a minor
change; a removed or renamed field requires a new major family (§13 U4).

Pre-freeze, `docs/stability.md` permits breaking changes on a `0.x → 0.y` bump.
Decisions taken under that latitude are recorded in [`docs/adr/`](./docs/adr/).
