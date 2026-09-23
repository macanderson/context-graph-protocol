# Context Graph Protocol (CGP) — normative specification

**Version:** `contextgraph/1.0`

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
`contextgraph/1.0` and `contextgraph/1.1` both belong to `contextgraph/1`
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
| **C3** | A provider **MUST** declare `egress: true` if data leaves the machine, directly or indirectly — including through a process, service, or proxy it relays to. | unverifiable over stdio (§4.3, §11.1); overridden over HTTP by C4 |
| **C4** | A host's HTTP transport **MUST** treat every non-loopback provider as egress regardless of its handshake claim. | HTTP transport |
| **C5** | A provider **MUST NOT** declare an off-machine `egress_scope` alongside `egress: false` — a local posture that names a destination content leaves is a contradiction a host rejects at the handshake. | `DataFlow::scopes_consistent` |
| **C6** | A host **MUST** refuse a query, with a typed error naming the scopes, when a provider declares off-machine egress scopes and any such scope has no recorded consent receipt; the payload **MUST NOT** be transmitted. | `ConsentStore::evaluate`; `scope-lie` witness |
| **C7** | A host's HTTP transport **MUST** use TLS for every non-loopback provider, and **MUST** refuse to transmit a query payload to a non-loopback provider over an unencrypted connection. | HTTP transport |
| **C8** | A host **MUST NOT** log, or place in an error surfaced off-machine, any bearer token, credential, or authorization header used to reach a provider. | HTTP transport |

C4 is the load-bearing one: C3 is a claim, and a protocol that trusted claims
about egress would have no security story at all. The transport overrides the
declaration because the transport *knows*. Over stdio it does not, and §4.3
states what the consent guarantee is worth there.

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

### 4.3 What the transport cannot see (C3 over stdio)

C4's argument has a converse. Over HTTP the transport knows that a query leaves
the host, so it overrides the provider's claim. Over stdio it knows nothing of
the kind: a stdio provider is a child process with its own sockets, and nothing
on the NDJSON pipe reveals whether it opens one. There, `egress: false` is C3 and
only C3, a declaration the host acts on and cannot check. So the consent
guarantee (C1, C2) holds as follows:

- **An HTTP provider** is never queried without recorded consent, whatever it
  declares (C4).
- **A stdio or in-process provider declaring `egress: true`** is never queried
  without recorded consent (C1, C2).
- **A stdio provider declaring `egress: false`** is queried without consent,
  because it has said nothing leaves. If that is a lie, it breaks C3, and nothing
  in the protocol, the reference host, or the conformance suite detects it
  (§11.1).

**C3 is a MUST even though nobody can check it.** Strength and checkability are
separate properties. A dishonest `egress: false` is a conformance violation a
deployment can act on, by contract, by delisting, or in a registry. That is how
every unverifiable but load-bearing declaration is handled, and A2's `cited` is
the precedent inside this specification. Leaving it a SHOULD would have made
exfiltration behind an honest-looking handshake merely *discouraged*.

**Confining a child's network belongs to the deployment.** The reference host
does not sandbox stdio providers. Network confinement is platform-specific
(network namespaces or seccomp on Linux, a sandbox profile on macOS,
AppContainer on Windows), and a partial implementation would state a guarantee
that holds on some platforms and silently not on others, which is the gap this
section exists to close. What the reference host provides is a seam that
composes with every such mechanism. `Host::add_stdio` spawns whatever program it
is given, with the scrubbed environment `stdio.rs` already applies, so an
operator who cannot trust a provider's declaration passes a confining wrapper as
the program: `bwrap --unshare-net`, `unshare -n`, `sandbox-exec -p`, or a
container with no network. A host that confines a stdio provider's network has
made C4's argument true for stdio again, because the transport knows.

**The reference host is stricter than C4 on loopback, deliberately.** C4 exempts
loopback. `HttpProvider` in `http.rs` does not: it forces `egress: true` for
every HTTP provider, loopback included. A loopback listener is a process of
unknown provenance that can relay every query onward, and a local proxy is the
cheapest way to walk a remote provider past C4. C4 stays the floor a conformant
host must meet. The reference host's stricter behaviour is policy, recorded in
[ADR 0024](./docs/adr/0024-consent-binds-what-the-transport-can-see.md), and is
not a divergence to relax.

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
    "as_of": "2026-07-01T00:00:00Z"      // optional; see Q2, F4
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
| **Q2** | When `as_of` is present, a provider **MUST NOT** return a frame whose valid-time window excludes it: a returned frame carrying `valid_from` satisfies `valid_from <= as_of`, and one carrying `valid_to` satisfies `as_of < valid_to`, compared as instants (§6.1). A frame carrying neither bound makes no temporal claim and is eligible. A provider with nothing valid at the pin returns zero frames. | `as-of-temporal`; `ignore-as-of`, `ignore-valid-to` witnesses |

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

### 5.2 Embedding space (E1)

| # | Requirement |
| - | ----------- |
| **E1** | A host **MUST NOT** populate `query.embedding` unless its own embedding fingerprint is **exactly equal** to the provider's declared `capabilities.embeddings_fingerprint`. A provider receiving a vector whose length contradicts its declared dimension **SHOULD** reply `bad_request`. |

Fingerprint grammar: `<model-id>/<dimensions>[/<normalization>]`, e.g.
`bge-small-en-v1.5/384/l2`. Equality is exact rather than model-id-only, because
dimension and normalization both change what a vector *means*: a 384-dim
unnormalized vector sent to an index of 384-dim L2-normalized vectors yields
plausible-looking, meaningless scores — the silent wrongness CGP exists to make
loud.

### 5.3 What `as_of` pins (Q2)

