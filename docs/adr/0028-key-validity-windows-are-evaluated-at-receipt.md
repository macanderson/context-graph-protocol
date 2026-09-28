# 28. Key validity windows are evaluated at receipt, never at `issued_at`

- Status: accepted
- Date: 2026-09-28
- Resolves #136 (frame layer) and #120 (record layer), settled the same way.
  Builds on [ADR 0016](0016-attestation-trust-roots.md) (the operator is the
  trust root) and [ADR 0021](0021-attestation-metadata-outside-the-signature.md)
  (`issued_at` is outside the signature). Corrects profile `LC3`. Amends ADR
  0016's Consequences.

## Context

A key an operator trusted never lapsed. `contextgraph_host::TrustedKey` held a
`key_id` and a public key and nothing else, so a key trusted in 2026 still
attested frames in 2030, and removal (`TrustStore::revoke`) was the whole
lifecycle — one that needed somebody to notice. The record layer was worse
off: profile `LC3` said "key rotation is by **key-id validity windows**", and
nothing anywhere expressed a window, held one, or consulted one.
`verify_record_attestation` took a raw public key and had no way to ask whether
the key named by `key_id` was in service.

Both issues found the same design question underneath, and it has to be
answered before a line of code is worth anything: **which clock decides?**

1. **The attestation's `issued_at`.** It is what the field seems to be for. It
   is also outside the signed preimage (`SPEC.md` §6.5.2, F18; profile `LC4`
   for records). Anyone who relays an attestation can rewrite it to any
   instant and the signature still verifies. A window checked against it
   therefore stops exactly one party — an honest signer who states the true
   time — and waves through the party a `not_after` exists for: the holder of
   a lapsed or stolen key, who writes an in-window date. The check would be
   worth nothing against the only adversary it is meant for.
2. **The verifier's clock at verification time.** Sound for a live host, and
   wrong for the case #136 names: an auditor replaying old evidence. An
   attestation archived in 2026 under a key retired at the end of 2026 would
   stop verifying the day the key lapsed, although it was produced, received
   and checked while the key was in service.
3. **Bind `issued_at` into the preimage.** The only way to make the signer's
   timestamp trustworthy, and it changes the commitment of every attestation
   ever produced — a new major family (§13 U4). ADR 0021 records that
   `contextgraph/2` should bind `issued_at` only if it also defines what a
   verifier does with it. This ADR is that definition for the one decision it
   needs, and it does not need binding.

## Decision

**1. A window is evaluated at the instant the verifier received the evidence —
its receipt instant — and never at `issued_at`.**

The receipt instant is the one the signer cannot choose, and it is a **sound
upper bound on the signing time**: nothing can be received before it exists.
So "received no later than `not_after`" proves "signed no later than
`not_after`", which is precisely what a key lapse needs to know. Option 2 is
this rule with the receipt instant approximated by "now"; option 1 is this rule
with the receipt instant replaced by a value the adversary writes.

- **A live host** reads its own clock once per answer, as the answer arrives.
  `contextgraph-host` does this in the fan-out and records the instant on
  `ProviderOutcome::received_at`, so the check that produced the audit can be
  reproduced.
- **An auditor replaying archived evidence** passes the receipt instant that
  was recorded when the evidence was first taken in —
  `TrustStore::check_result_signed_as_at`, `check_signed_as_at`, and
  `RecordKeyRing::verify_record(…, received_at)`. Evidence received while its
  key was in service verifies forever; evidence received after the key lapsed
  never did. That is the replay behavior an auditor needs, and it holds however
  many years pass between receipt and replay.
- **What the auditor cannot do** is prove the receipt instant to a third party.
  It is the verifier's own record, trusted exactly as far as the verifier is —
  the same standing as the trust store itself, which ADR 0016 already makes
  local and non-transitive. A deployment that needs signing time provable to
  strangers needs a signed or third-party timestamp (RFC 3161, a transparency
  log); ADR 0021 leaves that to a future major family and this ADR does not
  change that.

`issued_at` is still read by nothing. F18's position — "nothing in
`contextgraph/1` supports reasoning about an attestation's age, freshness, or
expiry" from `issued_at` — stands exactly as written; this decision reasons from
a different instant.

**2. A window is `not_before` and `not_after`, both optional, both inclusive,
both F4 timestamps, compared as instants.** Inclusive on both ends is the
X.509 convention (RFC 5280 §4.1.2.5), so an operator copying dates from a
certificate gets the meaning they expect. Comparison is by instant rather than
by bytes — `contextgraph_types::compare_protocol_timestamps` — because with
fractional seconds allowed, `"…:00.5Z"` sorts before `"…:00Z"` bytewise. A
window with neither bound admits every instant **without reading the clock**,
so a key with no window behaves exactly as a key did before windows existed.

