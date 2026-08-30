# Profile: Context Exchange Provider (`contextgraph/lifecycle/1.0-draft`)

> **Status: normative profile (draft).** This document ratifies the Context
> Exchange Provider profile for issue
> [#28](https://github.com/macanderson/context-graph-protocol/issues/28). It is
> the single source of truth for the lifecycle/records exchange layer: the wire
> shapes here, the JSON Schema
> [`schema/contextgraph-lifecycle-record.schema.json`](../../schema/contextgraph-lifecycle-record.schema.json),
> the Rust types
> [`contextgraph-types::record`](../../contextgraph-types/src/record.rs), and the
> example vectors under [`tests/fixtures/`](../../tests/fixtures) are one
> description of one layer. It supersedes the earlier "draft skeleton" and the
> downstream build prompt as the authority on record wire shapes.
>
> Requirement keys use RFC 2119 language and stable anchors (`LH1`, `LR1`, …)
> matching [`SPEC.md`](../../SPEC.md)'s style, so a conformance check or another
> document can cite them.
>
> **Scope guard.** This is a **profile on top of the base spec** (the same
> pattern as the host profile, issue #14), layered on the `contextgraph/1`
> family — **not** new frozen-`1.0` core surface. It carries record *values* and
> *decisions*; it never grants a host authority to act on them (ADR 0007 §3).
> Anchored on [ADR 0007 — the protocol/product boundary](../adr/0007-protocol-product-boundary.md)
> and the (now-resolved and removed) adaptive-context reconciliation delta
> table, which this profile resolved.

## 1. Why a profile, not core

`contextgraph/1.0` is a **read** protocol: a host queries providers for
budgeted, provenance-carrying frames and optionally revalidates them
(`context/verify`). It deliberately excludes the write path (`context/upsert`,
issue #5), push invalidation (`subscribe`, issue #6), and content resolution
(`context/resolve`, issue #50 → this profile) — each was removed or deferred
pre-freeze (ADR 0004; SPEC.md §6.4.1) precisely because core 1.0 had no consumer
that forced their design, and freezing an unexercised operation is the
dead-capability anti-pattern.

A **Context Exchange Provider (CEP)** is the consumer that forces those designs.
Beyond answering `context/query`, it offers a **durable, multi-tenant, auditable
exchange** of immutable context *records*: append with idempotency, retrieval by
identity, content resolution, retention commitments, and signed attestations.
That is a larger contract than a read-only provider, and it earns its own
**profile** layered on the `contextgraph/1` family rather than bloating the core
every provider must implement. It is also the concrete path to GOVERNANCE freeze
**criterion 1** (two independent implementations): the reference host + crates on
one side, a genuine third-party CEP on the other.

## 2. Profile identifier and discovery (resolves the identifier + handshake [OPEN])

| # | Requirement |
|---|---|
| **LP-ID1** | A CEP **MUST** be a conformant `contextgraph/1.0` provider first: green on `contextgraph-conformance` for its declared read capabilities (SPEC.md §12). The exchange operations are **additive**, gated behind capability advertisement. |
| **LP-ID2** | The profile identifier is **`contextgraph/lifecycle/1.0-draft`** (ADR 0007 §5; reconciliation row D4). Every record's `schema_version` **MUST** equal this string. The `cgep/*` namespace and the "CGEP" rename are rejected (ADR 0007 §5). |
| **LP-ID3** | A CEP advertises profile support **in the handshake capability document**, under a namespaced `lifecycle` capability block (a member of the provider's advertised capabilities, not a new envelope). A host discovers CEP support by reading that block; its absence means the provider is read-only and the exchange operations **MUST NOT** be sent to it. |
| **LP-ID4** | The core major-family rule (SPEC.md §3.1) and the extensibility rules (SPEC.md §13, U1–U4) apply unchanged. The profile version tracks the core `-draft` freeze but is versioned independently (`lifecycle/1.0`), so the profile may reach `1.x` on its own additive cadence. |

### 2.1 Capability negotiation (resolves D4)

The `lifecycle` capability block advertises, at minimum: the **representations**
served (`full`/`compact`/`reference`); **`known_at`** point-query support;
**resolve** support; the **record kinds** served (a subset of the 12 in §4); the
**operations** offered (§6); **payload and batch limits** for append/get;
**retention classes** honored; the **consent class**; and the provider's
**unknown-field behavior** (which **MUST** be U1 ignore-on-read for the interop
path, per SPEC.md §13). A host **MUST NOT** send an operation, representation, or
record kind the provider did not advertise; a provider asked for an unadvertised
one replies `error` with the matching code from §8.

## 3. Canonical hashing — RFC 8785 JCS (resolves the JCS [OPEN])

Records are **content-addressed**. `record_hash` is the anchor of a record's
identity, of idempotency replay, and of attestation.

| # | Requirement |
|---|---|
| **LH1** | `record_hash` **MUST** be `sha256:<64 lowercase hex>` (SPEC.md §6.2 grammar) over the **RFC 8785 (JCS)** canonicalization of the record **with its own top-level `record_hash` member removed from the preimage**. A record never hashes over its own hash. **Removed, not blanked**: a record therefore hashes identically whether it carries no `record_hash`, the right one, or a wrong one, so a producer computes the hash without first inventing a placeholder and a verifier never has to know which placeholder was chosen. A `record_hash` nested inside `extensions` or a body member is ordinary content and **MUST** stay in the preimage. |
| **LH2** | Canonicalization is **RFC 8785** exactly: object members sorted **by UTF-16 code unit** (RFC 8785 §3.2.3 — *not* by code point; the two orders differ for a supplementary character, whose lead surrogate sorts below U+E000), minimal separators, no insignificant whitespace, and the RFC 8785 **number policy** (the ECMAScript `Number.prototype.toString` shortest round-trip form — e.g. `0.9`, not `0.90`; integers with no decimal point; `-0` serialized as `0`). Two implementations that agree on the bytes agree on the hash. |
| **LH3** | The **detached** attestation (`RecordAttestation`, §7) is **never** part of the record or its `record_hash` preimage. Re-signing or key rotation therefore never perturbs a record's content-addressed identity. |
| **LH4** | The reference implementation **MAY** additionally compute a `command_hash` over `(record_hash + requested_retention + behavior-changing options)` for idempotency keying (§5); that hash is a provider-ledger concern, not part of the record wire shape. |
| **LH5** | The reference implementation is [`contextgraph_types::record_attest`](../../contextgraph-types/src/record_attest.rs), behind the off-by-default `record-hash` feature ([ADR 0012](../adr/0012-record-hash-and-record-attestation.md)). `record_hash_preimage` returns the exact canonical bytes, because a digest alone cannot tell an implementer *where* their canonicalization diverged. Its RFC 8785 conformance is checked against the RFC's own published vectors — §3.2.4's byte listing, §3.2.3's sorting data, and Appendix B's IEEE 754 number table. |

The canonical JCS/`record_hash` **golden vectors** are the interop spine and live
in this repo (§9): `tests/fixtures/record-hash-vectors.json` publishes the
canonical **text** of every fixture's preimage beside its hash. A fully worked
example is in [`tests/fixtures/README.md`](../../tests/fixtures/README.md).

Reproducing those bytes without a JCS library is possible for *these* fixtures,
and is not the same thing as implementing RFC 8785. A Python
`json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` matches
here because every fixture's member names are ASCII and every number round-trips
identically under CPython's `repr` and ECMAScript's `Number::toString`. The
profile guarantees neither property, and a record that broke either would still
be valid — so a provider computes `record_hash` with a conforming canonicalizer
and uses the shortcut only to sanity-check a vector.

## 4. The `ContextRecord` (resolves D1)

Every record is one immutable JSON object: a **common envelope** plus a **flat,
`record_kind`-discriminated body** (snake_case, one flat object — the
discriminant `record_kind` sits at the same level as the body's fields, exactly
like the envelope `type` on the wire). The 12 portable kinds (row D1):
`observation`, `knowledge`, `memory`, `directive`, `record_proposal`,
`evidence`, `artifact_contract`, `contract_validation`, `outcome_assessment`,
`promotion_event`, `context_use`, `context_use_feedback`.

**Common envelope:** `schema_version`, `record_id`, `lineage_id`, `record_kind`,
`record_status`, `scope`, `sharing_scope`, `sensitivity`, `observed_at`,
`valid_from`, `confidence`, `origin`, `evidence_links`, `record_links`,
`record_hash`, `provenance`, `extensions`.

| # | Requirement |
|---|---|
| **LR1** | A record **MUST** carry the required envelope members: `schema_version`, `record_id`, `lineage_id`, `record_kind`, `record_status`, `scope`, `sharing_scope`, `observed_at`, `origin`, `record_hash`, `provenance`. The rest are optional. |
| **LR2** | Records are **immutable**. A correction is a **new** record with a **new** `record_id` sharing the earlier record's `lineage_id`; a record is never mutated in place. |
| **LR3** | `record_status` is exactly three values — **`active` \| `retracted` \| `archived`** (row B5). `superseded` is **not** a status: supersession is **derived** from a later record on the same `lineage_id`, never stored. A host **MAY** keep richer internal statuses; the wire status is these three. |
| **LR4** | Temporal members (`observed_at`, `valid_from`) **MUST** match the protocol timestamp profile (SPEC.md §6.1/F4). `observed_at` is when the provider learned the record; `valid_from` bounds when its assertion was true in the world. |
| **LR5** | `confidence`, when present, **MUST** be in `[0, 1]`. |
| **LR6** | `record_kind` is **closed within `lifecycle/1.0`**; a new kind is a `lifecycle/1.x` addition. A receiver **MUST NOT** reject a record solely for carrying an unrecognised member (SPEC.md §13 U1); the strict JSON Schema is an authoring lint, not the interop contract. |
| **LR7** | `extensions` members and any vendor-specific `record_links.rel` **MUST** be namespaced (`vendor:name`, SPEC.md §13 U3) so a vendor field can never collide with a member this profile defines or later reserves. |

### 4.1 Directive records (resolves B3/B5)

A `directive` may exist as one immutable, provenance-bearing record kind that a
provider stores and serves. Carrying a directive record is **not** a frame
instructing a model: the host still decides whether any directive is admitted,
enforced, or authorized (ADR 0007 §4).

| # | Requirement |
|---|---|
| **LD1** | The **portable** `directive_kind` taxonomy is exactly **`preference` \| `rule` \| `constraint` \| `procedure`** (four kinds; ADR 0007 §4, row B3). `memory` and `fact` are **not** directive kinds — `memory` is its own record kind, `fact` is a `knowledge_kind`. The six-kind taxonomy in the superseded drafts is a host-runtime convenience, not a wire contract. |
| **LD2** | A `constraint` directive **MUST** carry `constraint_effect`, one of **`require` \| `forbid`** — **never `allow`**. Authorization stays host-side (ADR 0007 §3): a stored constraint is a value, not a grant. |
| **LD3** | `enforcement` is `advisory` \| `blocking`; absent ⇒ `advisory`. `blocking` is a **recorded intent**, not an enforcement grant — the host decides whether to enforce. |
| **LD4** | A `procedure` directive carries ordered `procedure_steps`. `promotion_stage`/`promotion_status` and pruning thresholds are **host** concerns and **MUST NOT** appear on the portable directive record (row B4, the build prompt's own rule). |

### 4.2 Records the protocol carries but does not execute (resolves D6/D7)

The protocol carries these record **schemas**; their **execution**, **judging**,
and **promotion decisions** are host concerns and stay out (ADR 0007 §3/§4).
This is the **schema-vs-execution split**: the wire moves the record, the host
acts on it.

| # | Requirement |
|---|---|
| **LX1** | `artifact_contract` carries named `requirements` (an open `requirement_kind` vocabulary; the reference validator recognises ten kinds). A `command` requirement **MUST** carry an `execution_approval_ref` — a pointer to an out-of-band approval, **not** an authorization to execute. Contract **execution** is host-side (row D6). |
| **LX2** | `contract_validation` records the **result** of validating a contract (`outcome: pass \| fail \| inconclusive`, optional per-requirement results). The act of validating and any semantic **judging** is host-side. |
| **LX3** | `outcome_assessment`, `promotion_event`, and `record_proposal` record **decisions the host already made**, as immutable events. **When** to promote (thresholds, policy, precedence) stays host-side (row D7); the protocol records the decision after the host makes it. `context/propose`, `context/promote`, and `context/validate` operations are **rejected** as protocol surface (ADR 0007 §3, row E4). |
| **LX4** | `context_use` and `context_use_feedback` overlap the core **usage-report (U1)** surface (SPEC.md §7.3). They are the **durable-record** projection of that signal; a CEP that also emits usage reports **MUST** keep the two reconcilable (the `context_use` booleans `selected`/`rendered`/`cited` carry the same meaning as attribution, SPEC.md §14 A2). |

## 5. Scope, sharing, idempotency, retention, identity

### 5.1 Portable scope (resolves E3)

| # | Requirement |
|---|---|
| **LS1** | The portable `scope` is the **7-key** object `{user_id, organization_id, repository_id, workspace_id, environment_id, session_id, task_id}`. Every key is optional; the **present keys are conjunctive (AND)**. |
| **LS2** | `sharing_scope` is one of **`user` \| `repository` \| `workspace` \| `organization`**, **conjunctive** with `scope`: it widens visibility within the scope keys present, it does not replace them. |
| **LS3** | `tenant_id` and `project_id` are **dropped** from the portable core (rows E2/E3): there is no cross-provider registry contract for them yet. A host **MAY** key on them internally; they **MUST NOT** appear in the portable `scope`. |

### 5.2 Idempotency, retention, identity, authorization

| # | Requirement |
|---|---|
| **LO-ID1** | Idempotency is keyed by `UNIQUE(authority_id, client_id, operation, idempotency_key)`. Same key + same command hash ⇒ replay the receipt as `duplicate`; same key + different hash ⇒ `idempotency_conflict`; an expired key ⇒ `idempotency_expired`; an existing `record_id` + different content ⇒ `record_identity_conflict`. **Never** silent re-execution. |
| **LO-ID2** | A provider that cannot honor a `requested_retention` **MUST** reject with `retention_rejected` **before** persistence — never silently shorten or lengthen. Accepted retention is recorded and enforced. |
| **LO-ID3** | The authenticated principal is resolved by the transport/auth layer; request-supplied identity labels **never** substitute. Sharing-scope authorization is enforced **before persistence and on every read** (`scope_denied`/`sharing_denied`). |
| **LO-ID4** | **Capability support never implies consent.** `consent_required` is a live error path even when a capability is advertised (cf. SPEC.md §4 C-series). Consent policy is sourced from the transport/host consent layer, not from capability advertisement. |

## 6. Operations (resolves D2/D5 and the `context/resolve` home)

Beyond core `context/query` + `context/verify`, this profile adds three
operations, each advertised in the `lifecycle` capability block (§2.1):

| Op | Purpose | Core issue it realizes |
| -- | ------- | ---------------------- |
| `context/records/append` | Durable, idempotent, batched write of records with an optional retention request; returns a receipt (`accepted`/`duplicate`/`rejected`). | #5 (write path) |
| `context/records/get` | Exact retrieval by record identity for the authorized principal. | #5 |
| `context/resolve` | Return the full source content of a `compact`/`reference` frame's `content_ref`, verifying `canonical_content_hash` before returning. | #50 |

### 6.1 `context/resolve` is a **profile-scoped** operation (explicit decision)

**Decision.** `context/resolve` is defined **by this profile**, not by the frozen
`contextgraph/1.0` core.

`SPEC.md` §6.4.1 freezes the `content_ref` handle and the `full`/`compact`/
`reference` frame shapes but **reserves `context/resolve` for a later additive
minor** — "there is no resolve envelope, and a host has no protocol-defined
operation that turns a `content_ref` into bytes … Resolution is reserved for a
`1.x` additive minor (§13)." Issue #50 deferred that exact operation to **#28**
(reconciliation row D3). This profile is where it lands. Within the profile:

| # | Requirement |
|---|---|
| **LO-R1** | `capabilities.resolve` tightens from the core **forward-declaration** (SPEC.md §6.4.1: "a shape check on the handshake, not an obligation a host can call") into a **callable contract**: a CEP advertising `resolve` **MUST** answer `context/resolve`. This is additive — a `1.0` host never emitted a resolve, so no deployed peer relied on its absence. |
| **LO-R2** | A resolve **MUST** verify the returned content against the `canonical_content_hash` the original `compact`/`reference` frame carried, and refuse to return content that does not match (`content_hash_mismatch`). The same digest-honesty discipline as SPEC.md §6.2/F5. |
| **LO-R3** | `content_ref.provider_id` names the exact provider that must answer; a fan-out host routes the resolve back to that provider. A handle that no longer resolves answers `reference_not_found` or `reference_expired` (§8). |
| **LO-R4** | Resolve rides the **same C-series consent gate** as `query` (SPEC.md §4): it transmits nothing new about the workspace, but may move source content off-machine if the provider is an egress provider. |

The base-spec `SPEC.md` §6.4.1 wording is unchanged: core 1.0 still ships no
resolve operation, so it remains honest ("ships no capability a host cannot
use"). The operation is exercised **only** inside the profile's capability
envelope, keeping the freeze boundary intact.

## 7. Provenance and attestation (resolves C5)

| # | Requirement |
|---|---|
| **LC1** | Every record carries structured `provenance`: `origin_provider_id`, `origin_authority_id?`, `producer_kind`, `producer_ref?`, `derivation_kind?`, `source_refs?`. `producer_kind` and `derivation_kind` are open vocabularies (recommended `human`/`agent`/`tool`/`system` and `summarization`/`inference`/`transformation`/`import`). |
| **LC2** | The envelope `origin` is a coarse class — `observed` \| `derived` \| `declared` \| `imported` — governed by the **origin→derivation validity matrix**: `observed`/`declared` **MUST NOT** carry a `provenance.derivation_kind`; `derived` **MUST** carry one; `imported` **MAY**. |
| **LC3** | A `RecordAttestation` is a **detached** signature over a record's `record_hash`: `{signed_record_hash, key_id, algorithm, attester_id, signature, issued_at}`. It travels as **ledger metadata beside** the record, **never inside** the record or its hash preimage (LH3), so key rotation never perturbs identity. Key rotation is by **key-id validity windows**. The attestation type is shared with issue #12. |
| **LC4** | The **signed message** is the domain tag `contextgraph/attest/1/record` (its ASCII bytes, no terminator) followed by the **32 raw bytes** of `signed_record_hash`. Both halves are fixed length, so the encoding is injective without a length prefix and any language can build it from the digest string alone. The tag is what stops a signature produced at another layer — or by an unrelated system that hashed the same JSON — from being presented as a record attestation: a frame commitment is domain-bound by construction, while a `record_hash` is a plain SHA-256 that anyone can also compute. `algorithm` is an open vocabulary; this revision defines `ed25519`, with the signature as **lowercase hex**. |
| **LC5** | A verifier **MUST** recompute the record's `record_hash` from the record in hand rather than trusting its stored member, then compare that against `signed_record_hash` **before** checking the signature. Trusting the stored member would let an attacker edit a record and rewrite its hash to match a stolen signature; checking the hash first means an operator is told the content moved, rather than being told the signature is bad. A verifier **MUST NOT** treat any outcome other than a successful verification as provisionally acceptable — "I cannot check this" and "this is good" are never the same answer. |

## 8. Typed error vocabulary (resolves D5)

Per SPEC.md §10 (X1) and §13 (U3), the error `code` vocabulary is **open and
namespaced**. The codes below are **profile-reserved** (unprefixed ⇒ owned by the
protocol/profile, SPEC.md §13 U3); a **vendor-specific** code **MUST** be
namespaced (`vendor:code`). An unrecognised code **MUST** be treated as
`internal` (SPEC.md §10 X1/X2). Errors carry **safe diagnostics only** — no
secret leakage (SPEC.md §11 C8).

| code | operation(s) | meaning | host reaction |
| --- | --- | --- | --- |
| `unsupported_capability` | any | the operation/representation/record kind was not advertised | do not retry; renegotiate |
| `unsupported_representation` | append / resolve | a representation the provider did not advertise (SPEC.md §10) | re-request `full` or skip |
| `unsupported_record_kind` | append | a `record_kind` this provider does not serve | narrow or skip |
| `invalid_record` | append | the record failed structural/schema validation | do not retry unchanged |
| `idempotency_conflict` | append | same `idempotency_key`, different command hash | do not retry; the key is spent |
| `idempotency_expired` | append | the idempotency key's window has elapsed | retry with a fresh key |
| `record_identity_conflict` | append | an existing `record_id` was re-submitted with different content | mint a new `record_id` on the same `lineage_id` |
| `retention_rejected` | append | the provider cannot honor `requested_retention` | lower the retention ask or skip |
| `consent_required` | append / get / resolve | capability is advertised but consent is not granted | obtain consent; do not retry blindly |
| `scope_denied` | get / resolve / append | the principal is not authorized for the record's `scope` | do not retry |
| `sharing_denied` | get / resolve | the record's `sharing_scope` excludes the principal | do not retry |
| `reference_not_found` | resolve | the `content_ref` names nothing resolvable | drop; treat contribution as empty |
| `reference_expired` | resolve | the handle's `expires_at` has passed | re-query for a fresh handle |
| `content_hash_mismatch` | resolve | returned content does not match `canonical_content_hash` | reject the content; report |
| `payload_too_large` | append | a record/batch exceeds the advertised payload limit | split and retry |
| `batch_too_large` | append / get | a batch exceeds the advertised batch limit | split and retry |
| `partial_failure` | append (batch) | some records in a batch were rejected; the receipt itemizes each | act per-item on the receipt |
| `unavailable` | any | transient overload / backing store down | retry with backoff |
| `internal` | any | provider fault (and the fallback for any unknown code) | report; count against health |

## 9. Conformance and fixtures (resolves the fixture-home + conformance [OPEN])

| # | Requirement |
|---|---|
| **LF1** | **`tests/fixtures/` is the canonical home** for lifecycle-profile example records and golden JCS/`record_hash` vectors — one fixture per `record_kind`; `record-hash-vectors.json`, holding the canonical JCS **text** and hash of every one of them; a detached `RecordAttestation` example carrying a real Ed25519 signature; and `record-attestation-key.json`, the published test key that signs it. The key is committed to a public repository and forgeable by anyone, which is the point: a vector nobody can reproduce is a shape example. Downstream implementations reconcile to these byte vectors (coordinating with the fixture-regeneration work, issue #52). |
| **LF2** | The record schema's `$id` **MUST** name this repository's GitHub-raw URL — the only host that serves these bytes, since this repository deploys no website ([ADR 0008](../adr/0008-deploy-topology-and-advertised-urls.md)), so there is no second copy to keep in sync. `schema/validate-examples.py` validates every fixture against the schema and pins the exact `$id` string, and `.github/scripts/check-deploy-hygiene.py` enforces the host rule repo-wide (the same discipline as the envelope schema). |
| **LF3** | `contextgraph-conformance`'s [`lifecycle_profile_examples`](../../contextgraph-conformance/tests/lifecycle_profile_examples.rs) suite round-trips every fixture through the reference Rust types, checks the profile envelope invariants (LR/LD/LC), **recomputes `record_hash` as the JCS-sha256 of the hashless record**, pins each fixture's canonical preimage text, and **verifies the attestation example under its published key** — so a fixture cannot merely assert a hash it does not satisfy, or carry a signature nothing can check. Every recomputation calls the library (LH5), not a copy of the rule kept in the suite: a second implementation living in the test is how a fixture set ends up agreeing with nothing that ships. |
| **LF4** | **Core conformance** is unchanged: a CEP is green on `contextgraph-conformance` for its declared read capabilities (SPEC.md §12). The **live HTTP-endpoint** profile suite (driving append/get/resolve over a real transport) is future work that rides the operation transport bindings (#5/#50/#13); until then this repo ships the record **schema**, the **JCS golden vectors**, and the round-trip/hash conformance above — the checkable, transport-independent core of the profile. |

## 10. Transport and security

A CEP is (typically) an HTTP provider, so core C4/C7/C8 bind unchanged: a host
treats it as **egress**, requires **TLS** for non-loopback, and **never logs its
credentials** (SPEC.md §11). The **auth scheme** (bearer / mTLS / OAuth) is the
transport layer's concern and coordinates with issue #13; it is **not**
re-specified here. Identity is resolved by that layer (LO-ID3); request-supplied
identity labels never substitute for it.

## 11. What this profile does not own

Observation extraction, confidence formulas and recurrence thresholds,
governance policy, review UI, automatic activation/publication/pruning
decisions, blocking authorization, artifact-contract **execution** and semantic
**judging**, prompt compilation and token budgeting, and product packaging are
**host/product** concerns (ADR 0007 §3). The rule of thumb is unchanged:
**mechanism in the protocol, policy in the host** — the protocol carries a value
or a recorded decision; it never authorizes acting on it.

---

*The reference implementation (Oxagen's platform-side Context Exchange Provider)
tracks this profile; where its build prompt and this document disagree on a wire
shape, **this document and the schema win**.*