`as_of` shipped the way `kinds` did (§5.1): a request field with a format rule
(F4) and no stated semantics. The conformance suite nevertheless enforced half
of one — it failed a provider for returning a frame whose `valid_from` postdated
the pin, citing a sentence of §6.1 that imposed nothing — and checked nothing
about the other half, so a frame whose `valid_to` had passed years before the
pin was returned and passed. Q2 states the rule the suite now checks, whole.

**It is a point-in-window predicate on valid time.** A frame's `valid_from` and
`valid_to` bound when its content was *true in the world* (§6.1), and `as_of`
asks what was true at one instant, so the answer is the frames whose window
contains that instant. The window is **half-open**, `[valid_from, valid_to)`: a
fact is admitted at the instant it becomes true and excluded at the instant it
stops, so two consecutive windows that share a boundary — a fact and the fact
that superseded it — never both answer for the boundary instant. An absent bound
is unbounded on that side. Instants are compared as instants, not as strings:
under F4 an optional fractional part sorts `…:00.5Z` before `…:00Z`, and spells
one instant as both `…:00Z` and `…:00.0Z`.

**It does not pin `recorded_at`.** §6.1 separates when content was true from when
the provider *learned* it, which is a bitemporal model, and `as_of` constrains
the first axis only. A historical query wants what was true then *as best the
provider knows now*, which a fact recorded after the pin can answer correctly. A
pin on the transaction-time axis — "what did the provider believe at that
instant" — is a different question with a different field, and is reserved for
an additive `1.x` minor (§13).

**An absent `as_of` imposes no temporal constraint this revision defines.**
Whether an unpinned query answers with the provider's current view, its whole
history, or something between is provider-private, like ranking (§5).

**Every provider can honour it, so it is a MUST with no capability and no error
code.** The predicate ranges over fields the provider itself emits, and is
applied to the frames it was about to return. A provider with no temporal index
serves frames that carry no window, which the predicate admits, and that absence
is itself the signal a host reads: undated evidence makes no claim about the
pinned instant, and a host that needs dated evidence filters on the fields'
presence. So nothing needs a `capabilities` flag — which §8.3 and
[ADR 0004](./docs/adr/0004-dead-capability-surface.md) would forbid in any case
while nothing exercised it — and there is no `unsupported_as_of`: following Q1, a
provider with nothing valid at the pin returns zero frames.

The `as-of-temporal` check (`check_as_of`, named by `CHECK_AS_OF`) pins two
instants on either side of the reference fixture's validity boundary, so each
half of the window has observable work to do, and the fixture's `ignore-as-of`
and `ignore-valid-to` modes each break one half. The decision and the
alternatives are recorded in
[ADR 0022](./docs/adr/0022-as-of-is-a-point-in-window-predicate.md).

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
| **F6** | A `ProvenanceAttestation` **MUST** be detached — it **MUST NOT** appear inside the frame it signs, nor inside any hash preimage this spec defines. | `attestation` |
| **F7** | An attestation's `signed_commitment` **MUST** be the `sha256:<64 lowercase hex>` rendering of a commitment computed exactly as §6.5.2 or §6.5.3 specifies. | `attestation` |
| **F8** | A verifier that does not recognise an attestation's `algorithm` **MUST** report it as uncheckable and **MUST NOT** treat the frame as attested. "I cannot check this" is never "this is good". | `attestation` |
| **F9** | A host **MUST NOT** reject or drop a frame solely because it carries an attestation the host cannot verify; an unverifiable attestation degrades the frame to *unattested*, exactly as if it carried none. | `attestation` |
| **F10** | `score` is **provider-local and ordinal** — this spec defines no shared scale. A host **MUST NOT** apply a cross-provider `score` threshold, and **MUST NOT** present a raw `score` as a cross-provider measure of relevance. A host that *orders* frames from different providers by raw `score` **MUST** document it as its own policy choice, never as a protocol guarantee. | host composition |
| **F11** | An attestation **MUST** travel beside the frames it covers, in the result's `frame_attestations` / `result_attestation` members, and **MUST NOT** appear as a member of a `ContextFrame` (F6 on the wire). A `frame_attestations` entry **MUST** name the full *(provider id, frame id, `content_digest`)* identity it attests rather than implying it by array position, and **MUST** name a frame the same result carries. | `attestation_wire` suite; envelope schema |
| **F12** | A `result_attestation`'s `signed_commitment` **MUST** be the §6.5.3 Merkle root over the commitments of **exactly** the frames carried in `result.frames`, in canonical order — never over a larger candidate set the provider truncated away. | `attestation_wire` suite; `contextgraph_types::attest::result_set_root` |
| **F13** | An `inclusion_proof` is **OPTIONAL**, and when present **MUST** recompute the `result_attestation` root. A host that retains a strict subset of a signed result set **MUST** derive and retain the proofs for the frames it keeps *before* dropping the rest; once the siblings are gone the root can never be recomputed. | `attestation_wire` suite; host composition |
| **F14** | A provider that signs a frame **MUST** populate that frame's `content_digest`. A frame carrying none remains conformant; *signing* one is not — the commitment would bind the frame's identity and provenance and nothing about its bytes (§6.5.2). | `attestation` suite |
| **F15** | A verifier **MUST** distinguish an attestation that binds content from one that does not, and **MUST NOT** report the second as though it were the first. | `attestation` suite; `contextgraph_types::attest::AttestationVerdict::ValidIdentityOnly` |
| **F16** | A host **SHOULD** surface that distinction to whoever reads the frame. A reader deciding whether to rely on a citation is asking about the bytes in front of them. | host composition |
| **F17** | A `range` on `file` provenance **MUST** be a `line-range` (§6.2.1) whose end is not before its start, and its digest **MUST** cover exactly the bytes §6.2.1 addresses. A verifier **MUST** report any other `range`, and one whose start lies past the resource's last line, as unverifiable — **never** as the whole resource and **never** as a mismatch. | `frame-validity` (grammar); `provenance-fixture-consistency` (bytes); `tests/vectors/range-vectors.json` |
| **F18** | A verifier **MUST NOT** present an attestation member outside the signed preimage — `attester_id`, `issued_at`, `key_id`, `algorithm` — as covered by the signature, and a host **SHOULD** mark `attester_id` and `issued_at` as unverified wherever it surfaces them (§6.5.2). | `contextgraph_types::attest` test `rewriting_attester_id_or_issued_at_leaves_the_verdict_valid`; `contextgraph_host::trust::AttestationState::unverified_attester_id` |

