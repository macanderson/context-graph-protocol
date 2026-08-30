# 0010 — Provenance attestation: signing the chain, not just digesting it

**Status:** Accepted (`contextgraph/1.0`, additive)

## Context

Provenance integrity has been a headline guarantee since the first draft, and
`SPEC.md` §6.2 (F5) makes it checkable: a `file` provenance link **MUST** carry
`sha256:<64 lowercase hex>`, and the host can re-read the addressed bytes and
compare.

That construction detects **drift**. It does not establish **evidence**, and the
distinction has been quietly load-bearing in how this protocol is positioned.

A digest is tamper-evident only to a party that already trusts whoever recorded
it. It proves the bytes have not changed since someone wrote that number down —
it says nothing about who wrote it, or whether they were entitled to. The digest
and the frame it describes are produced by the same unauthenticated party, so a
provider willing to fabricate a frame is equally willing to fabricate its
digest, and every check in §6.2 passes. F5-bytes narrows this for the one case
where the host can independently re-read a source it already trusts, but that
covers `file` provenance on the host's own disk and nothing else: not a
`derivation` link, not an `episode`, not a remote provider's claim about a
corpus the host has never seen.

Two questions therefore had no answer:

1. **"Prove this citation is what the provider actually served."** An auditor
   holding a stored frame months later cannot distinguish a genuine one from a
   fabrication, because both are internally consistent.
2. **"Prove this frame was in the answer, without showing me the rest."** A host
   that served twelve frames had no way to substantiate one of them in
   isolation.

The gap is a signature. Nothing else closes it. Comparable systems reached the
same conclusion — `world-model-mcp` signs events with Ed25519 and Merkle-chains
them for offline verification — and for the audit and regulated-deployment
buyers, "we have a trace" and "we have evidence" are different products
separated by exactly this.

The lifecycle profile already anticipated the shape: `RecordAttestation`
(reconciliation row C5) is a detached Ed25519 signature over `record_hash`. But
it was a **declared type with no implementation** — the workspace carried no
signing or verification code and no cryptographic dependency at all — and it
covers the *record* layer, not the *frame* provenance chain that `context/query`
returns.

## Decision

Define provenance attestation at the frame layer, in three constructions,
specified normatively in `SPEC.md` §6.5 and implemented in
`contextgraph_types::attest`.

### 1. A hash chain, not a set of independent digests

Provenance links fold source-first into a hash chain, each step consuming the
previous head. A set of per-link digests binds each link's *content* but not the
chain's *shape*: links could be inserted, dropped, or reordered with every
individual digest still checking out. Truncating a provenance chain — dropping
the derivation step that would reveal a frame was summarized rather than quoted
— was undetectable. A chain makes it a hash mismatch.

An empty chain hashes to a domain-separated genesis value rather than to zero,
so "this frame claims no provenance" becomes a signed assertion rather than a
gap in coverage.

### 2. The signed preimage binds the frame's identity

The commitment covers the `(provider_id, frame_id, content_digest)` triple of
§6.3 **in addition to** the chain head.

This is not defensive detail; without it the scheme is a forgery primitive. Two
frames citing the same source produce the same chain head, so a signature over
the head alone can be lifted from an innocuous frame and stapled onto a
fabricated one — it verifies, and the evidence is invented. Including
`content_digest` further means the signature covers the frame's *bytes*, not
merely its name, so a provider cannot re-serve different content under a
previously signed frame id.

### 3. A length-prefixed encoding, not RFC 8785 (JCS)

The lifecycle profile canonicalizes `record_hash` with JCS, and that remains
right *there*: a record's hash covers an open-ended JSON document.

A provenance link is six optional strings, and for that shape JCS is a
liability. It makes every implementation depend on a conforming JSON
canonicalizer, whose number-formatting and Unicode-escaping rules are precisely
where cross-language implementations diverge silently. This protocol's whole
posture is that a provider in any language should be implementable from the spec
alone; requiring a JCS implementation to compute one hash contradicts that.

The encoding therefore serializes the typed fields directly, each length-
prefixed. Length prefixing is what makes the encoding injective: bare
concatenation is ambiguous, and without prefixes a link with `uri: "ab",
range: "c"` encodes identically to one with `uri: "a", range: "bc"` — a
collision an adversary *chooses* rather than searches for. The presence byte
distinguishing `None` from `Some("")` is load-bearing for the same reason.

### 4. RFC 6962 Merkle trees for result sets

