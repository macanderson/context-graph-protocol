# 30. A pinned trust tier, strictly below a configured key

- Status: accepted
- Date: 2026-09-28
- Resolves #130. Closes the follow-up [ADR 0016](0016-attestation-trust-roots.md)
  left open and the question [ADR 0019](0019-one-home-for-an-attestation.md)
  §4 deferred. Adds host rules to `SPEC.md` §6.5.5; no new wire field.

## Context

ADR 0016 made the operator the trust root: a key is in
`contextgraph_host::TrustStore` because a person put it there. That is the right
*primary* tier, and it leaves a real gap. A provider a person installed but never
handed a key for serves evidence that reads as `NoTrustedKey` forever, even when
the same key has answered every query for a year. Trust-on-first-use would
record that continuity.

ADR 0016 rejected it "for this revision" on two grounds. The first was a fact
about the wire — no field carried a public key, so there was nothing to pin —
and it is no longer true: `handshake_ack.attester_keys` shipped (#161) and
ADR 0019 §4 kept it as a construction anchor, explicitly leaving pinning open.
The second ground is permanent and this ADR keeps it: **TOFU proves continuity,
never identity.** The key that signs today is the key the provider published
the first time this host met it. Nothing says whose that was, and an attacker
present at first contact is trusted for as long as the pin stands.

So the question is whether a tier that proves only continuity is worth having,
and if so, how to keep it from ever being read as the tier that proves more.

## Decision

**Adopt the pinned tier, opt-in, labelled at every surface, and strictly below
a configured key.**

**1. The wire is unchanged.** `handshake_ack.attester_keys` is the published key
the tier pins. It stays optional: `SPEC.md` §6.5.5 now says a receiver **MUST**
tolerate its absence and treat it as an empty list, the schema says the same,
and `examples/reference-messages.json` carries a `handshake_ack` that publishes
one. The transports keep what the handshake carried, and
`ContextProvider::attester_keys()` exposes it — a defaulted method returning no
keys, so no existing provider changes.

**2. A key records its tier.** `TrustedKey::tier` is `TrustTier::Configured`
(the default, ADR 0016's tier) or `TrustTier::Pinned`. It persists: the trust
file (ADR 0029) carries `"tier": "pinned"`, so a pin survives a restart, which
is what makes a later change noticeable at all. A configured key omits the
member, so a store written before tiers existed is byte-identical.

**3. A pinned verification is its own state, and it is not "attested".** A
signature that verifies against a pinned key reads as
`AttestationState::Pinned { key_id, attester_id, covers_content }` — a distinct
variant, not a flag on `Attested`. `is_attested()` is **false** for it. Every
existing reading of "attested" — `any_attested`, the audit's `attested()`, a
host's green badge — keeps meaning "verified against a key a person checked",
and a caller that wants to credit continuity asks for it by name:
`signature_verified()` is true for both, and `trust_tier()` says which. This is
the same safe-default shape ADR 0018 gave `ValidIdentityOnly`: the narrower
claim is never what a caller gets by accident.

**4. Pinning is an explicit host call, outside the query path.**
`Host::pin_attester_keys(provider_id)` (over `TrustStore::pin` /
`pin_all`) pins the keys the provider published. Nothing pins unless the host
calls it, and no fan-out mutates the store it reads — which answers #130's
concurrency note without interior mutability. The recommended call site is
after registration, and again on every later connect.

**5. Nothing is ever replaced by a pin, and a change is loud.**

| What is held under the published `key_id` | Outcome | Store |
|---|---|---|
| nothing | `Pinned` | the key is pinned |
| the same bytes, pinned | `AlreadyPinned` | unchanged |
| the same bytes, configured | `AlreadyConfigured` | unchanged, still configured |
| **different bytes, pinned** | **`KeyChanged` — alarm** | **unchanged; the pin stands** |
| **different bytes, configured** | **`ConflictsWithConfigured` — alarm** | **unchanged; the operator's key stands** |

Rotation is a new `key_id`, never a reused one (`SPEC.md` §6.5.2), so new bytes
under a pinned id are precisely what pinning exists to notice. Silently
re-pinning would hand the pin to whoever changed the key. `PinOutcome` is
`#[must_use]` and `is_alarm()` names the two a host must put in front of a
person. Replacing a pin is an operator act: revoke it, or trust a configured key
under that id, which also promotes it.

**6. Bounded.** The handshake is provider-controlled, so pinning refuses a key
this host cannot verify with (another algorithm, malformed hex, an empty or
oversized `key_id`) and caps a provider at `MAX_PINNED_KEYS_PER_PROVIDER` (16)
pinned keys; `pin_all` considers at most that many entries per handshake and
reports the rest once. Configured keys do not count against the cap.

**7. Keyed by the operator's id.** A pin lives under the local id the operator
registered the provider under, the same id trust and consent are keyed on, so a
provider cannot reach another provider's pin by declaring its name.

## Consequences

- `SPEC.md` §6.5.5 gains: `attester_keys` **MUST** be tolerated when absent; a
  host **MAY** pin; a host that pins **MUST NOT** present a pinned verification
  as equivalent to a configured one, **MUST NOT** replace a pinned key that
  changed and **MUST** report the change, and **MUST NOT** let a pin override a
  configured key.
- `contextgraph-host` gains `TrustTier`, `TrustedKey::tier` (a struct-literal
  break), `AttestationState::Pinned` (an exhaustive-`match` break),
  `AttestationState::signature_verified` and `trust_tier`, `TrustStore::pin` and
  `pin_all`, `PinOutcome`, `PinRefusal`, `MAX_PINNED_KEYS_PER_PROVIDER`,
  `Host::pin_attester_keys`, and the defaulted `ContextProvider::attester_keys`.
- Witnesses: `contextgraph_host::trust`'s
  `a_changed_key_under_a_pinned_key_id_is_an_alarm_and_is_never_re_pinned`,
  `a_pinned_key_verifies_as_pinned_never_as_attested` and
  `a_pin_never_outranks_the_operator`; and the integration test
  `a_pinned_key_is_its_own_tier_and_a_changed_key_is_reported_not_re_pinned`,
  which runs first contact, a restart, and a changed key through a real host.
- **What this does not prove, stated plainly.** A `Pinned` state says the key is
  the one this host saw first. It does not say who holds it, that the first
  contact was honest, or anything to a second host, which has its own first
  contact. It is worth exactly what "this ssh host key has not changed" is
  worth: a great deal against a later substitution, nothing against an attacker
  who was there first.

## Alternatives considered

**Decline the tier and close the option (the other way #130 allowed).** It would
leave the year-long, same-key provider reading as `NoTrustedKey`, which throws
away a real signal — continuity — that the wire already carries. Declining was
right while there was nothing to pin; with `attester_keys` shipped, the durable
answer is the labelled tier, not a permanently unused field.

**A `tier` field on `Attested` instead of a separate variant.** Smaller, and it
would make every existing `is_attested()` caller count pins as attested by
default — the exact equivalence the issue says the tier must never be presented
as. A separate variant makes the stronger claim the default and the weaker one
an explicit choice.

**Pin automatically on registration.** Convenient, and it would make pinning a
side effect a host did not choose, with alarms returned from a call whose
result is usually `Ok(())`. An explicit call returning every outcome keeps the
alarm where the caller has to look at it.

**Re-pin on change, with a warning.** "Report loudly" is not enough if the new
key is trusted in the same breath. A pin that moves when the key moves pins
nothing.