### 6.1 Temporal profile (F4)

The profile is a **strict subset** of RFC 3339: uppercase `T`, uppercase `Z`,
UTC only. RFC 3339 also permits lowercase `t`, a space separator, and numeric
offsets; those are **not** conformant here. One spelling per instant means two
frames with the same instant compare equal as strings, which the dedup and
cache-key properties depend on. Naming it a subset rather than "RFC 3339" is
deliberate accuracy.

Semantics: `valid_from`/`valid_to` bound when the content was *true in the
world*; `recorded_at` is when the provider *learned* it. `as_of` pins retrieval
to an instant on the first axis, under the predicate Q2 states (§5.3).

### 6.2 Digests (F5)

Grammar: `sha256:<64 lowercase hex>`. Lowercase is mandated, not conventional —
digests are compared byte-for-byte, and a case disagreement is indistinguishable
from tampering.

**Digested bytes:** the exact UTF-8 source bytes addressed by `uri` + `range` at
retrieval time, with **no normalization** (no line-ending translation, no
trailing-newline adjustment). Provenance without a `range` digests the whole
resource; provenance with one digests exactly the span §6.2.1 addresses.

Only `file` provenance is held to F5: a `derivation` or `episode` link has no
addressable bytes, so requiring a digest of it would be theatre.

#### 6.2.1 Line ranges (F17)

A digest is compared byte for byte, so two implementations that disagree about
which bytes `L120-160` names compute different digests over an identical file —
and a verifier reports that as a **mismatch**, the signal §6.5.4 treats as
tampering. The failure mode of an unstated range grammar is a false accusation,
not a parse error. The grammar is therefore normative:

```abnf
line-range  = %x4C line-number [ "-" line-number ]  ; "L", uppercase only
line-number = %x31-39 *DIGIT                         ; decimal, >= 1, no leading zero
```

`%x4C` rather than `"L"` because ABNF string literals are case-insensitive, and
`l120` is not a line range. A range addresses bytes as follows:

1. **Lines split on LF (`0x0A`) only.** A line is the bytes from the start of the
   resource, or from just after an LF, through **and including** the next LF —
   or through the end of the resource when no LF follows. A final LF does not
   begin an additional empty line; an empty resource has zero lines. Splitting is
   over bytes, with no decoding (UTF-8 never encodes `0x0A` inside a multi-byte
   sequence, so this agrees with splitting on U+000A for valid input).
2. **CR (`0x0D`) is content.** It is never stripped and never a terminator: a
   CRLF line carries both bytes, and a resource that uses bare CR is one line.
   This is §6.2's no-normalization clause, applied to ranges.
3. **1-indexed, end inclusive.** `L2-3` is lines two and three. `L5` addresses
   exactly what `L5-5` does.
4. **The addressed bytes** run from the first byte of line *start* through the
   last byte — its LF included — of line *end*, contiguous and exactly as stored.
5. **An end past the last line clamps** to the last line. **A start past the last
   line addresses nothing**, and a verifier reports it unverifiable: an empty span
   is not something a range can mean.
6. **An end before the start** addresses nothing. A producer **MUST NOT** emit
   one; a verifier reports it unverifiable.
7. **Line numbers are unbounded.** A verifier whose integers cannot hold a line
   number treats it as larger than any line count: such an end clamps, such a
   start lies past the last line. The meaning of a range never depends on the
   verifier's integer width.

**Every other spelling is reserved.** `contextgraph/1` defines no byte,
character, or column range, and GitHub's `L10-L20` is not this grammar. A later
revision may define further forms; the reservation is what makes that safe,
because a verifier built before it reports such a range unverifiable, which
degrades the digest check rather than faking it — the stance F8 takes toward an
unknown signature algorithm. A verifier **MUST NOT** fall back to digesting the
whole resource, which would confirm bytes the range never named, and **MUST NOT**
report a mismatch, which would call a grammar disagreement tampering.

**Why the end clamps.** The digest, not the range, is the integrity check: a
clamped span still has to hash to the declared digest, so clamping can never make
altered bytes verify. What it decides is only how a file that shrank beneath a
range is reported — as a mismatch, which is true (the bytes changed), rather than
as unverifiable. The rest of the ecosystem already depends on it: the reference
fixture and all four SDK examples declare `L1-40` over a four-line file. A
producer **SHOULD** nonetheless emit an end within the resource, and **SHOULD**
write a single line as `L<n>` rather than `L<n>-<n>`: the two address the same
bytes, but `range` is compared as a string by hosts deduplicating citations and is
signed as a string inside the attestation preimage (§6.5.1).

That preimage never parses `range`. §6.5.1 encodes it as opaque bytes, so an
attestation over a link whose `range` is outside this grammar is still well
defined — the `range` values in `tests/vectors/attestation-vectors.json` are
encoding inputs, not digest claims.