A provider signing a whole answer commits to a Merkle root over frame
commitments in canonical order, with inclusion proofs. RFC 6962's distinct leaf
(`0x00`) and interior (`0x01`) prefixes prevent an interior node's hash from
being presented as a leaf.

The common "duplicate the last leaf on an odd level" shortcut is rejected: it
admits two distinct leaf sets with the same root. That ambiguity is tolerable in
a checksum and disqualifying in evidence.

Inclusion proofs carry the leaf count as well as the index, because a root alone
does not pin the tree's size and a verifier that ignores it can be shown a proof
from a differently-shaped tree.

### 5. Cryptography is optional; the preimage rule is not

Hashing and signature verification sit behind an off-by-default `attestation`
feature. `contextgraph-types` is published as "zero dependencies beyond serde",
and that promise is a real adoption argument for the crate that every
implementer ports from — an unconditional `ed25519-dalek` would spend it.

`ProvenanceAttestation` itself always compiles. A host must be able to parse,
relay, and store an attestation it was not built to check, for the same reason
it must relay a frame kind it does not recognize (§13 U2).

### 6. The protocol defines the preimage, not key custody

`frame_commitment` and `merkle_root` are public, so a provider holding keys in
an HSM, a KMS, or a hardware token signs the 32 bytes with its own backend and
never hands this crate a secret. `sign_frame_attestation` exists for providers
content to sign in-process.

`algorithm` is a string rather than an enum, so adopting a post-quantum scheme
is an additive change; a verifier that does not recognize it reports
`UnknownAlgorithm` and declines.

### 7. Verdicts are named, and unverifiable ≠ invalid

Verification returns a named verdict, not a boolean. "This frame was altered
after signing" (`CommitmentMismatch`), "the signature is forged or the key is
wrong" (`BadSignature`), and "I was handed a truncated key" (`MalformedKey`) call
for entirely different responses; collapsing them sends an operator hunting a
key-management bug when the finding is tampering. The commitment is compared
*before* the signature so the more informative failure is the one reported.

F9 requires that an unverifiable attestation degrade a frame to *unattested*
rather than disqualify it. A host that dropped such frames would hand any peer a
denial-of-service primitive: attach a malformed attestation, watch the evidence
disappear.

## Alternatives considered

**Do nothing; F5 digests are enough.** Rejected. They answer a different
question, and the positioning around audit and provenance is currently writing
cheques the construction cannot cash.

**Reuse `RecordAttestation` for frames.** Rejected. Five of six fields match, but
the two sign different preimages, and a shared type invites exactly the confusion
the domain separation exists to prevent — presenting a record attestation as a
frame attestation. The cryptography already refuses it; the type system should
make it unsayable.

**Put the attestation inside the frame.** Rejected. It would perturb the frame's
content-addressed identity on every re-signing and key rotation, and make
countersigning impossible. Detached, as the record layer already established.

**A new `contextgraph-attest` crate rather than a feature.** Rejected. The
preimage is coupled to the wire types' field set — if `Provenance` gains a field
the encoding must change in lockstep — and a separate crate lets the two version
independently, which is how a cross-language hash rule silently forks. A feature
flag keeps one canonical definition and compile-checked coupling while
preserving the zero-dependency default.

**Mandate signing.** Rejected for this revision. Attestation is optional: a
conformant provider may serve none, and a conformant host may verify none. What
is not optional is the *construction* — an attestation that exists must be
computed exactly one way, or two implementations will disagree about whether the
same evidence is genuine.

## Consequences

- `contextgraph-types` gains an optional `attestation` feature
  (`sha2`, `ed25519-dalek`). The default dependency set is unchanged: serde only.
- `SPEC.md` gains §6.5 and requirements F6–F9.
- `contextgraph-types/tests/attestation_vectors.rs` publishes the byte vectors a
  reimplementation in another language reconciles against. A diff in that file is
  a wire-breaking change requiring a new major family.
- Not yet done, and tracked as follow-up work: host-side verification wired into
  composition, an `attestation` conformance check with an adversarial
  `--misbehave` mode, attestations carried in the `frames` envelope and the JSON
  Schema, and key distribution/rotation — which is deliberately out of scope
  here, since the protocol specifies the preimage and not the PKI.

  The wire carriage on that list has since landed:
  [ADR 0014](./0014-attestations-on-the-wire.md) puts the attestation on the
  `frames` result, in the JSON Schema, and in `examples/`.