**3. A window that cannot be evaluated fails closed.** A malformed bound, an
inverted window, or a malformed receipt instant against a bounded window is
never "in service". `KeyValidity::new` refuses the first two where a person is
reading the error; a window assembled around it is checked again at every
evaluation.

**4. One definition, in the types crate.** `contextgraph_types::KeyValidity`
and `WindowPosition` are dependency-free and ungated, so the frame layer, the
record layer and every SDK port share one spelling and one evaluation rule.
The window belongs to the verifier's record of a key and never to the wire: no
attestation, handshake or record carries one.

**5. Out of service is its own named outcome.** At the frame layer,
`AttestationState::KeyNotInService { key_id, received_at, position, validity }`
— distinct from `NoTrustedKey` (a configuration gap) and from `Invalid` (a
finding about the signature), on ADR 0010 §7's reasoning for naming verdicts.
The signature is not checked for a key out of service: no key was in service to
check it against, and the window is evaluated after the key lookup and before
any cryptography, so it also bounds work. At the record layer,
`RecordKeyRing` holds `RecordKey { key_id, public_key, validity }` and answers
`RecordKeyVerdict::{UnknownKey, KeyNotInService, Checked}`.

**6. F9 holds.** An out-of-service key degrades its frame to unattested and
never removes it. `KeyNotInService` is `was_offered() == true` and
`is_attested() == false`, and it rides the composition audit like every other
state.

**7. Profile `LC3` says what a window means.** The sentence "key rotation is by
key-id validity windows" now states the evaluation instant and the named
failure, and says that where a verifier keeps its windows is its own affair —
`RecordKeyRing` is the reference, not a requirement.

## Consequences

- `contextgraph-types` gains `KeyValidity`, `KeyValidityError`,
  `WindowPosition`, `compare_protocol_timestamps`, and, for records,
  `RecordKey`, `RecordKeyRing` and `RecordKeyVerdict`. All additive.
- `contextgraph-host`: `TrustedKey` gains `validity` (a Rust break for a struct
  literal; `TrustedKey::ed25519_bytes` / `ed25519_hex` plus `with_validity` are
  the constructors), `AttestationState` gains `KeyNotInService` (a break for an
  exhaustive `match`), `ProviderOutcome` gains `received_at`, and `TrustStore`
  gains the `_at` variants of its two checking methods. The persisted form is
  unchanged for a key with no window: `validity` is omitted when unbounded, so a
  store written before this ADR reads back identically.
- Witnesses: `contextgraph_host::trust`'s
  `a_key_is_attested_while_in_service_and_named_out_of_service_after`,
  `back_dating_issued_at_does_not_bring_a_lapsed_key_back_into_service` and
  `an_auditor_replays_archived_evidence_at_its_recorded_receipt_instant`; the
  integration test
  `a_lapsed_key_degrades_its_frame_to_unattested_and_never_removes_it` (F9);
  and `contextgraph_types::record_attest`'s
  `an_attestation_received_outside_its_keys_window_fails_for_that_reason`.
- No wire change, no schema change, no vector change.
- **What this still does not do.** It does not notice a compromise: a window is
  closed by an operator, as a key is revoked by one. And it does not make a
  receipt instant evidence for anybody but the verifier that recorded it.

## Alternatives considered

**Evaluate at `issued_at` and document that it is unsigned.** Honest, and
useless: the one case it exists for is the one it cannot catch. A check that
only an honest party can fail is a check nobody should rely on, and one that
looks like it works is worse than none.

**Evaluate at "now" only.** Correct for a live host and wrong for every
auditor, which is the audience attestation exists for (ADR 0010). The receipt
instant generalizes it: a live host's receipt instant *is* now.

**Put the window on the wire** — in `handshake_ack.attester_keys` or on the
attestation. It would be the provider stating its own key's lifetime, which is
the party under audit vouching for itself; and a window on the attestation
would be unsigned for the same reason `issued_at` is. The window is a belief the
verifier holds about a key, like the key itself (ADR 0016).

**Exclusive `not_after`, as `valid_to` is for frames (ADR 0022).** A frame's
`[valid_from, valid_to)` is half-open so that consecutive versions tile time
without overlap. Key windows are not tiled — rotation overlaps keys on purpose
— and the operator's source for them is usually a certificate, whose bounds
are inclusive. Matching X.509 is the choice an operator will not trip on.