The reference implementation is `contextgraph_types::LineRange` (addressing) and
`contextgraph_host::verify`'s `extract_line_range` (the re-read); an unrecognised
range surfaces there as `DigestVerification::Unreadable` carrying an
`unsupported_range` reason. `tests/vectors/range-vectors.json` publishes the
addressed bytes and digest for every rule above, including each unverifiable
case, so a second implementation can check itself.

### 6.3 Frame identity (D1–D5)

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
| **D5** | On the wire — a `verify` request, a `verified` verdict, a `frame_attestations` entry, and the §6.5.2 commitment — *provider id* is the provider's handshake-declared `provider.name` (§3). Inside a host — composition, dedup (D2), reuse (D3), usage reports (UR1), attribution (A1) — it is the host's local id for the provider. A host **MUST** translate at the connection boundary: it substitutes the declared name of the provider it is sending to, and resolves an echoed identity to the local id of the connection it arrived on, **never** by looking a declared name up across providers. | `Host::verify_frames`; `verify-honesty` (the suite registers the provider under a local id that differs from its declared name) |

**Which provider id (D5).** A host knows every provider by two names. The
*local id* is the key the operator configured it under (`Host::add_stdio(id, …)`),
under which consent (§4) and attestation trust are recorded; the provider never
sees it. The *declared name* is `provider.name` from the handshake, the only
provider identifier both ends of the wire observe. They need not agree, and the
reference conformance suite deliberately registers every provider under a local
id that differs from it.

Each is right for exactly one half of the triple's job, which is why D5 assigns
both rather than choosing one:

- **The wire needs the declared name**, because a provider can only answer about,
  or sign over, an identifier it knows. §6.5.2 already puts it in the signed
  preimage, and that preimage is frozen for the `contextgraph/1` family.
- **The host needs the local id**, because a declared name is a claim, not a
  credential: H2 requires only that it be non-empty. A provider declaring another
  provider's name would otherwise mint identities that collide with that
  provider's — and D2 says two frames sharing an identity are *the same content*,
  so the collision would poison dedup, reuse, the usage-report join, and
  attribution at once. The local id is chosen by the operator in the same act as
  the consent grant, so it names the provider the operator actually configured.

Because translation is per connection, two configured providers that declare the
same name are not ambiguous: each is asked only about the frames held under its
own local id, and each verdict is resolved through the connection it arrived on.
A host **MAY** warn an operator about the duplicate; it need not refuse it.

An identity keyed on a local id is **host-scoped**: meaningful together with that
host's configuration and not beyond it, and canonical order (§6.3,
[`docs/context-reuse.md` §1](./docs/context-reuse.md)) is byte-stable across hosts
only where they configure the same local ids. A host that shares identities
beyond itself — a fleet-wide cache, a usage warehouse spanning hosts — carries its
local-id → declared-name binding with them. Matching identities across hosts on
the declared name alone inherits H2's weakness, so a deployment that needs it
pins each declared name to the attester key it trusts for that provider (§6.5,
[ADR 0016](./docs/adr/0016-attestation-trust-roots.md)). The decision and the
alternatives are recorded in
[ADR 0023](./docs/adr/0023-frame-identity-names-two-provider-ids.md).

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
sketch existed for this during design work but is not kept in-tree. The **Context Exchange
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

### 6.5 Provenance attestation (F6–F9)

F5 makes a frame's provenance **tamper-evident**. It does not make it
**evidence**. A digest proves the bytes have not changed since someone wrote
that number down; it says nothing about who wrote it. The digest and the frame
it describes come from the same unauthenticated party, so a provider willing to
fabricate a frame is equally willing to fabricate its digest, and every check in
§6.2 passes. Detecting *accidental* drift and proving *deliberate* honesty are
different problems, and only a signature solves the second.

A **provenance attestation** is a detached signature over a commitment to a
frame's identity and its provenance chain. It is **optional**: a conformant
provider may serve no attestations at all, and a conformant host may verify
none. What is not optional is the construction — an attestation that exists must
be computed exactly this way, or two implementations will disagree about whether
the same evidence is genuine.

#### 6.5.1 Canonical encoding

Each provenance link encodes as its typed fields in **declaration order**, each
length-prefixed:

```text
enc_str(s)   = uint32be(byte_length(utf8(s))) ‖ utf8(s)
enc_opt(None)    = 0x00
enc_opt(Some(s)) = 0x01 ‖ enc_str(s)

encode(link) = enc_str(link.type)
             ‖ enc_opt(link.uri)    ‖ enc_opt(link.range)
             ‖ enc_opt(link.digest) ‖ enc_opt(link.method)
             ‖ enc_opt(link.by)
```

Field order is **normative**. So is the length prefix: bare concatenation is
ambiguous, and without prefixes a link with `uri: "ab", range: "c"` encodes
identically to one with `uri: "a", range: "bc"` — a collision an adversary picks
rather than searches for. The presence byte is equally load-bearing: without it
`uri: null` and `uri: ""` collide, and a link's URI could be deleted from a
signed chain without disturbing the hash.

This encoding is deliberately **not** RFC 8785 (JCS), which the Context Exchange
Provider profile uses for `record_hash`. JCS is right for a record, whose hash
covers an open-ended JSON document. A provenance link is six optional strings,
and for that shape JCS only adds a dependency on a conforming JSON canonicalizer
— whose number formatting and Unicode escaping rules are precisely where
cross-language implementations silently diverge. Any language can produce the
encoding above from the typed fields with no library at all.

#### 6.5.2 Chain head and frame commitment

The links of `provenance` fold **source-first** — the order §6 already requires
them to be carried in — into a hash chain:

