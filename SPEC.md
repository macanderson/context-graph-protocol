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
`shutdown`, `error`.

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
| **H3** | A version-family mismatch **MUST** be reported as a named error, never left to hang. | `versions_compatible`; `handshake` check |
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

C4 is the load-bearing one: C3 is a claim, and a protocol that trusted claims
about egress would have no security story at all. The transport overrides the
declaration because the transport *knows*.

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
| **G3** | A provider declaring `capabilities.graph` **SHOULD** boost frames within a small number of relation hops of a query `anchor`. | advisory |

### 8.1 Relation vocabulary (SHOULD)

The `rel` vocabulary is **open** — a host **MUST NOT** reject an unknown value.
These names are published so independent providers converge instead of each
inventing `calls` / `call` / `code.call`:

`code.calls` · `code.imports` · `code.defines` · `code.references` ·
`doc.documents` · `episode.follows`

Provider-specific edges belong under their own namespace (`myindex.owns`), which
keeps the shared namespace meaningful.

---

## 9. Errors

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
| `budget_unsatisfiable` | budget too small for any meaningful frame | raise budget or skip |
| `unavailable` | transient overload, backing store down | retry with backoff |
| `shutting_down` | provider is tearing down | re-spawn or drop |
| `internal` | provider fault | report, count against health |

| # | Requirement |
| - | ----------- |
| **X1** | The `code` vocabulary is **open**. An unrecognised code **MUST** be treated as `internal`. |
| **X2** | An absent `code` **MUST** be treated as `internal`. |

X1 and X2 both default to the conservative reading: a host must never infer
"safe to retry" from a code it does not understand, or from silence. This is
also what lets the vocabulary grow in a 1.x minor without breaking deployed
hosts.

---

## 10. Robustness

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **R1** | A provider **MUST NOT** crash on a malformed line or bad request. It **SHOULD** reply `error` with code `bad_request`. | `malformed-input-tolerance` |
| **R2** | A provider **MUST** tear down cleanly on `shutdown`. | `shutdown-clean` |
| **R3** | A host **MUST** treat frame `content` as untrusted data — delimited as quoted material, never executed as instructions. | host contract *(see gap below)* |

### 10.1 Known enforcement gaps

Listing these is deliberate. A conformance suite that quietly omitted the rules
it cannot check would be exactly the self-attestation this project rejects.

- **R3 is not machine-checked.** The suite tests providers; R3 binds hosts.
  Host-side conformance is issue #14.
- **C1/C2/C4 and B2 are host-binding** and likewise unchecked by the
  provider-facing suite.
- **F5 checks digest _grammar_, not whether the digest matches the bytes.**
  End-to-end verification requires the host to re-read the source, which is
  issue #12's remaining half.

---

## 11. Conformance

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

## 12. Changing this specification

See [GOVERNANCE.md](./GOVERNANCE.md). A normative change needs an issue, a PR
updating this document and `CHANGELOG.md`, and a **witness** — a conformance
check or a wire example. The bias is additive: a new optional field is a minor
change; a removed or renamed field requires a new major family.

Pre-freeze, `docs/stability.md` permits breaking changes on a `0.x → 0.y` bump.
Decisions taken under that latitude are recorded in [`docs/adr/`](./docs/adr/).
