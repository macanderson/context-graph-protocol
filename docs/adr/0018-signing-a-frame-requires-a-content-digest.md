# 18. Signing a frame requires a content digest, and a verifier must say when one is missing

- Status: accepted
- Date: 2026-09-10
- Supersedes nothing. Corrects a claim in [ADR 0010](0010-provenance-attestation.md) §2.

## Context

`frame_commitment` (`SPEC.md` §6.5.2) signs this preimage:

```text
SHA256( "contextgraph/attest/1/frame"
      ‖ enc_str(provider_id) ‖ enc_str(frame.id)
      ‖ enc_opt(frame.content_digest)
      ‖ chain_head )
```

`content_digest` is an `Option`, and D3 permits a conformant frame to declare
none. `enc_opt` writes a single `0x00` presence byte for that case, which is the
correct encoding — absence stays distinguishable from an empty string, and the
hash records it honestly.

The consequence was not written down anywhere a reader would find it. **A
provider could sign a digest-less frame, serve one document under that frame id,
later serve a completely different document under the same id, and the original
signature would still verify** — because the content was never in the preimage.
The reference verifier returned `Valid` for both, so a host had no way to tell
that the second answer was not the one that had been signed.

ADR 0010 §2 asserted the opposite in prose: "Including `content_digest` further
means the signature covers the frame's *bytes*, not merely its name." That is
true only when the field is present, and the ADR said it without qualification.
The doc comment on `frame_commitment` named the gap honestly; nothing else did.

Found by writing a host-side test that expected altering `frame.content` to
produce a `CommitmentMismatch`. It produced `Valid`.

This matters more than a documentation error because a signature is the one
thing in this protocol that is supposed to survive distrust of the host storing
it. A digest is tamper-evident only to a party that already trusts whoever wrote
it down; the whole reason §6.5 exists is to answer the auditor who trusts
nobody. An attestation that verifies while covering nothing the frame *says* is
the failure mode that machinery was built to rule out.

## Decision

**An attester MUST populate `content_digest` on any frame it signs, and a
verifier MUST distinguish an attestation that binds content from one that does
not.** Both, not either.

Three options were on the table (#128):

1. **Say so normatively and change nothing else.** Cheapest; changes no bytes.
2. **Require an attester to refuse.** Tightens a conformance rule rather than
   the wire, so it is additive within `contextgraph/1` — but it makes a
   previously conformant signing provider non-conformant.
3. **Put the content itself in the preimage.** Wire-breaking; needs
   `contextgraph/2`.

Option 3 is rejected: it buys nothing option 2 does not, at the cost of a major
protocol version. `content_digest` already *is* the frame's content in hashed
form, so binding the digest binds the bytes.

Option 1 alone is rejected because documenting a footgun is not removing it. The
hazard is that a signature reads as a stronger claim than it makes, and a
paragraph in a specification does not reach the host rendering a checkmark.

**Options 1 and 2 together are the durable answer**, and the two halves do
different jobs:

- Option 2 stops new digest-less signatures being produced. It is the fix.
- Option 1 governs how the ones already in existence are read. Without it, every
  attestation signed before this rule becomes ambiguous rather than merely
  narrow.

The blast radius of option 2 is small enough to take now and would not be later.
The crates serve `0.1.2`, two of the SDK packages have never been published, and
no signing provider is known outside this repository. This is the cheapest this
decision will ever be.

## Consequences

**`AttestationVerdict` gains `ValidIdentityOnly`.** The signature verified, over
a preimage that does not bind content. `is_valid()` is **false** for it, keeping
faith with that method's existing doctrine — "I could not check it" and "it is
good" are never the same answer, and a host asking "is this good?" is asking
about the bytes. `signature_verifies()` answers the narrower question for a
caller who genuinely wants provider identity without a claim about content, and
`binds_content()` answers the wider one.

Rust's exhaustive matching means every existing consumer is asked the question
by the compiler rather than left to notice it. That is most of why the
distinction is a variant rather than a boolean beside the verdict.

**An identity-only attestation is attested, not invalid.** The host maps it to
`AttestationState::Attested { covers_content: false }`, which is the state that
field already existed to express. Demoting it to `Invalid` would discard a
signature that genuinely checks out, and would punish the consumer for the
attester's mistake.

`covers_content` is now read off the verdict instead of being re-derived from
`frame.content_digest.is_some()`. The two agreed when this was written; a single
source of truth is what keeps them agreeing.

**The conformance suite reports it as a problem, not a degradation.** A provider
offering such an attestation is publishing a signature that outlives the content
it appears to cover. The suite is the only place a provider author learns that
before a consumer does.

**Previously conformant signing providers become non-conformant** if they sign
digest-less frames. This is intended, and it is the cost of option 2. A provider
already computing a `content_digest` — which most do, since frame identity uses
it — needs no change at all.

**Verification remains possible for older attestations.** `frame_commitment`
still computes a commitment for a digest-less frame. A verifier has to be able
to check signatures produced before this rule; refusing to compute them would
turn old evidence unreadable rather than correctly-labelled.

**What is still true and still uncomfortable.** A frame that declares a
`content_digest` binds *that digest*, not the bytes a host happens to hold. A
provider that lies about its own digest at signing time signs a consistent lie.
Attestation proves who said it and that it has not changed since; it has never
proved the claim is true, and ADR 0016's Consequences already say so.

## Verification

`contextgraph-types/src/attest.rs`, module `content_binding_tests`:

- `a_signed_frame_with_no_content_digest_is_attested_over_nothing_it_says` is
  the demonstration from #128, kept as a regression test. It signs a digest-less
  frame, rewrites the content, and asserts the verdict is `ValidIdentityOnly`
  both times — not `Valid`. Before this change it asserted `Valid` twice, for
  two different sets of bytes.
- `a_frame_that_declares_a_digest_is_bound_to_it` is the contrasting case:
  altering a declared digest produces `CommitmentMismatch`.
- `stripping_a_digest_after_signing_is_a_mismatch_not_a_downgrade` closes the
  obvious attack on the new path — removing a digest from a frame that was
  signed with one must not launder a tampered frame into a passing verdict.