```text
h₋₁  = SHA256("contextgraph/attest/1/genesis")
hᵢ   = SHA256("contextgraph/attest/1/link" ‖ hᵢ₋₁ ‖ encode(linkᵢ))
chain_head = hₙ₋₁ , or h₋₁ when provenance is empty
```

Because each step consumes the previous head, no link can be inserted, removed,
reordered, or edited without changing the result — the property a set of
independent per-link digests never had. An empty chain hashes to the genesis
value rather than to zero, so "this frame claims no provenance" is a signed
assertion rather than a gap.

The signed preimage for a single frame binds that head to the frame's full
identity:

```text
frame_commitment = SHA256(
    "contextgraph/attest/1/frame"
  ‖ enc_str(provider_id) ‖ enc_str(frame.id)
  ‖ enc_opt(frame.content_digest)
  ‖ chain_head )
```

**The identity binding is not optional.** Two frames citing the same source share
a chain head, so a signature over the head alone can be lifted from one frame and
stapled to another: it verifies, and the evidence is invented. Including the
*(provider id, frame id, `content_digest`)* triple of §6.3 means a signature binds
to one frame from one provider carrying one set of bytes, or it binds to nothing.

**What a signature binds when `content_digest` is absent.** `content_digest` is
encoded as an *option* because a frame is permitted to carry none (D3), and
`enc_opt` records that absence honestly rather than substituting a placeholder.
The consequence has to be stated plainly, because it is not what the presence of
a signature suggests: an attestation over a frame that declares no
`content_digest` binds **the provider's identity, the frame id, and the
provenance chain, and nothing whatsoever about the frame's content**. The same
provider may serve one set of bytes under that frame id today and entirely
different bytes tomorrow, and the original signature still verifies, because the
content was never in the preimage.

Three rules follow:

* **F14.** A provider that signs a frame **MUST** populate that frame's
  `content_digest`. A frame carrying no digest remains conformant; *signing* one
  is not. This is a requirement on the attester, not on the wire — a frame with
  no digest and no attestation is unaffected.
* **F15.** A verifier **MUST** distinguish an attestation that binds content
  from one that does not, and **MUST NOT** report the second as though it were
  the first. Reporting them alike is what lets a host render "signed" over bytes
  the signature never covered.
* **F16.** A host **SHOULD** surface the distinction to whoever reads the frame.
  A reader deciding whether to rely on a citation is asking about the bytes in
  front of them, and "the provider signed something with this id" is a different
  answer to that question.

The reference implementation returns a distinct `ValidIdentityOnly` verdict for
this case rather than `Valid`, and its host records it as attested with
`covers_content: false`. An implementation is free to spell the distinction
differently; it is not free to omit it.

**What a signature does not bind: the attestation's own metadata.** The
commitment above — or, for a result attestation, the §6.5.3 root — is the whole
of what a signature covers. An attestation carries six members, and this is how
each relates to the signature:

| Member | Covered by the signature? |
| ------ | ------------------------- |
| `signed_commitment` | **Yes** — it is the signed message, recomputed from the frame in hand (§6.5.4). |
| `signature` | It *is* the signature. |
| `key_id` | **No.** It selects the verifying key. Rewriting it selects a different key, which the signature fails against, or none — a safe failure either way. |
| `algorithm` | **No.** Rewriting it yields a signature that fails or an algorithm the verifier declines (F8) — a safe failure. |
| `attester_id` | **No.** Nothing reads it during verification. |
| `issued_at` | **No.** Nothing reads it during verification. |

The last two are what the presence of a signature suggests is covered, so the
consequence has to be stated plainly. Anyone who handles an attestation in
transit — a relaying host, a cache, a registry, a compromised distribution step —
can rewrite **`attester_id`** to name any authority and **`issued_at`** to any
instant, and the signature still verifies as `Valid`. `attester_id` therefore
carries no accountability the signature vouches for: who stands behind a verified
attestation is answered by the key that verified it, resolved from the verifier's
own trust store (§6.5.5), never by the name the attestation prints. `issued_at`
is the attestation's only temporal claim and it is unauthenticated: a
well-formed timestamp (F4) is the one part a forger has no reason to get wrong,
and nothing in `contextgraph/1` supports reasoning about an attestation's age,
freshness, or expiry.

* **F18.** A verifier **MUST NOT** present an attestation member outside the
  signed preimage — `attester_id`, `issued_at`, `key_id`, `algorithm` — as
  covered by the signature, and a host **SHOULD** mark `attester_id` and
  `issued_at` as unverified wherever it surfaces them. Reporting "valid" beside
  an unsigned name and date is how a forged attribution comes to be read as a
  signed one.

The reference implementation's `sign_frame_attestation` and
`verify_frame_attestation` follow this boundary, and its host's
`AttestationState::Attested` vouches only for the `key_id` that verified,
exposing the `attester_id` it echoes as `unverified_attester_id` and carrying no
`issued_at` at all. Binding either member into the preimage would change the
commitment of every attestation ever produced, which is a new major family
(§13 U4); that question is recorded, and deliberately left to one, in
[ADR 0021](./docs/adr/0021-attestation-metadata-outside-the-signature.md).

`provider_id` is the provider's handshake-declared `provider.name` (§3). A host
also keeps a local id for each provider it has configured, and that one is not a
string the provider ever sees — so it is not one a provider could sign against.
The declared name is the only identifier both ends of the wire observe.

#### 6.5.3 Result-set Merkle root

A provider signing a whole answer commits to a Merkle root over its frames'
commitments, taken in the canonical order of §6.3, using RFC 6962 hashing:

