# 0017 — `record_hash` and `RecordAttestation`: implementing the record layer's identity

**Status:** Accepted (`contextgraph/lifecycle/1.0-draft`, additive)

## Context

The Context Exchange Provider profile has said since its first draft that a
record is content-addressed: `record_hash` is `sha256:<hex>` over the RFC 8785
(JCS) canonicalization of the record with its own `record_hash` member removed
(`LH1`), and `RecordAttestation` is a detached Ed25519 signature over that hash
(`LC3`).

Both were prose and a struct. There was no canonicalization code in the
workspace, no record hashing anywhere a provider could call, and no signature
verification at all. What existed instead:

- A **test helper.** `contextgraph-conformance`'s `lifecycle_profile_examples`
  suite carried its own private copy of the hashing rule and checked the
  fixtures against it. It was correct, and it was the only implementation — so
  the suite proved the fixtures agreed with the suite.
- A **placeholder signature.** `tests/fixtures/record-attestation.json` carried
  49 bytes of DER-shaped filler where a 64-byte Ed25519 signature belongs, with
  no key published. No implementation could reproduce or refute it. It sat in
  the directory `LF1` calls the canonical home for the profile's golden vectors.
- An **unstated preimage.** "A detached signature over `record_hash`" does not
  say what bytes are signed. Nothing had signed anything, so nothing had had to
  decide.

