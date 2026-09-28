# 31. A result-set attestation is checked once per answer, and a proof's shape before its hashes

- Status: accepted
- Date: 2026-09-28
- Resolves #133. Completes the host half of
  [ADR 0019](0019-one-home-for-an-attestation.md) §5 (the proof-only shape) and
  builds on [ADR 0010](0010-provenance-attestation.md) (the Merkle root) and
  [ADR 0014](0014-attestations-on-the-wire.md) (where it rides). Adds
  normative text to `SPEC.md` §6.5.3; no wire, schema or vector change.

## Context

`SPEC.md` §6.5.3 defines an RFC 6962 Merkle root over a whole answer, with an
`InclusionProof` per frame, so a provider can sign an answer **once** — cheaper
than *n* signatures, and the only construction that supports selective
disclosure. #133 found the host consuming none of it. ADR 0019 §5 then taught
`TrustStore::check_result` the proof-only entry shape, which closed the headline
gap: a frame proven under a signed root reads as attested through the same
`AttestationState` and the same audit field as a per-frame signature.

Two of #133's requirements were still unmet, and both are about what an
attacker can make a host do:

1. **The root signature was verified once per frame.** Each proof-only entry
   went through `verify_frame_inclusion`, which recomputes the root from the
   frame and verifies the answer-level signature over it. For *n* frames that
   is *n* Ed25519 verifications of one signature — the cost the root exists to
   avoid, paid by the verifier instead of the signer.
2. **`leaf_count` was checked only as `leaf_index < leaf_count`.**
   `InclusionProof::leaf_count` is part of the proof precisely so a verifier
   cannot be shown a proof from a differently-shaped tree, and a verifier that
   ignores it has the bug the field exists to prevent. `root_from_proof` walked
   whatever path it was handed, of any length up to the 64-step cap, with any
   sides — so a proof's stated tree and its actual path could disagree, and the
   only thing that noticed was a root comparison after the hashing was done.

## Decision

**1. The root signature is verified at most once per answer.**
`TrustStore::check_result_signed_as_at` checks the `result_attestation` lazily
— only when the first proof-only entry needs it — and every later entry reuses
that verdict. Checking it runs the same ordered gauntlet as a per-frame
signature: the scheme (F8), then the key and its validity window
(ADR 0028), then the structural lengths, and only then the one signature
verification. A root that fails is every proof-only frame's state, named once.
`n` proof-only frames cost one verification and `n` proof walks. A unit test
counts the verifications (`a_result_set_root_signature_is_verified_once_per_result_not_once_per_frame`),
so the bound is a checked fact rather than a comment.

The per-frame half lives in the types crate, beside the rule it shares:
`contextgraph_types::frame_inclusion_under_verified_root` checks one frame's
membership of a root whose signature the caller has already verified, under
the same content-binding rule as every other verdict (ADR 0018), and checks no
signature itself. `verify_frame_inclusion` stays the offline, single-frame API
an auditor holding one frame uses.

**2. A proof's shape is checked before any of its hashes.** RFC 6962 fixes the
path for leaf *i* of a tree of *n* leaves: its length and every step's side
follow from index arithmetic over at most ⌈log₂ *n*⌉ split points.
`contextgraph_types::inclusion_path_sides(i, n)` computes that shape, and
`InclusionProof::is_well_shaped` compares a proof against it.
`root_from_proof` now refuses a proof that is not well-shaped, so every caller
— the host, an auditor, the conformance suite — honors `leaf_count`, and it
refuses before the first hash. The split point is computed with bit arithmetic,
not a doubling loop, because `leaf_count` is provider-supplied and a count near
`usize::MAX` must be arithmetic, not an overflow.

**3. At the host, a proof must state the answer's own size.** F12 puts exactly
the frames in `result.frames` under the root, so a proof in an answer as it
arrived that states any other `leaf_count` came from a different tree — a
candidate set the provider truncated, say — and is refused on that comparison,
before anything else about it is examined. The per-frame order is: the root's
verdict, then `leaf_count == frames.len()`, then the 64-step cap, then the
shape, and only then the leaf commitment and the walk.

**4. Every refusal is the existing vocabulary.** A mis-shaped or mis-sized
proof is `AttestationState::Invalid { verdict: MalformedCommitment }` — the
verdict `verify_frame_inclusion` already gave a proof it could not walk — and
F9 holds: the frame is served, included and quoted, and only its attestation
state says what went wrong.

**5. `SPEC.md` §6.5.3 says so.** A verifier **MUST** refuse a proof whose shape
disagrees with its `(leaf_index, leaf_count)`, on that arithmetic and before
hashing; a host checking an answer as it arrived **MUST** refuse a proof whose
`leaf_count` is not the answer's frame count; and a host **SHOULD** verify the
root's signature once per answer.

## Consequences

- `contextgraph-types` gains `inclusion_path_sides`,
  `InclusionProof::is_well_shaped` and `frame_inclusion_under_verified_root`,
  all additive. `root_from_proof` and `verify_frame_inclusion` now refuse a
  mis-shaped proof. Every proof `inclusion_proof` builds is well-shaped (a test
  checks every leaf of every tree up to 40 leaves), so no honest proof, fixture
  or vector changes its verdict; the published inclusion-proof vector is leaf 3
  of 7 and is well-shaped.
- `contextgraph-host`: the per-result check is restructured around one root
  verdict; no public signature changes.
- Witnesses in `contextgraph-host/tests/attestation_composition.rs`:
  `every_frame_proven_under_a_signed_root_is_attested_in_the_audit`,
  `a_proof_from_a_differently_shaped_tree_is_not_attested`, and
  `a_malformed_root_or_proof_leaves_every_frame_served` (F9).
- **The SDK verifiers must follow.** The Python, TypeScript and Go ports of
  `root_from_proof` check `leaf_index < leaf_count` only. Until each adds the
  shape check, it accepts a proof the reference refuses — on adversarial input
  only, since every honest proof is well-shaped — and that divergence is a
  §6.5.3 conformance gap in the port, tracked for each SDK.

## Alternatives considered

**Keep per-frame verification and cache nothing.** Simple, and it makes the
cheapest honest signing shape the most expensive one to check, handing any
provider a way to multiply a host's signature work by the size of its answer.

**Derive the proofs at the host from the full answer and ignore the carried
ones.** A host holding the whole answer can rebuild every proof, which would
make the carried proofs redundant for this check. It would also make the
host's check differ from the one an auditor holding a single frame runs, and
ADR 0014 already explains why proofs are carried: the moment a host keeps a
subset, the siblings are gone. One verification path, the carried one, is the
durable choice.

**A new verdict or state for a mis-shaped proof.** It is a malformed commitment
in every sense a verdict names — the proof cannot yield the root it claims to —
and `MalformedCommitment` is what verifiers already report for a proof they
cannot walk. A new name would grow the audit's vocabulary for no new response.