```text
leaf(c)       = SHA256(0x00 ‖ c)
node(l, r)    = SHA256(0x01 ‖ l ‖ r)
MTH({})       = SHA256("contextgraph/attest/1/merkle-empty")
MTH({c})      = leaf(c)
MTH(C)        = node( MTH(C[0..k]), MTH(C[k..n]) ),  k = largest power of 2 < n
```

The distinct leaf and interior prefixes are what stop an interior node's hash
from being presented as a leaf — without them a subtree could masquerade as a
single frame. The RFC 6962 split is chosen over the common "duplicate the last
leaf on an odd level" shortcut because that shortcut admits two distinct leaf
sets with the same root; acceptable for a checksum, disqualifying for evidence.

An **inclusion proof** carries the leaf index, the leaf count, and the sibling
hash at each level with the side it sits on. The leaf count is part of the proof
because a root alone does not pin the tree's size, and a verifier that ignores it
can be shown a proof from a differently-shaped tree. This is what makes a signed
answer selectively disclosable: a host proves one frame was in the set without
revealing the others.

#### 6.5.4 Verification

Verification is **offline and pure**: a commitment, an attestation, and a public
key are sufficient. A verifier recomputes the commitment from the frame in hand,
compares it to `signed_commitment` **before** examining the signature, and only
then checks the signature over the commitment bytes.

The comparison order is deliberate. A mismatch means the frame changed after
signing; a signature failure means the key is wrong or the signature forged.
Reporting the first as the second sends an operator hunting a key-management bug
when the actual finding is tampering.

Verifiers **MUST** distinguish these outcomes rather than collapsing them into a
boolean — F8's "uncheckable" and "invalid" are different findings with opposite
responses — and, per F9, an unverifiable attestation degrades a frame to
*unattested* rather than disqualifying it. A host that dropped such frames would
hand any peer a denial-of-service primitive: attach a malformed attestation and
watch the evidence disappear.

A verdict is a statement about the commitment and the key, and about nothing
else in the attestation. A `Valid` verdict says the frame in hand matches what the
holder of the verifying key signed; it says nothing about the attestation's
`attester_id` or `issued_at`, which the signature does not cover (§6.5.2, F18).

Implementations **SHOULD** use a strict Ed25519 verifier — one rejecting
small-order public keys and non-canonical signature encodings. A signature two
conforming verifiers can disagree about is not evidence.

#### 6.5.5 Carrying an attestation on the wire (F11–F13)

An attestation travels **beside** the thing it signs, never inside it (F6), and
it reaches a verifier over two hops.

**The key rides the handshake.** `handshake_ack.attester_keys` publishes the
public keys the provider signs with. A provider that publishes none offers no
attestation, which is conformant: §6.5 makes the *construction* mandatory and
the signing optional.

```jsonc
{
  "type": "handshake_ack",
  "protocol_version": "contextgraph/1.0",
  "provider": { "name": "example-docs", "version": "1.0.0",
                "data_flow": { "reads": true, "writes": false, "egress": false } },
  "capabilities": { "query": { "kinds": ["doc"] } },
  "attester_keys": [
    { "key_id": "example-docs-ed25519-1", "algorithm": "ed25519",
      "public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a" }
  ]
}
```

A published key settles whether an attestation is **built** the way this section
requires. It settles nothing about *who* signed: it comes from the party under
audit. A deployment that needs the second answer resolves `key_id` against its
own trust store and ignores what the handshake said. It rides the handshake
rather than the answer because a key republished with every response could be
swapped by the same forgery that swapped the signature.

**The evidence rides the result.** A `frames` envelope's `result` carries two
optional members, and the `frames` envelope itself carries **no** attestation
member of its own. Both are omitted when empty, so an unsigned answer is
byte-identical to one from a provider written before attestation existed, and a
1.0 peer that ignores them still reads a signed answer as a valid answer
(§13 U1).

```jsonc
{
  "type": "frames",
  "id": "q3",
  "result": {
    "frames": [
      // the answer's frames, elided — see examples/reference-messages.json
    ],
    "truncated": false,

    // One entry per attested frame, naming the identity it covers in full.
    "frame_attestations": [
      {
        "frame": { "provider_id": "repo-graph", "frame_id": "repo-graph:retry-doc",
                   "content_digest": "sha256:<64 hex>" },
        "attestation": {
          "signed_commitment": "sha256:<64 hex>",   // the §6.5.2 frame commitment
          "key_id": "repo-graph-2026-08",
          "algorithm": "ed25519",
          "attester_id": "repo-graph",
          "signature": "<128 hex>",
          "issued_at": "2026-08-29T12:00:00Z"
        },
        "inclusion_proof": {
          "leaf_index": 0,
          "leaf_count": 2,
          "path": [{ "sibling": "sha256:<64 hex>", "sibling_is_left": false }]
        }
      }
    ],

    // One signature over the whole answer: the §6.5.3 Merkle root.
    "result_attestation": {
      "signed_commitment": "sha256:<64 hex>",
      "key_id": "repo-graph-2026-08",
      "algorithm": "ed25519",
      "attester_id": "repo-graph",
      "signature": "<128 hex>",
      "issued_at": "2026-08-29T12:00:00Z"
    }
  }
}
```

They sit on the **result** rather than on the envelope for the same reason
`truncated` does: an attestation is a property of the answer, not of the
transport. The envelope carries only `type` and the correlation `id`, and an
in-process provider that returns a result with no envelope at all must still be
able to sign what it serves.

**There is exactly one home, and that is the point.** An earlier revision of
this section also allowed an `attestations` member on the `frames` envelope, so
one signed answer had two encodings and nothing said which won when they
disagreed. Two encodings of one fact with no tie-breaking rule is the ambiguity
this specification exists to remove, and it is gone: a receiver that encounters
an envelope-level `attestations` member ignores it.

