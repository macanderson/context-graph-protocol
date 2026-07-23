# Profile: Context Exchange Provider (CEP) — DRAFT SKELETON

> **Status: draft skeleton, not normative.** This frames issue #28 and marks the
> decisions a real profile needs. It is co-developed with the first
> implementation (Oxagen's platform-side Context Exchange Provider) and **must
> not** be treated as a frozen contract. Sections marked **[OPEN]** are
> maintainer/implementer decisions, not settled by this document.

## Why a profile, not core

`contextgraph/1.0` is a **read** protocol: a host queries providers for
budgeted, provenance-carrying frames and optionally revalidates them
(`context/verify`). It deliberately excludes the write path (`context/upsert`,
issue #5), push invalidation (`subscribe`, issue #6), and content resolution
(`context/resolve`, issue #50) — each was removed or deferred pre-freeze
(ADR 0004; SPEC.md §6.4.1) precisely because core 1.0 had no consumer that
forced their design and freezing an unexercised operation is the
dead-capability anti-pattern.

A **Context Exchange Provider** is the consumer that forces those designs. It is
a provider that, beyond answering `context/query`, offers a **durable,
multi-tenant, auditable exchange** of context records: append with idempotency,
retrieval by identity, content resolution, retention commitments, and signed
attestations. That is a larger contract than a read-only provider, and it earns
its own **profile** layered on the `contextgraph/1` family rather than bloating
the core every provider must implement.

This profile is also the concrete path to GOVERNANCE freeze **criterion 1** (two
independent implementations): the reference host + crates on one side, a genuine
third-party CEP on the other.

## Relationship to the core protocol

- A CEP **MUST** be a conformant `contextgraph/1.0` provider first: it passes
  `contextgraph-conformance` for its declared capability set. The exchange
  operations are **additive** on top, gated behind capability advertisement.
- Profile identifier: **[OPEN]** the implementation targets
  `cgep/lifecycle/1.0-draft` as a profile version distinct from the wire
  `contextgraph/1.0-draft`. Decide whether the profile version rides in the
  handshake capability document, a separate profile-version field, or a
  namespaced capability — and how a host discovers CEP support. The core
  major-family rule (§3.1) and extensibility rules (§13) apply unchanged.

## Operations this profile adds (beyond core `context/query` + `context/verify`)

| Op | Purpose | Core issue it realizes |
| -- | ------- | ---------------------- |
| `context/records/append` | Durable, idempotent, batched write of context records with optional retention request; returns a receipt (`accepted`/`duplicate`/`rejected`). | #5 (write path) |
| `context/records/get` | Exact retrieval by record identity for the authorized principal. | #5 |
| `context/resolve` | Return the full source content of a `compact`/`reference` frame's `content_ref`, verifying `canonical_content_hash` before returning. | #50 / SPEC.md §6.4.1, [docs/sketches/resolve.md](../sketches/resolve.md) |
| *(deferred)* change feed / subscribe | Push staleness/invalidation. | #6 ([docs/sketches/push-invalidation.md](../sketches/push-invalidation.md)) |

## Contract surface a CEP profile must pin (skeleton — details **[OPEN]**)

1. **Canonical hashing.** Records are content-addressed. The reference
   implementation uses RFC 8785 JCS with `record_hash` omitted from its own
   preimage, and a separate `command_hash` over `(record_hash + requested
   retention + behavior-changing options)`. **[OPEN]** adopt JCS normatively and
   ship golden vectors (see Cross-repo fixtures below), including a
   number/integer policy.
2. **Idempotency.** `UNIQUE(authority_id, client_id, operation, idempotency_key)`.
   Same key + same command hash ⇒ replay the receipt as `duplicate`; same key +
   different hash ⇒ `idempotency_conflict`; expired ⇒ `idempotency_expired`;
   existing record id + different hash ⇒ `record_identity_conflict`. Never
   silent re-execution.
3. **Retention.** A provider that cannot honor a `requested_retention` **MUST**
   reject (`retention_rejected`) before persistence — never silently shorten or
   lengthen. Accepted retention is recorded and enforced.
4. **Identity & authorization.** The authenticated principal is resolved by the
   transport/auth layer; request-supplied identity labels never substitute.
   Sharing-scope authorization (user / repository / workspace / organization) is
   enforced before persistence and on every read. **Capability support never
   implies consent** (`consent_required` is a live error path).
5. **Attestation.** Append and publication receipts carry a detached ed25519
   attestation (`signed_record_hash`, `key_id`, `algorithm`, `attester_id`,
   `signature`, `issued_at`) as ledger metadata, never inside the record hash.
   Key rotation via key-id validity windows.
6. **Error vocabulary.** The reference implementation names ~24 typed codes
   (`unsupported_capability` … `partial_failure`). Per core X1/§13 U2, the CEP
   error vocabulary is **open and namespaced** (`cgep:...` or bare within a
   reserved profile namespace — **[OPEN]**), and errors carry safe diagnostics
   only (no secret leakage, cf. core C8).
7. **Transport & security.** CEP is an HTTP provider, so core C4/C7/C8 bind: a
   host treats it as egress, requires TLS for non-loopback, and never logs its
   credentials. Auth scheme (bearer / mTLS / OAuth) is **[OPEN]** and coordinates
   with issue #13.

## Conformance

- **Core:** green on `contextgraph-conformance` for the CEP's declared read
  capabilities — a checkable claim, unchanged.
- **Profile:** a CEP-specific suite exercising append/get/resolve idempotency,
  hash verification, retention rejection, authorization matrix, and attestation
  verification. **[OPEN / blocked]** running the Rust conformance suite against
  the HTTP endpoint is gated on the protocol repo shipping the lifecycle
  capability; until then the profile suite lives with the implementation and this
  repo ships only the shared hash/JCS golden vectors.

## Cross-repo fixtures

Golden JCS/`record_hash`/`command_hash` vectors are the interop spine between the
protocol repo, this profile, and downstream implementations. The reference
implementation authors them under its own tree; **[OPEN]** decide the canonical
home (this repo's `tests/` vs the implementation's `fixtures/`) and reconcile to
byte-identical vectors, coordinating with the fixture-regeneration work (issue
#52) so a CEP and the core suite validate against the same bytes.

## Open decisions rolled up (for the issue-#28 design discussion)

- **[OPEN]** Profile-version identifier and handshake discovery mechanism.
- **[OPEN]** Whether `context/resolve` is specified *in* this profile or as a
  standalone `1.x` core additive minor that the profile references (SPEC.md
  §6.4.1 currently reserves it for core `1.x`).
- **[OPEN]** JCS library/number policy; attestation key custody; consent policy
  source; get-batch limits (mirrors of the implementation's own open list).
- **[OPEN]** How much of §"Contract surface" is normative in the *protocol* repo
  vs owned by the implementation's build prompt (today the build prompt is
  normative and this is a summary).

---

*Reference implementation in progress: Oxagen platform-side Context Exchange
Provider (`packages/context-exchange`, `apps/api/src/routes/cgep/`). This
skeleton summarizes its published spec; the implementation's build prompt is the
current source of truth for wire details until this profile is ratified.*
