# 23. A frame identity's provider id is the declared name on the wire and the local id in the host

- Status: accepted
- Date: 2026-09-23
- Issue: #186. Builds on the §6.5.2 rule that the attestation preimage carries
  the declared name, and on [ADR 0016](0016-attestation-trust-roots.md), which
  keys trust on the operator's local id.

## Context

`SPEC.md` §6.3 makes *(provider id, frame id, `content_digest`)* every frame's
stable identity. §9's `verify` carries it, UR1 bills against it, A1 attributes
against it, and D2 says two frames sharing it **MUST** be treated as the same
content. The spec never said which provider id it meant, and a host has two:

- the **local id**, the key the operator configured the provider under
  (`Host::add_stdio(id, …)`). Consent and attestation trust are recorded
  against it, and the provider never sees it.
- the **declared name**, `provider.name` from the handshake. §6.5.2 calls it
  the only identifier both ends of the wire observe and already puts it in the
  signed commitment.

The reference implementation used a different one in each place:

| Surface | id used |
|---|---|
| `FrameId` in composition, dedup, audit, usage reports | local |
| Attestation commitment | declared |
| `verify` request sent to the provider | local |

The third row reached the wire. `Host::verify_frames` grouped held frames by
`FrameId::provider_id` and sent them unchanged, so every `verify` request carried
a string the provider had never seen. It worked only because the reference
provider matched on `frame_id` and ignored `provider_id`. A provider that took
§9's "echoes the identity in full" at its word, and checked the id against its
own name, would have rejected every request the reference host sent.

The same ambiguity had already produced one live defect. The host recomputed
attestation commitments with its local id and reported `CommitmentMismatch`, the
tampering verdict, for honest signatures. That was patched separately. The spec
gap that allowed it was not.

The issue named three resolutions: the declared name everywhere, the local id
everywhere, or the declared name on the wire and the local id inside the host
with a specified mapping.

## Decision

**The third: the triple carries the declared name on the wire and the local id
inside a host, and a host translates at the connection boundary.** This is
requirement D5 in §6.3.

- On the wire (a `verify` request, a `verified` verdict, a `frame_attestations`
  entry, and the §6.5.2 commitment), *provider id* is the declared name.
- Inside a host (composition, dedup, reuse, usage reports, attribution), it is
  the local id.
- When a host sends an identity to a provider, it substitutes that provider's
  declared name. When an identity comes back, the host resolves it to the local
  id of the connection it arrived on, and never by looking the declared name up
  across providers.

§9 gains **V5**. A `verify` request carries the recipient's declared name. A
provider **MAY** answer `unknown` for an identity naming any other provider id,
and **MUST NOT** reject the whole request for it. The answer is per entry
because V2 and V3 already make `unknown` safe, while a whole-request error would
let one misaddressed entry deny revalidation to every other frame in the batch.

The permission is a MAY rather than a MUST NOT answer `valid`. Once the host
translates, a foreign id reaches a provider only from a broken or hostile host.
The permission is what makes the host's rule enforceable, because a conformant
provider may rely on it. Obliging every provider to police it would turn the
existing SDK providers non-conformant for no gain.

`Host::verify_frames` now translates. The reference fixture now uses the V5
permission and answers `unknown` to any identity that does not name its declared
name. The conformance suite registers every provider as `provider-under-test`,
which never equals a declared name, so `verify-honesty` is the witness. A host
that leaked its local id onto the wire gets every frame back `unknown` and
fails. Two host unit tests cover the same path in process, one with a local id
that differs from the declared name and one with two providers declaring the
same name.

## Consequences

- The wire shape of `verify` is unchanged. Only the value in `provider_id`
  changes, to the one §6.5.2 already used, so the attestation preimage and the
  `contextgraph/1` family are untouched.
- Two configured providers may declare the same name. Translation happens per
  connection, so neither can answer for the other's frames, and the internal
  identities differ because the local ids do. A host may warn about the
  duplicate. It does not need to refuse it.
- Identities keyed on local ids are host-scoped. Canonical order is byte-stable
  across hosts only where they configure the same local ids. A host that exports
  identities (a fleet cache, a cross-host usage warehouse) carries its local-id
  to declared-name binding with them. §6.3 says so, and says that matching on
  declared names alone inherits H2's weakness.
- `FrameId::provider_id`'s doc comment and `VerifyRequest`'s now state which id
  applies where.

## Alternatives considered

**The declared name everywhere.** This makes identities portable across hosts
and makes them attacker-controlled. H2 requires only that `provider.name` be
non-empty. A provider declaring `name: "repo-graph"` would mint identities that
collide with the honest `repo-graph`'s, and D2 would then require the host to
treat its frames as the same content, poisoning dedup, reuse, UR1's billing join
and A1's attribution with one string. Making it safe would need a naming
authority or operator-confirmed names, which is a PKI decision ADR 0016
deliberately did not make. That decision can still be taken later on top of
D5, by pinning declared names to trusted attester keys, without changing the
wire.

**The local id everywhere.** This contradicts §6.5.2, whose preimage is frozen
for the family: a provider cannot sign over a string it never sees. It would
also put that string on the wire in `verify`, which is the defect being fixed.

**Leave `Host::verify_frames` alone and rely on providers ignoring the id.**
This is the status quo. It is correct only as long as no provider reads a field
the spec tells it is part of the identity it must echo in full, and it left the
spec with a MUST (D2) keyed on an undefined referent.