**The identity is echoed in full, never implied by position.** A parallel array
indexed against `frames` would be smaller and unusable: a provider that
reorders, omits, or duplicates a frame would shift one frame's evidence onto
another, which is the substitution §6.5.2's identity binding exists to prevent.
It is also what a verifier needs — `provider_id` and `content_digest` are two of
the three inputs to the frame commitment and are recoverable from nowhere else.
This is the discipline §9's `FrameVerdict` already applies to `verify`.

**Both members of an entry are optional, and an entry with neither asserts
nothing.** The cheapest honest way to sign an answer is one signature over the
root plus a per-frame inclusion proof, with no per-frame signature at all;
requiring `attestation` would make that shape unrepresentable and force a
provider into *n* signatures to say what one says. A provider that signs frames
individually and publishes no root sends no proof. A host reading an entry that
carries neither treats the frame as *unattested* (F9).

**Inclusion proofs are carried, not derived, and F13 says why.** A host holding
the *complete* result set can rebuild every proof itself: it has all the
commitments. But a host that keeps a *subset* — after budget truncation,
cross-provider dedup, or ordinary composition — cannot, because the dropped
siblings' commitments are gone and no amount of later work recovers them. The
retained frames are then attested by a root nothing can recompute. So the proofs
have to be derivable at the one moment the whole set is in hand, and a provider
that ships them makes a host's correctness the host's own affair rather than a
step it has to know to take. The reasoning, and the wire-size argument against
mandating them, are in
[ADR 0014](./docs/adr/0014-attestations-on-the-wire.md).

**A truncated answer signs what it returned.** F12's root covers exactly the
frames in `result.frames`, never the candidate set the provider considered. A
root over frames the host never received is unverifiable by construction, and an
unverifiable root is worse than none: it looks like evidence.

### 6.6 What `score` means, and does not (F10)

F1 constrains `score` to `[0, 1]`. That is a **range**, not a **scale**, and the
difference matters the moment a host composes frames from more than one
provider. Nothing in this specification defines what `0.8` means, and nothing
could: one provider's score is a cosine similarity, another's a BM25 rank
normalized by its own corpus, a third's a hand-tuned blend. Two providers
returning `0.8` are not making the same claim, and the same provider need not
mean the same thing across two queries.

So `score` is **provider-local and ordinal**: it orders *that provider's* frames
against *that query*. It is not a measurement, and it is not comparable across
sources.

**Why this is stated rather than fixed.** The obvious alternative is to mandate
calibration — require providers to map scores onto a shared scale. It is
unenforceable, and unenforceable requirements are worse than absent ones. There
is no reference corpus a conformance suite could score against without
prescribing what relevance *is*, which would put this specification in the
business of defining retrieval quality. A provider could satisfy any calibration
rule we wrote while its numbers stayed meaningless, and the suite would certify
it. §7 could make budget honesty checkable because token cost is a function of
bytes both sides observe; relevance has no such anchor. Claiming comparability
we cannot verify is exactly the self-attestation §11.1 exists to rule out.

**What a host does instead.** Any cross-provider ordering is the *host's*
policy, and it owns the consequences: per-provider quotas, round-robin
interleaving, a reranker it controls, an explicit trust weighting it can defend
— or, most simply, ranking by raw `score` and saying so. All are permitted. What
F10 forbids is passing that choice off as something the protocol guaranteed.

A host **MAY** apply a threshold to a **single** provider's scores, where the
ordering is meaningful. Applying one uniformly across providers silently prefers
whichever provider scores most generously — a ranking decided by an
implementation detail of someone else's retriever, which is precisely the
unaccountable behavior this protocol exists to eliminate.

**The reference host, stated plainly.** `dedup_cross_provider` compares scores
across providers, but only to pick a survivor among frames already proven to be
*the same evidence* by content digest or overlapping file provenance — it breaks
a tie between duplicates and never decides what is relevant.

Everything else routes through one seam, `RankingStrategy`
([ADR 0015](./docs/adr/0015-cross-provider-ranking-strategies.md)), and the
strategy's order is what the budget packer walks — so the policy decides which
frames reach the prompt, not only where they sit in it. Three ship:

- `ScoreDescending` ranks the union by raw `score`. It is the default that
  `order_by_value` and `compose_for_prompt` apply, a deliberate documented
  choice for a host that has no better policy, and not a claim that the scores
  are commensurable. It is also simply correct for a single-provider host,
  where the cross-provider question does not arise.
- `RoundRobinByRank` interleaves providers by **within-provider** rank — every
  provider's best frame, then every provider's second — so the only score
  comparisons it makes are the ones this section says are meaningful.
- `PerProviderQuota` does the same in blocks of `k`, so a provider's evidence
  stays contiguous.

A host with its own reranker or trust weighting implements the trait and passes
it to `compose_for_prompt_with`, or ranks the frames itself and calls
`fold_to_edges` for placement alone.

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

### 8.1 Relation vocabulary (SHOULD)

The `rel` vocabulary is **open** — a host **MUST NOT** reject an unknown value.
These names are published so independent providers converge instead of each
inventing `calls` / `call` / `code.call`:

`code.calls` · `code.imports` · `code.defines` · `code.references` ·
`doc.documents` · `episode.follows`

Provider-specific edges belong under their own namespace (`myindex.owns`), which
keeps the shared namespace meaningful.

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
design sketch existed for this during design work but is not kept in-tree.

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
`valid` onto the wrong frame. The `provider_id` in those identities is the
recipient's handshake-declared `provider.name`, never the host's local id for it
(D5, V5).

