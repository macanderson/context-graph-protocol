# 0014 — Attestations on the wire: where they ride, and who builds the proofs

**Status:** Accepted (`contextgraph/1.0`, additive)

## Context

[ADR 0010](./0010-provenance-attestation.md) defined `ProvenanceAttestation` and
the three constructions behind it — the provenance hash chain, the frame
commitment, and the RFC 6962 Merkle root over a result set. It left one thing
undone, and named it in its own consequences: *"attestations carried in the
`frames` envelope and the JSON Schema."*

Until that landed, a provider that signed a frame had nowhere to put the
signature. The feature was reachable only by out-of-band agreement between one
host and one provider, which is the opposite of what a protocol is for: two
implementations that had never met could each be perfectly conformant and still
be unable to exchange a single piece of evidence.

Three questions had to be answered together, because the answer to each
constrains the others.

## Decision

### 1. The attestation rides on the result, not on the envelope

`frame_attestations` and `result_attestation` are members of
`ContextQueryResult`, which is the `result` payload of the `frames` envelope.

The alternative was `Envelope::Frames { id, result, attestations }`. It is a
smaller diff and it is wrong. The envelope is the **transport binding**; the
result is the **semantic layer**, and `SPEC.md` §2 defines the two
independently on purpose. Everything the envelope carries — the `type` tag and
the correlation `id` — is a fact about the connection. Everything about the
*answer* — its frames, whether it was truncated, how much was dropped — lives on
the result.

An attestation is a fact about the answer. Putting it on the envelope would mean
an in-process provider, which returns a `ContextQueryResult` and never
constructs an envelope at all, could not sign what it serves; it would mean the
MCP bridge and the MCP server, which reserialize the result into their own
`structuredContent`, would silently drop the evidence; and it would mean a
future JSON-RPC binding (ADR 0002) would have to redefine the carriage rather
than inherit it.

### 2. An entry names the frame it covers, in full

A `frame_attestations` entry is `{ frame: FrameId, attestation?, inclusion_proof? }`,
where `FrameId` is the whole *(provider id, frame id, `content_digest`)* triple.

The cheaper design is a parallel array indexed against `frames`. It is rejected
for the reason §9's `FrameVerdict` already rejects it: a provider that reorders,
omits, or duplicates an entry would shift one frame's evidence onto another, and
a host that filtered the set — the normal case — would have to reconstruct the
mapping from an order nobody wrote down. Position is not identity, and this is a
protocol whose entire §6.5 argument is that a signature must bind to *one* frame
or to nothing.

It is also the only shape that works: `provider_id` and `content_digest` are two
of the three inputs to the frame commitment, and a verifier cannot recover either
from the frame body. An entry that omitted them would not be checkable offline,
which is the property the whole construction exists to deliver.

Both payload members are optional, because two honest shapes exist and neither
may be unrepresentable. A provider signing a whole answer sends **one** root
signature plus a proof per frame — requiring a per-frame `attestation` would
force it into *n* signatures to say what one says. A provider signing frames
individually publishes no root and sends no proof. An entry with neither is
noise, and a host reads it as *unattested* (F9), which is what F9 already says
about every uncheckable attestation.

### 3. Inclusion proofs are carried, not derived — optional on the wire, mandatory before a host drops a frame

**This is the question the issue asked, and it is not symmetric.**

A host that receives a *complete* signed result set needs no proofs at all: it
holds every frame, so it can recompute every commitment, rebuild the tree, and
derive any proof it later wants. On that reading, shipping proofs is pure
redundancy — roughly `n · log₂(n)` extra 32-byte hashes for something the
receiver can already compute.

But the receiver of a signed answer is not the last party to hold it. A host
composes: it truncates to a budget, dedups across providers, reranks, and keeps a
*subset*. The moment a frame is dropped, its commitment is gone, and with it the
sibling every surviving frame's proof needed. The retained frames are then
attested by a root that nothing — not the host, not an auditor, not the provider
months later — can recompute. Selective disclosure, the single reason §6.5.3
builds a Merkle tree instead of signing a list, is destroyed by ordinary
composition.

