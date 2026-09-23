# 21. An attestation's metadata stays outside the signature in `contextgraph/1`, and says so

- Status: accepted
- Date: 2026-09-23
- Resolves #185. Adds requirement **F18** to `SPEC.md` §6.5.2. Builds on
  [ADR 0010](0010-provenance-attestation.md) (the preimage),
  [ADR 0016](0016-attestation-trust-roots.md) (the operator is the trust root)
  and [ADR 0018](0018-signing-a-frame-requires-a-content-digest.md) (saying out
  loud what a signature does not bind).

## Context

A `ProvenanceAttestation` has six members. The signature is over the §6.5.2
frame commitment (or the §6.5.3 root), and `signed_commitment` is that
commitment; `signature` is the signature. The other four — `key_id`,
`algorithm`, `attester_id`, `issued_at` — are not in any preimage.

Two of those are harmless: rewriting `key_id` selects a different key and the
signature fails against it, and rewriting `algorithm` yields a failed signature
or an algorithm the verifier declines (F8). Both fail in the safe direction.

The other two are not. Anyone who handles an attestation in transit can
rewrite `attester_id` to name any authority and `issued_at` to any instant, and
`verify_frame_attestation` still returns `Valid` — content-binding and all. The
reference host then carried the rewritten `attester_id` forward in
`AttestationState::Attested`, documented as "who is accountable for the claim".
Accountability is exactly what an unsigned field cannot carry.

`SPEC.md` said none of this. §6.5.2 had already set the precedent of stating a
signature's blind spot plainly — "What a signature binds when `content_digest`
is absent" — and the same sentence applied here and had not been written.

The issue laid out two answers and noted they are not interchangeable:

1. **Document it.** Say what the commitment covers, add a rule that a verifier
   must not present unsigned metadata as signed, and tell hosts to mark it.
2. **Bind it.** Put `attester_id` and `issued_at` into the preimage.

## Decision

**1. In `contextgraph/1`, document the boundary and make presenting it as
signed a violation.** §6.5.2 now tabulates all six members and states the
consequence for `attester_id` and `issued_at` specifically. **F18**: a verifier
**MUST NOT** present `attester_id`, `issued_at`, `key_id` or `algorithm` as
covered by the signature, and a host **SHOULD** mark `attester_id` and
`issued_at` as unverified wherever it surfaces them. §6.5.4 adds that a verdict
is a statement about the commitment and the key, and nothing else.

**2. The reference implementation carries the boundary in its types, not only
in prose.** `contextgraph_types::attest`'s module docs and each field's docs
state it, because they are what an implementer in another language reads.
`AttestationState::Attested` now documents `key_id` as the one identity it
vouches for and `attester_id` as the attestation's unverified claim, and
`AttestationState::unverified_attester_id` exposes that claim under a name that
carries the mark to every call site. The host carries no `issued_at` at all: it
makes no decision on it, and a state that exposed it would invite one. This is
additive; the `Attested` variant keeps its shape, so no caller breaks.

**3. Binding the metadata is a question for a future major family, and this
ADR does not answer it there.** Binding changes the commitment of every
attestation ever produced — every published vector, every archived signature —
which §13 U4 and §3.1 make a new major family. That rules it out of a `1.x`
minor, so the choice inside `contextgraph/1` is forced. What remains open is
whether `contextgraph/2` *should* bind them, and the honest answer is that
binding is the easy half:

- **Binding `attester_id` buys little.** Who stands behind a verified
  attestation is already answered, correctly, by the key that verified it and
  the operator who trusted that key for that provider (ADR 0016). A signed
  `attester_id` would be a string the key-holder chose — self-asserted, like
  `provider.name` under H2 — and would add a second, weaker answer to a
  question the trust store already answers. If `contextgraph/2` wants signed
  attribution to a party distinct from the key, it needs a delegation or
  certificate story, not a signed string.
- **Binding `issued_at` is the door to expiry, replay windows and revocation,**
  and that is a protocol-direction decision, not a field placement. A signed
  timestamp is only meaningful with a policy for consuming it — what a verifier
  does with an old attestation, how that interacts with key rotation and key
  lapse (#136), whether a clock the signer controls is trusted at all, and
  whether a third-party timestamp (RFC 3161, a transparency log) is wanted
  instead. Signing the field without that policy would produce a timestamp that
  is authenticated and still means nothing.

So the recorded position is: **`contextgraph/2` should bind `issued_at` if, and
only if, it also defines what a verifier does with it**, and should not bind
`attester_id` as a bare string. Until a major family takes up that design, the
field is metadata and F18 says how to treat it.

## Consequences

- The boundary has a witness in each crate that owns it.
  `contextgraph_types::attest`'s
  `rewriting_attester_id_or_issued_at_leaves_the_verdict_valid` pins that
  rewriting both leaves the verdict `Valid` for a frame attestation and for a
  result-set root, and `rewriting_the_algorithm_fails_safe` pins the safe
  direction. `contextgraph_host::trust`'s
  `unsigned_attestation_metadata_is_echoed_as_a_claim_never_verified` pins that
  the host still reports the frame attested, echoes the rewritten
  `attester_id` only as the unverified claim, and never verifies a rewritten
  `key_id`. If any of these fails, the preimage moved — which is a new major
  family, not a refactor.
- No wire change, no vector change, no schema change.
- The record layer has the same shape: `RecordAttestation` signs the
  `record_hash` alone, and its `attester_id` and `issued_at` are likewise
  unsigned. That profile is documented separately
  (`docs/profiles/context-exchange-provider.md`) and is outside this ADR; the
  same reasoning applies to it, and it should state the same boundary.

## Alternatives considered

**Bind them now.** The only change that makes the fields trustworthy, and it
cannot ship inside `contextgraph/1` without breaking every signature ever
issued. Doing it in `contextgraph/2` without the expiry and delegation design
above would spend a major version to sign two fields nothing is specified to
read.

**Drop `attester_id` and `issued_at` from the wire.** Honest in a way —
nothing unsigned, nothing to misread — but U4 forbids deleting a defined field
inside a major family, and both have legitimate unsigned uses: a human-readable
label beside a key fingerprint, and a provider's own record of when it signed.

**Leave it undocumented.** Two implementations would draw the boundary
differently, and the one that draws it generously would render forged
attribution as signed. That is the defect.