| # | Requirement | Verified by |
| - | ----------- | ----------- |
| **V1** | A `verify` request **MUST** carry frame identities only — no frame bodies. A host **SHOULD** include only identities carrying a `content_digest`; a digest-less frame cannot be revalidated and is re-queried instead. | `verify-honesty` |
| **V2** | A provider declaring `capabilities.verify` **MUST** answer a `verify` with a `verified` reply. A requested identity that comes back with no verdict **MUST** be treated by the host as `unknown`. | `verify-honesty`; `rubber-stamp-verify`, `hollow-verify` witnesses |
| **V3** | A verdict is one of `valid`, `stale`, `gone`, `unknown`. A host **MUST** reuse a held frame body **only** on `valid`; `unknown` **MUST NOT** be read as validity. Reuse requires a positive answer, never the absence of a negative one. | `verify-honesty` |
| **V4** | A `stale` verdict **MAY** carry a `replacement_digest` — the provider's current digest for the frame, a digest never a body. A host **MUST NOT** keep serving its stored copy of a `stale` or `gone` frame. | `verify-honesty` |
| **V5** | Every identity in a `verify` request **MUST** carry the recipient provider's handshake-declared `provider.name` as its `provider_id` (D5). A provider **MAY** answer `unknown` for an identity naming any other provider id — it did not serve that frame, whatever its frame id — and **MUST NOT** reject the whole request on that basis. | `Host::verify_frames`; `verify-honesty` |

V5 answers a foreign `provider_id` per entry rather than per request because V2
and V3 already make `unknown` safe — a host never reads it as validity — while a
whole-request error would let one misaddressed entry deny revalidation for every
other frame in the batch. The reference fixture exercises the permission: it
answers `unknown` to any identity not naming its own declared name, which is what
makes a host that leaked its local id onto the wire fail `verify-honesty`.

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

That harness drives *this* repository's host. A **composition harness**
(`contextgraph-conformance`'s `composition_conformance` module) covers the step
above it, in whatever host implements it: given a `ComposingHost` — anything that
answers "with these providers and this query, what reaches the prompt, and what
did you drop getting there?" — it checks the rules binding a host's merge across
providers. `Host::query_all` audits budget honesty **per provider**, so a set of
individually conformant providers can still overflow a shared budget in
aggregate: three providers each returning one honest 400-token frame against a
1000-token query are each within budget and jointly 200 over. The checks are the
cross-provider **token bound** (§7); the **total partition** — every offered frame
is admitted or reported dropped, never silently truncated (issue #15); the
**quarantine** (§7 B2/B4) — a provider the audit rejected contributes nothing,
checked with a *frame flooder* whose frames are individually cheap, so only having
consulted the audit keeps them out; and **determinism** — an unchanged frame set
composes to the same render order, the prompt-cache guarantee of
`docs/context-reuse.md` §1. `ReferenceComposingHost` (`query_all` plus
`compose_for_prompt`) is the worked example that passes it. A host with its own
merge implements the trait and gets the same audit instead of an assurance.

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
- **C3 over stdio — a stdio provider's `egress: false` is unverifiable.** This
  is a larger hole than the HTTP rules above. A child process can open a
  socket and send the query payload anywhere while declaring `egress: false`,
  and the host queries it without asking for consent, because C3 is the only
  thing that says otherwise and the pipe carries no evidence either way (§4.3). No provider
  check can observe it from the wire, and no host scenario can observe it from
  the transport. The `contextgraph-host` test
  `a_stdio_provider_declaring_no_egress_exfiltrates_undetected`
  (`tests/stdio_egress_gap.rs`) witnesses that the gap exists today: a child
  declaring `egress: false` connects out carrying the query's goal, and the
  query succeeds with no consent recorded and no error raised. That test is
  the fail→pass case for any later fix, whether confinement in the reference
  host or a detection hook. Until then, closing the gap is the deployment's
  confinement choice (§4.3).
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
| **U2** | The `FrameKind` set (`snippet`, `symbol`, `fact`, `doc`, `memory`, `episode`, `graph`) is the **base vocabulary of a major family**; a new kind is a `1.x` addition. A host that receives an unrecognised `kind` **MUST** treat the frame as opaque evidence — it **MAY** decline to specialise its handling, but **MUST NOT** fail to deserialise, reject, or crash — and if it re-emits the frame it **MUST** preserve the original `kind` string verbatim. New *open* vocabularies (`rel`, error `code`, `egress_scope`) grow without a version bump; a receiver **MUST NOT** reject an unknown value in any of them (§8.1, §10 X1, §4.1). |
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

### 13.1 Where the schemas are published

The schemas are versioned on the same axis as everything else in this section —
the **major family** — and their `$id` says so:

| Schema | `$id` |
| --- | --- |
| envelope | `https://contextgraphprotocol.org/schema/v1/contextgraph-envelope.schema.json` |
| lifecycle record | `https://contextgraphprotocol.org/schema/v1/contextgraph-lifecycle-record.schema.json` |

`v1` is `contextgraph/1`, not the crate version. Because U1–U4 make `1.x`
evolution additive-only, a consumer holding a copy fetched earlier in the
family's life is never *wrong* about what it does know — which is what lets one
URL serve the whole family. A `contextgraph/2` schema would be published at
`/schema/v2/`, and `/schema/v1/` would keep answering.

Every `$ref` in both schemas is a same-document pointer (`#/$defs/…`); neither
references the other, so both validate fully offline from a local copy. The
former `$id`, on `raw.githubusercontent.com`, still resolves to the same bytes.
See [ADR 0013](docs/adr/0013-schema-identity-on-a-branded-versioned-url.md).

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
transport is deferred to a 1.x additive minor. Shipping a negotiated feedback method
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
