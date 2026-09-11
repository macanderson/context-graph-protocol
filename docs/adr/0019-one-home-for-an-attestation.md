# 19. One `FrameAttestation`, one wire home, and what `attester_keys` is for

- Status: accepted
- Date: 2026-09-11
- Completes [ADR 0014](0014-attestations-on-the-wire.md). Narrows a consequence
  of [ADR 0016](0016-attestation-trust-roots.md). Does not decide the
  trust-on-first-use tier, which stays open.

## Context

Three issues were worked in parallel off one base. #88 gave the host a way to
verify an attestation, #90 put an attestation on the wire, and #89 wrote the
adversarial conformance check for both. Each needed somewhere to put "a
provenance attestation bound to a frame", and none could see the others' answer.
All three shipped.

What landed was **three** Rust types named `FrameAttestation` and **two** places
on the wire to put the same signature:

| Definition | Shape |
|---|---|
| `contextgraph-types/src/attest.rs` | `frame: FrameId`, `attestation: Option<…>`, `inclusion_proof: Option<…>` |
| `contextgraph-host/src/trust.rs` | `frame_id: String`, `attestation: ProvenanceAttestation` |
| `contextgraph-host/src/wire.rs` | `frame_id: String`, `attestation: ProvenanceAttestation` |

`Envelope::Frames` was `{ id, result, attestations }` while `ContextQueryResult`
already carried `frame_attestations` and `result_attestation`. So one signed
answer had two encodings, and nothing anywhere said which one wins when they
disagree.

The spec had the same split. `SPEC.md` carried **two** sections numbered §6.5.5,
one describing each encoding, and the JSON Schema carried two `$defs` entries
named `FrameAttestation` and two named `ProvenanceAttestation` — duplicate JSON
object keys, where the second silently shadows the first in every parser we
tried. A reader could have read either one and believed they had read the
contract.

For a protocol whose whole pitch is "enforced by contract, not convention", two
encodings of one fact with no tie-breaking rule is the exact defect it exists to
prevent. It is worse than a wrong rule, because a wrong rule is at least
checkable.

## Decision

**1. `contextgraph_types::FrameAttestation` is the type.** The two in
`contextgraph-host` are deleted and the canonical one is re-exported from that
crate, so a host author still has one import. It belongs in the types crate
because it is a wire type that rides `ContextQueryResult`, and because a
verifier in another language reads that crate to know what to build.

**2. `ContextQueryResult` is the wire home, and the only one.**
`Envelope::Frames.attestations` is removed. The reasoning is ADR 0014's and has
not changed: an attestation is a property of the answer, exactly like
`truncated`, and an in-process provider that returns a `ContextQueryResult`
without ever constructing an envelope must still be able to sign what it serves.
The envelope carries what the transport needs and nothing else.

**3. There is one provider seam, not two.** `ContextProvider::query_attested`
and `AttestedQueryResult` are removed. They existed for the months when the
`frames` envelope had nowhere to put an attestation, so an in-process provider
needed a side channel. It has one now: a signing provider populates
`frame_attestations` on the result `query` already returns. Keeping both would
have moved the duplication from the wire to the Rust API rather than removing
it — a host could still have been handed signatures that disagreed with the
frames they cover. `TrustStore::check_result` now reads the evidence off the
result it is checking, so passing a mismatched pair is not expressible.

**4. `handshake_ack.attester_keys` is a construction anchor, and stays.** ADR
0016 made the operator the trust root and specified no PKI. A key self-asserted
at the handshake looks like it contradicts that, and the question was raised
that way (#161). It does not, and the distinction is worth writing down rather
than leaving in a doc comment:

- A key published by the party under audit settles whether an attestation is
  **built** the way §6.5 requires. That is what the F6–F9 conformance check
  needs, and construction is the half §6.5 makes mandatory.
- It settles nothing about **who** signed. It is not a trust root, it does not
  enter `TrustStore`, and the reference host never consults it when deciding
  whether a frame is attested.

So it neither adopts nor forecloses a trust-on-first-use tier. Pinning a
published key — recording continuity as a labelled second tier, strictly below a
configured key — remains **open under #130**, along with the tier's own ADR, a
`TrustStore` that records a key's origin, and an `AttestationState` that reports
which tier verified it. #130's step 1 (an additive wire field carrying the key)
is what shipped; its steps 2 through 4 have not, and this ADR does not decide
them.

**5. A host verifies the proof-only shape.** A `FrameAttestation` may carry a
per-frame signature, an inclusion proof in the signed `result_attestation` root,
or both — the middle case being the cheapest honest way to sign an answer, one
signature instead of *n*. The host previously checked only the first, so a
provider that chose the cheap shape read as unattested: the shape the type went
out of its way to make representable was the one nothing could verify.
`contextgraph_types::verify_frame_inclusion` now recomputes the root from the
frame's own commitment and checks the answer-level signature over it, beside
`verify_frame_attestation` and under the same content-binding rule (ADR 0018).

## Consequences

- Breaking for `contextgraph-host` at the Rust level: `AttestedQueryResult`,
  `ContextProvider::query_attested` and two `FrameAttestation` types are gone,
  `TrustStore::check_result` loses its third argument, and
  `Host::query_provider_attested` returns a `ContextQueryResult`.
- Breaking on the wire only for a provider that used the envelope member. No
  published crate carried it: `contextgraph-types` on crates.io is `0.1.2`,
  which predates all of this, and `contextgraph-host` has never been published
  (#102). The removal costs nothing anybody depends on, which is why it happens
  now rather than after the first release.
- `AttestationState` gains `UnusableEvidence`: an entry that named a frame and
  could not be turned into a check — an inclusion proof with no signed root, or
  an entry carrying neither member. F9 treats it as unattested for every
  decision. It is named rather than folded into `Unattested` because the two say
  different things about the provider, and only one of them is worth an
  operator's attention.
- `contextgraph_types::MAX_INCLUSION_PATH_STEPS` caps a proof path at 64 steps.
  Each step costs a hash and the path arrives from the provider; 64 steps
  describes a tree over 2⁶⁴ leaves, so the cap binds nothing real and bounds the
  work a peer can buy.
- `ContextQueryResult::unattested(frames, truncated, dropped_estimate)` exists so
  that a caller written before §6.5 — `contextgraph-types` `0.1.2` consumers,
  chiefly `macanderson/stella` — bumps with a one-line change rather than a
  rewrite. `Default` plus struct-update syntax is the other way, and both are
  documented on the type.

## Alternatives considered

**Keep both encodings and write a precedence rule.** "The result wins; the
envelope member is advisory." It is one sentence, and it would have to be
implemented identically by every host in every language, unenforced by anything.
A rule nothing checks is a convention, and the whole claim of this protocol is
that it does not run on conventions.

**Make `contextgraph-host`'s type the canonical one.** Its `frame_id: String`
is smaller and easier. It also cannot express the proof-only shape, cannot
distinguish two frames that share an id and differ in bytes, and is not
reachable from the crate a non-Rust implementer reads. The smaller type is the
one that loses information the protocol is about.

**Remove `attester_keys` as a side effect.** It was the issue's opening
proposal, on the reading that it contradicted ADR 0016. It does not, it is load
bearing for the F6–F9 conformance check and for `SPEC.md` §6.5.5, and ripping a
shipped, spec'd, exercised wire field out inside a refactor is exactly the
"landed as a side effect" move the issue objected to. Writing down what it is
for is the smaller and more honest change.