That is the same gap ADR 0010 closed at the frame layer, one layer down
(issue #96).

## Decision

Implement both in `contextgraph_types::record_attest`, behind two new
off-by-default features, and publish vectors that make the `LF1` claim true.

### 1. RFC 8785 (JCS) is right here, and it is delegated

ADR 0010 §3 rejects JCS for a provenance link, and that decision stands
unchanged: a link is six optional strings, and making every implementer obtain a
conforming JSON canonicalizer to hash six strings is a tax with no return.

A record is the opposite shape. It is an open-ended JSON document — an
extensible body, a map of namespaced extensions, floating-point confidences —
and there is no typed encoding to write down that stays correct as the profile
grows a member. The canonicalization has to be generic, and RFC 8785 is the one
generic rule with implementations in several languages to reconcile against.

The cost is real: JCS number serialization is ECMAScript `Number::toString`,
whose exponent thresholds and shortest-round-trip digit selection are exactly
where independent implementations diverge in silence. So the rule is
**delegated, not hand-rolled** — `serde_json_canonicalizer` (MIT), which routes
number formatting through `ryu-js`, the Boa engine's ECMAScript formatter
(Apache-2.0 OR BSL-1.0). The crate was already a dev-dependency of the
conformance suite; this promotes it to a pinned workspace dependency so the
library and the suite that checks it cannot canonicalize with two different
versions.

Delegation is not a substitute for evidence. `record_attest`'s tests check the
canonicalizer against **RFC 8785's own published vectors**: §3.2.4's
hexadecimal byte listing for the specification's worked example, §3.2.3's
property-sorting test data, and Appendix B's Table 1 of IEEE 754 bit patterns
and their required ECMAScript text — the `-0` case, the `1e+21` and `0.000001`
exponent thresholds, the round-to-even sample, and the rest.

### 2. The omitted member is removed, not blanked

`LH1` says removed, and this ADR records what that buys rather than treating it
as arbitrary: a record hashes identically whether it carries no `record_hash` at
all, the correct one, or a wrong one. A producer therefore computes the hash of
the record it is about to publish without first inventing a placeholder, and a
verifier never has to know which placeholder the producer chose. A blanking rule
would have made the placeholder itself part of the interop contract — one more
value to get wrong in another language, for nothing.

Only the **top-level** member is removed. A `record_hash` nested inside
`extensions` or a body member is ordinary content and stays in the preimage;
dropping it would let a producer hide a value from the signature.

### 3. The signature is domain-separated

A frame commitment is domain-bound by construction — it is
`SHA256(domain::FRAME ‖ …)`, so nothing else in this protocol produces those
32 bytes. A `record_hash` is a plain SHA-256 over a JSON document, which any
number of unrelated systems also compute, and signing it raw would make one
Ed25519 signature mean whatever the presenter says it means.

So the signed message is:

```text
"contextgraph/attest/1/record" ‖ <32 raw bytes of record_hash>
```

Both halves are fixed length, so the encoding is injective without a length
prefix, and any language can build it from the digest string alone. This is
additive to `LC3` rather than a change to it: the signature is still over the
record's hash, and nothing had implemented the ambiguous reading.

Verification recomputes the record's hash rather than reading the stored member,
so the obvious laundering move — edit the content, then rewrite `record_hash` so
the record is internally consistent again — is caught as a
`CommitmentMismatch` rather than passing.

### 4. Two features, not one

`attestation` stays exactly what ADR 0010 §5 made it: `sha2` and
`ed25519-dalek` for the frame layer's length-prefixed encoding. The record layer
adds:

- **`record-hash`** — `sha2`, `serde_json`, `serde_json_canonicalizer`. Content
  addressing with no signatures, which is all a provider keying idempotent
  replay needs.
- **`record-attestation`** — `record-hash` plus `attestation`, for the
  signatures.

A frame-only consumer never pays for a JSON canonicalizer, and a provider that
only content-addresses never pays for Ed25519. The crate's "zero dependencies
beyond serde" promise holds for everyone who opts into nothing, which is the
default.

`AttestationVerdict` is **shared** with the frame layer rather than duplicated.
The verdict is a result vocabulary, not a signed preimage: an auditor needs the
same distinctions at both layers — "this is forged" against "I cannot check
this" against "the content moved under the signature" — and two enums with the
same variants would drift. The types that must stay distinct, and do, are
`RecordAttestation` and `ProvenanceAttestation`.

### 5. The vectors are published, and checked from two languages

`LF1` promised `tests/fixtures/` holds golden JCS/`record_hash` vectors. It held
records with hashes, which is half of it: a digest cannot say *where* two
implementations diverged, only that they did.

- `tests/fixtures/record-hash-vectors.json` publishes the exact JCS preimage
  **text** of every record fixture beside its hash, so an implementer whose
  digest disagrees gets a byte diff instead of a mystery.
- `tests/fixtures/record-attestation.json` now carries a real Ed25519 signature,
  and `tests/fixtures/record-attestation-key.json` publishes the seed, the
  public key, and the signed message. The key is a test key, committed to a
  public repository, and labelled as one everywhere it appears — publishing it
  is the point, because a vector nobody can reproduce is a shape example.
- `contextgraph-types/tests/record_vectors.rs` carries the same values inline,
  so they travel inside the published crate rather than depending on files at
  the repository root.

The conformance suite recomputes all of it through the **library**, not through
a copy of the rule, which is what makes `LF3` a statement about shipped code.
`schema/validate-examples.py` then checks the two properties that make a vector
usable to someone who has neither Rust nor a JCS library: that the published
canonical bytes hash to the published digest, and that those bytes parse back to
the fixture with its `record_hash` removed. Neither check claims Python
canonicalizes JSON the way RFC 8785 does — it does not, and a check that
pretended otherwise would be an agreeable coincidence rather than evidence.

## Consequences

- A provider can content-address a record and attest to it with the reference
  crate instead of reimplementing a rule from prose.
- The record fixtures' hashes are **unchanged** by this work. The library
  reproduces the rule the suite's private helper already implemented, which is
  the evidence that this is an implementation rather than a redefinition.
- `LC3` gains a normative preimage. A provider that had already shipped a
  signature over the bare digest — none exists, since nothing implemented
  signing — would have to re-sign.
- The frame layer is untouched. ADR 0010's argument against JCS for a provenance
  link is unaffected by this ADR adopting JCS for a record, and the two modules
  each say so where a reader will meet the apparent contradiction.