So the proof must be materialized while the whole set is in hand. Three ways to
guarantee that, and only one survives:

- **Never inline; the host always derives.** Rejected. It makes evidence
  survival depend on a host knowing to derive proofs *before* it filters — a
  step nothing forces, whose omission is silent, and whose cost is discovered
  months later by an auditor. A protocol should not hand its central guarantee
  to an implementation detail of the receiver.
- **Always inline; mandatory whenever a root is signed.** Rejected. It taxes the
  common case — a host that consumes the whole answer within one turn and stores
  nothing — with `n · log₂(n)` hashes it will never read. Worse, it makes signing
  an answer more expensive in bytes than not signing it by a margin that grows
  with the result size, which is a bad incentive to put in front of the feature
  we want adopted.
- **Optional inline, with a stated host obligation.** Chosen. A provider **MAY**
  carry an `inclusion_proof` per attested frame; a host that retains a strict
  subset of a signed set **MUST** derive and retain the proofs for what it keeps
  before dropping the rest (F13). A proof that *is* present must recompute the
  root, so a wrong one is a red test rather than a plausible-looking string.

The obligation is on the host rather than the provider because the host is the
only party that knows whether it is about to filter. The provider cannot know,
and a rule conditioned on knowledge the actor does not have is unenforceable —
the same reasoning §6.6 uses to refuse mandating score calibration.

What this does **not** settle: nothing mechanically checks a host's F13
obligation today. It is a host-composition requirement, checkable only by a
host-side conformance scenario over a filtering host, and no such scenario
exists yet.

## Alternatives considered

**A separate `attested_frames` envelope type.** Rejected. It would double the
`frames` reply for a signing provider and force every host to correlate two
messages, for no gain: the members are optional, so an unsigned answer already
costs nothing.

**Attaching the attestation to the frame itself.** Already rejected by ADR 0010
and restated as F6; nothing here revisits it. Detachment is what lets a key
rotation, a re-signing, or a countersignature leave a frame's content-addressed
identity untouched.

**Signing the root over the provider's whole candidate set.** Rejected as F12.
A root covering frames the host never received cannot be verified by the host,
by an auditor, or by anyone — and an unverifiable root is worse than no root,
because it looks like evidence.

**A `key` or `keys` member carrying the public key beside the signature.**
Deferred, not rejected. ADR 0010 §6 puts key custody and distribution out of
scope, and inlining a key would invite a verifier to trust the key the signer
supplied — which verifies every forgery. Key distribution is its own decision.

## Consequences

- `ContextQueryResult` gains two optional members, `frame_attestations` and
  `result_attestation`, both omitted when empty. Additive within
  `contextgraph/1`: an unsigned answer serializes to the same bytes as before,
  and a 1.0 peer ignoring the members still parses a signed one (§13 U1).
- `contextgraph_types::attest` gains `FrameAttestation`, plus
  `result_set_commitments` and `result_set_root` behind the `attestation`
  feature, so a provider computes the F12 root the one specified way rather than
  reimplementing the ordering rule.
- `SPEC.md` gains §6.5.5 and requirements F11–F13.
- `schema/contextgraph-envelope.schema.json` gains `ProvenanceAttestation`,
  `InclusionStep`, `InclusionProof`, and `FrameAttestation`.
- `examples/` ships a signed exchange, and
  `contextgraph-conformance/tests/attestation_wire.rs` recomputes every
  commitment and verifies every signature in it — a wire example whose
  signatures nobody checks would teach an implementer to produce forgeries.
- `contextgraph-conformance` takes `contextgraph-types` with the `attestation`
  feature as a **dev**-dependency. The published dependency set is unchanged,
  and `cargo test` now compiles and runs the §6.5 constructions, which nothing
  in CI did before.
- Still open: a host-side conformance scenario for F13, and an adversarial
  `--misbehave` mode that serves a forged attestation (issue #89).
