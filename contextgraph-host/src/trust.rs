//! Trust roots for provenance attestation, and the host-side verifier that
//! consumes them (`SPEC.md` §6.5, F8–F9;
//! [ADR 0016](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0016-attestation-trust-roots.md)).
//!
//! [ADR 0010](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0010-provenance-attestation.md)
//! specifies the bytes a provider signs and deliberately stops there, so a
//! provider holding keys in an HSM signs a public 32-byte commitment with its
//! own backend. That leaves the question on the host's side of the wire: given
//! a [`ProvenanceAttestation`] and a frame, **where does the public key come
//! from?**
//!
//! The answer here is the only one that needs no organization behind it: **the
//! operator is the trust root.** A [`TrustStore`] maps a `provider_id` to the
//! keys that provider may sign under, and a key is in it because a person put
//! it there — from the same material, in the same act, as the provider's own
//! configuration and its consent grant. This is how `ssh` learns a host key and
//! how `minisign` learns a signer. A registry, a well-known endpoint or a
//! transparency log would all work better and all require a party both sides
//! already trust; `GOVERNANCE.md`'s consent boundary rules that out for a host,
//! and the attestation stays portable enough for one to be built *over* this.
//!
//! # F9 is the load-bearing rule
//!
//! An attestation this host cannot verify degrades its frame to *unattested*.
//! It never disqualifies it. A host that dropped such frames would hand any
//! peer a denial-of-service primitive — attach a malformed attestation, watch
//! the evidence vanish — so every path in this module ends in an
//! [`AttestationState`] and none of them ends in a dropped frame. Verification
//! adds a fact to the audit; it never subtracts evidence and never reranks.
//!
//! # Attacker-controlled work is bounded before any cryptography runs
//!
//! Every field of an attestation arrives from the provider, so this module
//! checks the cheap structural facts first and only then hashes or verifies:
//! an oversized signature is rejected on its length rather than hex-decoded, an
//! attestation naming an unknown `key_id` never reaches the signature check at
//! all, and at most one attestation is verified per frame. The frame count is
//! already bounded by the `max_frames` audit that runs before this
//! ([`Host::query_all`](crate::Host::query_all)), so the total work is linear in
//! a quantity the host already agreed to accept.

use std::collections::{BTreeMap, HashMap};

use contextgraph_types::{
    ALGORITHM_ED25519, AttestationVerdict, ContextFrame, ContextQueryResult, FrameAttestation,
    FrameId, InclusionProof, ProvenanceAttestation, verify_frame_attestation,
    verify_frame_inclusion,
};
use serde::{Deserialize, Serialize};

/// The exact length of a `sha256:<64 lowercase hex>` commitment string.
const COMMITMENT_LEN: usize = "sha256:".len() + 64;

/// The exact length of a hex-encoded Ed25519 signature (64 bytes).
const ED25519_SIGNATURE_HEX_LEN: usize = 128;

/// The exact length of a hex-encoded Ed25519 public key (32 bytes).
const ED25519_PUBLIC_KEY_HEX_LEN: usize = 64;

/// How much of a provider-supplied identifier is copied into an audit record.
/// A `key_id` or an `algorithm` is echoed back so an operator can act on it, and
/// an attacker must not be able to make the audit grow without bound by sending
/// a megabyte one.
const MAX_ECHOED_IDENTIFIER: usize = 128;

/// An Ed25519 public key a host trusts for one provider.
///
/// `public_key` is lowercase hex rather than raw bytes for the reason
/// [`ProvenanceAttestation::signature`] is: one encoding across the whole
/// surface, and a key an operator can paste out of a provider's README into a
/// config file without a base64 detour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKey {
    /// The `key_id` an attestation must name to be checked against this key.
    /// Rotation is a new `key_id`, never a reused one (`SPEC.md` §6.5), so a
    /// store may hold several keys for one provider at once.
    pub key_id: String,
    /// The raw public key, lowercase hex.
    pub public_key: String,
}

impl TrustedKey {
    /// A trusted Ed25519 key from a `key_id` and a 64-character lowercase-hex
    /// public key.
    ///
    /// Returns `None` for anything that is not a well-formed Ed25519 public
    /// key encoding, so a typo in a config file fails where a person is reading
    /// the error rather than months later as an unexplained `MalformedKey` in
    /// an audit.
    pub fn ed25519_hex(key_id: impl Into<String>, public_key: impl Into<String>) -> Option<Self> {
        let public_key = public_key.into();
        if public_key.len() != ED25519_PUBLIC_KEY_HEX_LEN || decode_hex(&public_key).is_none() {
            return None;
        }
        Some(Self {
            key_id: key_id.into(),
            public_key,
        })
    }

    /// A trusted Ed25519 key from raw public-key bytes — the form
    /// [`contextgraph_types::public_key_for`] returns.
    pub fn ed25519_bytes(key_id: impl Into<String>, public_key: &[u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            public_key: encode_hex(public_key),
        }
    }

    /// A `sha256:<hex>` fingerprint over the key bytes — the short string a host
    /// shows a person next to the consent prompt, so "I consent to this
    /// provider" and "I trust this key" are one decision (ADR 0016 §2).
    ///
    /// Over the *decoded* bytes, so two spellings of the same key cannot
    /// fingerprint differently. A key whose hex does not decode has no
    /// fingerprint to show.
    pub fn fingerprint(&self) -> Option<String> {
        let bytes = decode_hex(&self.public_key)?;
        Some(contextgraph_types::digest_string(&sha256(&bytes)))
    }
}

/// The keys a host trusts, per provider — the local answer to "who may sign
/// evidence I will treat as attested?" (ADR 0016).
///
/// Serde-able and persistable, mirroring
/// [`ConsentStore`](crate::consent::ConsentStore), because it is the same kind
/// of object: a record of a decision one person made about one provider on one
/// machine. Nothing populates it implicitly — there is no discovery, no
/// fetching, and no trust-on-first-use. An empty store is a host that verifies
/// nothing and loses nothing, which is the default posture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustStore {
    /// `provider_id -> key_id -> key`. `BTreeMap` inside so iteration over one
    /// provider's keys is deterministic in an audit or a rendered report.
    #[serde(default)]
    keys: HashMap<String, BTreeMap<String, TrustedKey>>,
}

impl TrustStore {
    /// An empty store: no provider has a trusted key, so every attestation
    /// resolves to [`AttestationState::NoTrustedKey`] and every frame is still
    /// served (F9).
    pub fn new() -> Self {
        Self::default()
    }

    /// Trust `key` for `provider_id`, replacing any key already held under the
    /// same `key_id`.
    pub fn trust(&mut self, provider_id: impl Into<String>, key: TrustedKey) {
        self.keys
            .entry(provider_id.into())
            .or_default()
            .insert(key.key_id.clone(), key);
    }

    /// Stop trusting one key. Returns whether a key was actually removed.
    ///
    /// This is the whole of revocation, and it is local: nothing here learns
    /// that a key was compromised, so a host that is told so out of band calls
    /// this, and a host that is never told keeps trusting it (ADR 0016).
    pub fn revoke(&mut self, provider_id: &str, key_id: &str) -> bool {
        let Some(keys) = self.keys.get_mut(provider_id) else {
            return false;
        };
        let removed = keys.remove(key_id).is_some();
        if keys.is_empty() {
            self.keys.remove(provider_id);
        }
        removed
    }

    /// The key held for `(provider_id, key_id)`, if any.
    pub fn key(&self, provider_id: &str, key_id: &str) -> Option<&TrustedKey> {
        self.keys.get(provider_id)?.get(key_id)
    }

    /// Every key trusted for one provider, in `key_id` order.
    pub fn keys_for(&self, provider_id: &str) -> impl Iterator<Item = &TrustedKey> {
        self.keys
            .get(provider_id)
            .into_iter()
            .flat_map(|k| k.values())
    }

    /// Whether this store trusts no key at all.
    pub fn is_empty(&self) -> bool {
        self.keys.values().all(|keys| keys.is_empty())
    }

    /// Check one attestation against this store and report what was found
    /// (`SPEC.md` §6.5.4).
    ///
    /// Total: every input produces a state, and none of them is an error a
    /// caller could mistake for a reason to drop the frame (F9). The cheap
    /// structural checks run first so a hostile attestation cannot buy more
    /// than a constant amount of work before it is dismissed.
    pub fn check(
        &self,
        provider_id: &str,
        frame: &ContextFrame,
        attestation: &ProvenanceAttestation,
    ) -> AttestationState {
        // F8, first: a scheme this build cannot check is *uncheckable*, which is
        // a different finding from invalid and is not improved by holding a key.
        if attestation.algorithm != ALGORITHM_ED25519 {
            return AttestationState::UnknownAlgorithm {
                algorithm: echoed(&attestation.algorithm),
            };
        }

        // No key ⇒ no signature check. This is also the bound that keeps an
        // unknown peer from spending the host's CPU: reaching the verifier at
        // all requires an operator to have trusted a key under this exact id.
        let Some(key) = self.key(provider_id, &attestation.key_id) else {
            return AttestationState::NoTrustedKey {
                key_id: echoed(&attestation.key_id),
            };
        };

        // Structural length checks before any decoding. `verify_commitment`
        // would reach the same verdicts, but only after hex-decoding a string
        // whose length the provider chose.
        if attestation.signed_commitment.len() != COMMITMENT_LEN {
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedCommitment,
            };
        }
        if attestation.signature.len() != ED25519_SIGNATURE_HEX_LEN {
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedSignature,
            };
        }
        let Some(public_key) = decode_hex(&key.public_key) else {
            // A key this host stored that does not decode: an operator
            // configuration bug, reported as the verdict it is rather than as a
            // finding about the provider.
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedKey,
            };
        };

        // Both verifying verdicts are *attested* — an identity-only attestation
        // is a narrower guarantee, not a failed one, and demoting it to
        // `Invalid` would discard a signature that genuinely checks out.
        //
        // `covers_content` is read off the verdict rather than re-derived from
        // `frame.content_digest.is_some()`. The two agreed when this was
        // written, and a single source of truth is what keeps them agreeing:
        // the rule for what a commitment binds lives in `frame_commitment`, and
        // this is downstream of it (#128).
        match verify_frame_attestation(provider_id, frame, attestation, &public_key) {
            verdict @ (AttestationVerdict::Valid | AttestationVerdict::ValidIdentityOnly) => {
                AttestationState::Attested {
                    key_id: attestation.key_id.clone(),
                    attester_id: echoed(&attestation.attester_id),
                    covers_content: verdict.binds_content(),
                }
            }
            verdict => AttestationState::Invalid { verdict },
        }
    }

    /// Check the evidence a provider attached to one query result, and return
    /// one outcome per frame in it — including the frames no entry covered,
    /// which are [`AttestationState::Unattested`].
    ///
    /// The evidence is read off `result` itself
    /// ([`frame_attestations`](ContextQueryResult::frame_attestations) and
    /// [`result_attestation`](ContextQueryResult::result_attestation)), not
    /// passed alongside it. That is deliberate and it is the whole of #161: an
    /// attestation has exactly one home (`SPEC.md` §6.5.5, ADR 0014), so a
    /// caller cannot hand this method a set of signatures that disagrees with
    /// the answer they cover, and no tie-breaking rule is needed because there
    /// is never a tie.
    ///
    /// The result is a **total** account of the frames: a caller can read a
    /// state for every frame it is about to compose, and never has to guess
    /// whether an absent entry means unsigned or unchecked.
    ///
    /// At most one entry is checked per frame, and at most
    /// `result.frames.len()` entries are examined at all. A conforming provider
    /// sends no more than one entry per frame, so the cap binds only a provider
    /// that already over-sent — and the consequence falls on that provider
    /// alone: its own later entries read as absent, and its frames are still
    /// served.
    pub fn check_result(
        &self,
        provider_id: &str,
        result: &ContextQueryResult,
    ) -> Vec<FrameAttestationOutcome> {
        let mut offered: HashMap<&FrameId, &FrameAttestation> = HashMap::new();
        for entry in result.frame_attestations.iter().take(result.frames.len()) {
            // First entry wins: a flood of duplicates for one frame cannot
            // multiply the verification work.
            offered.entry(&entry.frame).or_insert(entry);
        }

        result
            .frames
            .iter()
            .map(|frame| {
                // Match on the whole identity triple, never on the frame id
                // alone: two frames sharing an id but not a digest are
                // different bytes, and handing the first one's evidence to the
                // second is the substitution the binding exists to prevent
                // (`SPEC.md` §6.5.2).
                let identity = frame.identity(provider_id);
                let state = match offered.get(&identity) {
                    Some(entry) => self.check_entry(
                        provider_id,
                        frame,
                        entry,
                        result.result_attestation.as_ref(),
                    ),
                    None => AttestationState::Unattested,
                };
                FrameAttestationOutcome {
                    frame: identity,
                    state,
                }
            })
            .collect()
    }

    /// One entry's outcome: the per-frame signature if it carries one, its
    /// membership of the signed result-set root if it does not.
    ///
    /// A [`FrameAttestation`] carries **either** shape, and the reason both
    /// exist is cost: signing one Merkle root with a per-frame inclusion proof
    /// says with one signature what *n* per-frame signatures say. A host that
    /// checked only the first shape would report every provider that chose the
    /// cheap one as unattested.
    ///
    /// The per-frame signature wins when an entry carries both. It is the
    /// narrower claim — it binds this frame directly, with no tree in between —
    /// and checking it costs one verification rather than a walk plus one.
    fn check_entry(
        &self,
        provider_id: &str,
        frame: &ContextFrame,
        entry: &FrameAttestation,
        result_attestation: Option<&ProvenanceAttestation>,
    ) -> AttestationState {
        if let Some(attestation) = &entry.attestation {
            return self.check(provider_id, frame, attestation);
        }
        match (&entry.inclusion_proof, result_attestation) {
            (Some(proof), Some(root)) => self.check_inclusion(provider_id, frame, proof, root),
            // A proof of membership of a root the answer never carried, or an
            // entry naming a frame and asserting nothing about it. Neither can
            // be turned into a check, and F9 makes that a degradation to
            // unattested rather than a reason to withhold the frame.
            _ => AttestationState::UnusableEvidence,
        }
    }

    /// Check one frame's membership of the signed result-set root
    /// (`SPEC.md` §6.5.3, F13).
    ///
    /// Ordered exactly as [`check`](Self::check) is, and for the same reason:
    /// the scheme, then the key, then the structural lengths, and only then any
    /// hashing. The key lookup in particular is the bound that keeps an
    /// untrusted peer from spending this host's CPU walking a Merkle path —
    /// reaching the walk at all requires an operator to have trusted a key
    /// under the root attestation's exact `key_id`.
    fn check_inclusion(
        &self,
        provider_id: &str,
        frame: &ContextFrame,
        proof: &InclusionProof,
        root: &ProvenanceAttestation,
    ) -> AttestationState {
        if root.algorithm != ALGORITHM_ED25519 {
            return AttestationState::UnknownAlgorithm {
                algorithm: echoed(&root.algorithm),
            };
        }
        let Some(key) = self.key(provider_id, &root.key_id) else {
            return AttestationState::NoTrustedKey {
                key_id: echoed(&root.key_id),
            };
        };
        if root.signed_commitment.len() != COMMITMENT_LEN {
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedCommitment,
            };
        }
        if root.signature.len() != ED25519_SIGNATURE_HEX_LEN {
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedSignature,
            };
        }
        let Some(public_key) = decode_hex(&key.public_key) else {
            return AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedKey,
            };
        };

        // `covers_content` is read off the verdict rather than re-derived, for
        // the reason `check` gives: the rule for what a commitment binds lives
        // in `frame_commitment`, and both call sites stay downstream of it
        // (#128).
        match verify_frame_inclusion(provider_id, frame, proof, root, &public_key) {
            verdict @ (AttestationVerdict::Valid | AttestationVerdict::ValidIdentityOnly) => {
                AttestationState::Attested {
                    key_id: root.key_id.clone(),
                    attester_id: echoed(&root.attester_id),
                    covers_content: verdict.binds_content(),
                }
            }
            verdict => AttestationState::Invalid { verdict },
        }
    }
}

/// What a host found when it checked one frame's attestation (ADR 0016 §4).
///
/// Named outcomes rather than a boolean, for the reason
/// [`AttestationVerdict`] is named: [`NoTrustedKey`](Self::NoTrustedKey) is a
/// configuration gap an operator closes in a minute, and
/// [`Invalid`](Self::Invalid) carrying
/// [`CommitmentMismatch`](AttestationVerdict::CommitmentMismatch) is an
/// incident. Collapsing them sends someone hunting the wrong one.
///
/// **None of these states removes a frame from a composition.** F9 makes an
/// unverifiable attestation a degradation to *unattested*, never a
/// disqualification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AttestationState {
    /// No attestation check was performed on this frame — the host composed it
    /// without consulting a trust store. Distinct from
    /// [`Unattested`](Self::Unattested): "I did not look" is not "there was
    /// nothing to find".
    #[default]
    NotChecked,
    /// The provider offered no attestation for this frame.
    Unattested,
    /// The signature verified against a key this host trusts for this provider.
    ///
    /// This means exactly "signed by a key this operator chose to trust". It
    /// does not mean the content is true, and it carries no weight for a
    /// second host that holds no key (ADR 0016 Consequences).
    Attested {
        /// The key that verified it.
        key_id: String,
        /// The attesting authority the attestation names — who is accountable
        /// for the claim, as distinct from the key that produced it.
        attester_id: String,
        /// Whether the signature covers the frame's **content bytes**.
        ///
        /// A frame commitment is over `(provider_id, frame_id, content_digest)`
        /// plus the provenance chain head (`SPEC.md` §6.5.2), and
        /// `content_digest` is optional. So a frame that declares none has a
        /// perfectly valid signature over its identity and its provenance and
        /// **nothing at all over its text**: the same provider can re-serve
        /// different content under the same frame id and this signature still
        /// verifies.
        ///
        /// `false` says so out loud, so a host does not render such a frame as
        /// though its words were signed. It is not a failure — the frame is
        /// attested — it is a narrower claim than a reader would otherwise
        /// assume, and assuming it is the mistake this field exists to prevent.
        covers_content: bool,
    },
    /// An attestation was offered, but this host holds no trusted key under
    /// that `key_id` for that provider. A configuration gap, **not** a forgery
    /// finding: the signature was never checked, so nothing is known about it.
    NoTrustedKey {
        /// The `key_id` the attestation named, so an operator knows what to add.
        key_id: String,
    },
    /// The attestation names a signature scheme this build cannot check
    /// (`SPEC.md` F8). A refusal to guess, not a failure to validate.
    UnknownAlgorithm {
        /// The scheme the attestation named.
        algorithm: String,
    },
    /// An entry named this frame and this host could not turn it into a check:
    /// an inclusion proof with no signed `result_attestation` root to prove
    /// membership *of*, or an entry carrying neither a signature nor a proof.
    ///
    /// F9: the frame is served, and every decision treats this as unattested.
    /// The state is named rather than folded into
    /// [`Unattested`](Self::Unattested) because the two say different things
    /// about the provider — one chose not to sign, the other sent evidence that
    /// does not resolve — and only the second is worth an operator's attention.
    /// [`was_offered`](Self::was_offered) is true here.
    UnusableEvidence,
    /// A trusted key was found and the check did not succeed — a forgery, a
    /// frame altered after signing, or an attestation too malformed to check.
    /// The verdict says which.
    Invalid {
        /// The named finding from `contextgraph_types::attest`.
        verdict: AttestationVerdict,
    },
}

impl AttestationState {
    /// Whether this frame is attested by a key this host trusts. Every other
    /// state — including [`NotChecked`](Self::NotChecked) — is `false`, because
    /// "I could not check it" is never "it is good" (`SPEC.md` F8).
    pub fn is_attested(&self) -> bool {
        matches!(self, Self::Attested { .. })
    }

    /// Whether the signature covers the frame's content bytes as well as its
    /// identity and provenance — see
    /// [`Attested::covers_content`](Self::Attested). `false` for every state
    /// that is not [`Attested`](Self::Attested).
    pub fn covers_content(&self) -> bool {
        matches!(
            self,
            Self::Attested {
                covers_content: true,
                ..
            }
        )
    }

    /// Whether an attestation was offered at all. A host reports on
    /// [`NoTrustedKey`](Self::NoTrustedKey) differently from
    /// [`Unattested`](Self::Unattested): the first is the host's gap, the
    /// second is the provider's choice.
    pub fn was_offered(&self) -> bool {
        !matches!(self, Self::NotChecked | Self::Unattested)
    }
}

/// One frame's attestation outcome, keyed by the frame's stable identity so it
/// joins the composition audit without a positional assumption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAttestationOutcome {
    /// The frame's stable identity.
    pub frame: FrameId,
    /// What the host found.
    pub state: AttestationState,
}

/// Every frame's attestation state from one fan-out, keyed by identity — the
/// join the composer reads so an [`AuditEntry`](crate::compose::AuditEntry) can
/// say whether the evidence it quotes was signed.
///
/// An **empty** ledger is not "nothing was attested": it is "nothing was
/// checked", and a lookup returns [`AttestationState::NotChecked`] to say so.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttestationLedger {
    states: BTreeMap<FrameId, AttestationState>,
}

impl AttestationLedger {
    /// An empty ledger: every lookup is [`AttestationState::NotChecked`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one frame's outcome, replacing any previous entry for the same
    /// identity.
    pub fn record(&mut self, outcome: FrameAttestationOutcome) {
        self.states.insert(outcome.frame, outcome.state);
    }

    /// The state recorded for a frame, or [`AttestationState::NotChecked`] when
    /// this ledger has nothing to say about it.
    pub fn state_for(&self, frame: &FrameId) -> AttestationState {
        self.states
            .get(frame)
            .cloned()
            .unwrap_or(AttestationState::NotChecked)
    }

    /// Whether this ledger recorded nothing.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// How many frames this ledger has a state for.
    pub fn len(&self) -> usize {
        self.states.len()
    }
}

impl FromIterator<FrameAttestationOutcome> for AttestationLedger {
    fn from_iter<I: IntoIterator<Item = FrameAttestationOutcome>>(outcomes: I) -> Self {
        let mut ledger = Self::new();
        for outcome in outcomes {
            ledger.record(outcome);
        }
        ledger
    }
}

/// The first `MAX_ECHOED_IDENTIFIER` bytes of a provider-supplied identifier,
/// cut at a UTF-8 boundary. An audit record echoes these so an operator can act
/// on them; the cap is what stops a hostile provider growing the record without
/// bound.
fn echoed(value: &str) -> String {
    if value.len() <= MAX_ECHOED_IDENTIFIER {
        return value.to_string();
    }
    let mut end = MAX_ECHOED_IDENTIFIER;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// Decode a lowercase-or-uppercase hex string into bytes. `None` for an odd
/// length or a non-hex byte.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    // Indexed rather than `chunks_exact(2)`: clippy 1.98 wants `as_chunks::<2>`
    // for a constant chunk size, and that API is newer than this workspace's
    // MSRV. `get` keeps the walk panic-free without either.
    while index < bytes.len() {
        let hi = (*bytes.get(index)? as char).to_digit(16)?;
        let lo = (*bytes.get(index + 1)? as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        index += 2;
    }
    Some(out)
}

/// Lowercase hex for raw bytes.
fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble is < 16"));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble is < 16"));
    }
    out
}

/// SHA-256 over `bytes`, for [`TrustedKey::fingerprint`].
fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::{
        FrameKind, Provenance, public_key_for, sign_commitment, sign_frame_attestation,
    };

    /// A deterministic seed. Tests need reproducible signatures, and this key
    /// signs nothing outside this file.
    const SEED: [u8; 32] = [3u8; 32];
    /// A second seed, for "signed by someone else" cases.
    const OTHER_SEED: [u8; 32] = [9u8; 32];

    const PROVIDER: &str = "docs";
    const KEY_ID: &str = "docs-2026-08";

    /// A frame that declares a `content_digest`, so its commitment binds its
    /// bytes as well as its identity (`SPEC.md` §6.5.2).
    fn frame(id: &str) -> ContextFrame {
        let mut frame = digestless_frame(id);
        frame.content_digest = Some(format!("sha256:{}", "cd".repeat(32)));
        frame
    }

    /// A frame with **no** `content_digest` — permitted by the protocol, and
    /// the case whose signature covers no content.
    fn digestless_frame(id: &str) -> ContextFrame {
        let mut frame = ContextFrame::full(id, FrameKind::Doc, "Title", "content", 0.9, 4);
        frame.provenance = vec![Provenance {
            kind: "file".into(),
            uri: Some("file:///repo/README.md".into()),
            range: Some("L1-4".into()),
            digest: Some(format!("sha256:{}", "ab".repeat(32))),
            method: None,
            by: None,
        }];
        frame
    }

    fn signed(frame: &ContextFrame, seed: &[u8; 32]) -> ProvenanceAttestation {
        sign_frame_attestation(
            PROVIDER,
            frame,
            seed,
            KEY_ID,
            "docs-provider",
            "2026-08-29T00:00:00Z",
        )
    }

    fn store_trusting(seed: &[u8; 32]) -> TrustStore {
        let mut store = TrustStore::new();
        store.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(seed)),
        );
        store
    }

    #[test]
    fn a_signature_from_a_trusted_key_is_attested() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let state = store_trusting(&SEED).check(PROVIDER, &frame, &attestation);
        assert_eq!(
            state,
            AttestationState::Attested {
                key_id: KEY_ID.to_string(),
                attester_id: "docs-provider".to_string(),
                covers_content: true,
            }
        );
        assert!(state.is_attested());
        assert!(state.covers_content());
    }

    #[test]
    fn a_signed_frame_with_no_content_digest_is_attested_over_nothing_it_says() {
        // The honest reading of §6.5.2: the commitment covers
        // `(provider_id, frame_id, content_digest)` and the provenance chain,
        // and `content_digest` is optional. Rewriting the *content* of a frame
        // that declares none leaves the signature verifying, because the bytes
        // were never in the preimage. The state says so rather than letting a
        // reader assume the words were signed.
        let original = digestless_frame("frm_1");
        let attestation = signed(&original, &SEED);
        let store = store_trusting(&SEED);

        let state = store.check(PROVIDER, &original, &attestation);
        assert!(state.is_attested());
        assert!(
            !state.covers_content(),
            "a frame with no content_digest has no signed bytes"
        );

        let mut rewritten = original.clone();
        rewritten.content = Some("words the provider never signed".to_string());
        assert_eq!(
            store.check(PROVIDER, &rewritten, &attestation),
            state,
            "the same signature still verifies over the rewritten content"
        );
    }

    #[test]
    fn an_unknown_key_id_is_a_configuration_gap_not_a_forgery_finding() {
        // The store holds the *right* key under a *different* id. Nothing about
        // the signature is known, and the state must not imply otherwise.
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let mut store = TrustStore::new();
        store.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes("some-other-key", &public_key_for(&SEED)),
        );
        assert_eq!(
            store.check(PROVIDER, &frame, &attestation),
            AttestationState::NoTrustedKey {
                key_id: KEY_ID.to_string()
            }
        );
    }

    #[test]
    fn a_key_trusted_for_another_provider_does_not_carry_over() {
        // Trust is keyed by (provider_id, key_id): the same key trusted for a
        // sibling provider must not attest this one's frames.
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let mut store = TrustStore::new();
        store.trust(
            "some-other-provider",
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)),
        );
        assert_eq!(
            store.check(PROVIDER, &frame, &attestation),
            AttestationState::NoTrustedKey {
                key_id: KEY_ID.to_string()
            }
        );
    }

    #[test]
    fn a_signature_from_the_wrong_key_is_a_bad_signature() {
        // Signed by OTHER_SEED, checked against SEED under the same key_id —
        // the shape of a forgery or a swapped provider.
        let frame = frame("frm_1");
        let attestation = signed(&frame, &OTHER_SEED);
        assert_eq!(
            store_trusting(&SEED).check(PROVIDER, &frame, &attestation),
            AttestationState::Invalid {
                verdict: AttestationVerdict::BadSignature
            }
        );
    }

    #[test]
    fn a_frame_altered_after_signing_is_a_commitment_mismatch() {
        // Truncating the provenance chain — dropping the link that would reveal
        // a frame was summarized rather than quoted — is the attack the chain
        // construction exists to catch (ADR 0010 §1).
        let original = frame("frm_1");
        let attestation = signed(&original, &SEED);
        let mut altered = original.clone();
        altered.provenance.clear();
        match store_trusting(&SEED).check(PROVIDER, &altered, &attestation) {
            AttestationState::Invalid {
                verdict: AttestationVerdict::CommitmentMismatch { expected, signed },
            } => {
                assert_ne!(expected, signed, "the two commitments must differ");
                assert_eq!(signed, attestation.signed_commitment);
            }
            other => panic!("expected a CommitmentMismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_signature_is_named_malformed_not_forged() {
        let frame = frame("frm_1");
        let mut attestation = signed(&frame, &SEED);
        attestation.signature = "not hex, and not 128 characters either".into();
        assert_eq!(
            store_trusting(&SEED).check(PROVIDER, &frame, &attestation),
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedSignature
            }
        );
    }

    #[test]
    fn a_malformed_commitment_is_named_before_the_signature_is_touched() {
        let frame = frame("frm_1");
        let mut attestation = signed(&frame, &SEED);
        attestation.signed_commitment = "sha256:nope".into();
        assert_eq!(
            store_trusting(&SEED).check(PROVIDER, &frame, &attestation),
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedCommitment
            }
        );
    }

    #[test]
    fn an_unrecognised_algorithm_is_uncheckable_not_invalid() {
        // F8: "I cannot check this" is never "this is forged", and holding a key
        // does not change that.
        let frame = frame("frm_1");
        let mut attestation = signed(&frame, &SEED);
        attestation.algorithm = "dilithium3".into();
        assert_eq!(
            store_trusting(&SEED).check(PROVIDER, &frame, &attestation),
            AttestationState::UnknownAlgorithm {
                algorithm: "dilithium3".to_string()
            }
        );
    }

    #[test]
    fn an_oversized_identifier_cannot_grow_the_audit_record_without_bound() {
        // Attacker-controlled strings are echoed into the audit; the echo is
        // capped so a megabyte key_id costs a bounded record.
        let frame = frame("frm_1");
        let mut attestation = signed(&frame, &SEED);
        attestation.key_id = "k".repeat(1_000_000);
        match store_trusting(&SEED).check(PROVIDER, &frame, &attestation) {
            AttestationState::NoTrustedKey { key_id } => {
                assert_eq!(key_id.len(), MAX_ECHOED_IDENTIFIER);
            }
            other => panic!("expected NoTrustedKey, got {other:?}"),
        }

        let mut oversized_algorithm = signed(&frame, &SEED);
        oversized_algorithm.algorithm = "å".repeat(1_000);
        match store_trusting(&SEED).check(PROVIDER, &frame, &oversized_algorithm) {
            AttestationState::UnknownAlgorithm { algorithm } => {
                assert!(algorithm.len() <= MAX_ECHOED_IDENTIFIER);
                // Cut at a char boundary: the echo is still a valid string.
                assert!(algorithm.chars().all(|c| c == 'å'));
            }
            other => panic!("expected UnknownAlgorithm, got {other:?}"),
        }
    }

    #[test]
    fn an_enormous_signature_is_rejected_on_its_length_before_any_decoding() {
        // The bound that matters on a fan-out: a ten-megabyte signature must
        // cost a length comparison, not a ten-megabyte hex decode.
        let frame = frame("frm_1");
        let mut attestation = signed(&frame, &SEED);
        attestation.signature = "ab".repeat(5_000_000);
        assert_eq!(
            store_trusting(&SEED).check(PROVIDER, &frame, &attestation),
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedSignature
            }
        );
    }

    #[test]
    fn a_result_reports_a_state_for_every_frame_including_the_unsigned_ones() {
        let signed_frame = frame("frm_signed");
        let bare_frame = frame("frm_bare");
        let attestation = signed(&signed_frame, &SEED);
        let result = ContextQueryResult {
            frame_attestations: vec![FrameAttestation::signed(
                signed_frame.identity(PROVIDER),
                attestation,
            )],
            ..ContextQueryResult::unattested(
                vec![signed_frame.clone(), bare_frame.clone()],
                false,
                None,
            )
        };

        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);

        assert_eq!(outcomes.len(), 2, "one outcome per frame, always");
        assert_eq!(outcomes[0].frame, signed_frame.identity(PROVIDER));
        assert!(outcomes[0].state.is_attested());
        assert_eq!(outcomes[1].frame, bare_frame.identity(PROVIDER));
        assert_eq!(outcomes[1].state, AttestationState::Unattested);
    }

    #[test]
    fn an_attestation_naming_a_frame_that_is_not_in_the_result_is_ignored() {
        let served = frame("frm_served");
        let elsewhere = frame("frm_elsewhere");
        let result = ContextQueryResult {
            frame_attestations: vec![FrameAttestation::signed(
                elsewhere.identity(PROVIDER),
                signed(&elsewhere, &SEED),
            )],
            ..ContextQueryResult::unattested(vec![served.clone()], false, None)
        };
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].state, AttestationState::Unattested);
    }

    #[test]
    fn an_entry_matching_the_frame_id_but_not_the_digest_does_not_attest_it() {
        // #161: matching is on the whole identity triple. Two frames can share
        // an id and differ in bytes, and lending the first one's signature to
        // the second is the substitution the binding exists to prevent.
        let served = frame("frm_1");
        let mut impostor_identity = served.identity(PROVIDER);
        impostor_identity.content_digest = Some(format!("sha256:{}", "ee".repeat(32)));
        let result = ContextQueryResult {
            frame_attestations: vec![FrameAttestation::signed(
                impostor_identity,
                signed(&served, &SEED),
            )],
            ..ContextQueryResult::unattested(vec![served.clone()], false, None)
        };
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);
        assert_eq!(outcomes[0].state, AttestationState::Unattested);
    }

    #[test]
    fn at_most_one_attestation_per_frame_is_examined() {
        // A provider flooding duplicates for one frame buys one verification,
        // and the frames are still served either way.
        let one = frame("frm_1");
        let identity = one.identity(PROVIDER);
        let good = signed(&one, &SEED);
        let bad = signed(&one, &OTHER_SEED);
        // First entry wins, so the leading good one decides.
        let result = ContextQueryResult {
            frame_attestations: vec![
                FrameAttestation::signed(identity.clone(), good),
                FrameAttestation::signed(identity.clone(), bad.clone()),
            ],
            ..ContextQueryResult::unattested(vec![one.clone()], false, None)
        };
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);
        assert!(outcomes[0].state.is_attested());

        // And the scan is capped at frames.len(), so a leading junk entry is
        // what a provider that over-sends gets judged on — a degradation that
        // falls on that provider, never a dropped frame.
        let mut unrelated = identity.clone();
        unrelated.frame_id = "frm_unrelated".into();
        let result = ContextQueryResult {
            frame_attestations: vec![
                FrameAttestation::signed(unrelated, bad),
                FrameAttestation::signed(identity, signed(&one, &SEED)),
            ],
            ..ContextQueryResult::unattested(vec![one.clone()], false, None)
        };
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].state, AttestationState::Unattested);
    }

    /// Sign the Merkle root over `frames` and hand back a result whose every
    /// frame is attested *only* through an inclusion proof — the cheap shape:
    /// one signature for the whole answer, no per-frame signature at all.
    fn root_signed_result(frames: Vec<ContextFrame>, seed: &[u8; 32]) -> ContextQueryResult {
        use contextgraph_types::{inclusion_proof, result_set_commitments, result_set_root};

        // Canonical order is by `FrameId`, not the order the frames were
        // served in, so the leaf index comes from the commitment list.
        let ordered = result_set_commitments(PROVIDER, &frames);
        let commitments: Vec<[u8; 32]> = ordered.iter().map(|(_, c)| *c).collect();
        let root = result_set_root(PROVIDER, &frames);
        let entries = ordered
            .iter()
            .enumerate()
            .map(|(index, (id, _))| {
                FrameAttestation::proven(
                    id.clone(),
                    inclusion_proof(&commitments, index).expect("index is in range"),
                )
            })
            .collect();
        ContextQueryResult {
            frame_attestations: entries,
            result_attestation: Some(sign_commitment(
                &root,
                seed,
                KEY_ID,
                "docs-provider",
                "2026-08-29T00:00:00Z",
            )),
            ..ContextQueryResult::unattested(frames, false, None)
        }
    }

    #[test]
    fn a_frame_attested_only_through_an_inclusion_proof_is_attested() {
        // #161: the canonical `FrameAttestation` makes `attestation` optional
        // so one root signature can stand for n frames. A host that only knew
        // how to check per-frame signatures would call every one of these
        // unattested, which would make the cheapest honest shape the one
        // nothing can verify.
        let frames = vec![frame("frm_1"), frame("frm_2"), frame("frm_3")];
        let result = root_signed_result(frames.clone(), &SEED);

        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);

        assert_eq!(outcomes.len(), 3);
        for (outcome, frame) in outcomes.iter().zip(&frames) {
            assert_eq!(outcome.frame, frame.identity(PROVIDER));
            assert_eq!(
                outcome.state,
                AttestationState::Attested {
                    key_id: KEY_ID.to_string(),
                    attester_id: "docs-provider".to_string(),
                    covers_content: true,
                },
                "every leaf of the signed root is attested by the one signature"
            );
        }
    }

    #[test]
    fn a_proof_of_a_root_this_host_holds_no_key_for_is_a_configuration_gap() {
        let frames = vec![frame("frm_1"), frame("frm_2")];
        let result = root_signed_result(frames, &SEED);
        let outcomes = TrustStore::new().check_result(PROVIDER, &result);
        assert_eq!(
            outcomes[0].state,
            AttestationState::NoTrustedKey {
                key_id: KEY_ID.to_string()
            },
            "no key means nothing was checked, never that something failed"
        );
    }

    #[test]
    fn a_proof_that_recomputes_a_different_root_is_a_commitment_mismatch() {
        // The loud case: a frame edited after the root was signed. The frame's
        // *identity* is unchanged — provenance is not part of it — so the entry
        // still names this frame, and the leaf it recomputes is a different one.
        let frames = vec![frame("frm_1"), frame("frm_2")];
        let mut result = root_signed_result(frames, &SEED);
        result.frames[0].provenance.clear();

        match store_trusting(&SEED).check_result(PROVIDER, &result)[0].state {
            AttestationState::Invalid {
                verdict: AttestationVerdict::CommitmentMismatch { .. },
            } => {}
            ref other => panic!("expected a CommitmentMismatch, got {other:?}"),
        }
    }

    #[test]
    fn an_inclusion_proof_with_no_signed_root_is_unusable_not_unattested() {
        // F9 treats it as unattested for every decision, but the state is named
        // so an audit can tell "the provider signs nothing" from "the provider
        // sent evidence that does not resolve".
        let frames = vec![frame("frm_1"), frame("frm_2")];
        let mut result = root_signed_result(frames, &SEED);
        result.result_attestation = None;

        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);

        assert_eq!(outcomes[0].state, AttestationState::UnusableEvidence);
        assert!(!outcomes[0].state.is_attested());
        assert!(
            outcomes[0].state.was_offered(),
            "something was offered; it just could not be checked"
        );
    }

    #[test]
    fn an_entry_carrying_neither_a_signature_nor_a_proof_asserts_nothing() {
        let one = frame("frm_1");
        let mut entry = FrameAttestation::signed(one.identity(PROVIDER), signed(&one, &SEED));
        entry.attestation = None;
        let result = ContextQueryResult {
            frame_attestations: vec![entry.clone()],
            ..ContextQueryResult::unattested(vec![one.clone()], false, None)
        };
        assert!(!entry.carries_evidence());
        assert_eq!(
            store_trusting(&SEED).check_result(PROVIDER, &result)[0].state,
            AttestationState::UnusableEvidence
        );
    }

    #[test]
    fn a_per_frame_signature_is_preferred_over_a_proof_on_the_same_entry() {
        // Both shapes on one entry: check the narrower claim, which binds this
        // frame directly with no tree in between, and costs one verification
        // rather than a walk plus one.
        let frames = vec![frame("frm_1"), frame("frm_2")];
        let mut result = root_signed_result(frames.clone(), &SEED);
        // A per-frame signature from a key nobody trusts. If the proof were
        // preferred the frame would read as attested; the signature must decide.
        result.frame_attestations[0].attestation = Some(signed(&frames[0], &OTHER_SEED));

        assert_eq!(
            store_trusting(&SEED).check_result(PROVIDER, &result)[0].state,
            AttestationState::Invalid {
                verdict: AttestationVerdict::BadSignature
            }
        );
    }

    #[test]
    fn an_unbounded_inclusion_path_is_rejected_on_its_length() {
        // Every step of the path costs a hash and the path comes from the
        // provider. `MAX_INCLUSION_PATH_STEPS` caps the walk before it starts.
        use contextgraph_types::{InclusionStep, MAX_INCLUSION_PATH_STEPS};

        let one = frame("frm_1");
        let mut result = root_signed_result(vec![one.clone()], &SEED);
        result.frame_attestations[0].inclusion_proof = Some(InclusionProof {
            leaf_index: 0,
            leaf_count: usize::MAX,
            path: vec![
                InclusionStep {
                    sibling: format!("sha256:{}", "11".repeat(32)),
                    sibling_is_left: false,
                };
                MAX_INCLUSION_PATH_STEPS + 1
            ],
        });

        assert_eq!(
            store_trusting(&SEED).check_result(PROVIDER, &result)[0].state,
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedCommitment
            }
        );
    }

    #[test]
    fn an_empty_store_checks_nothing_and_rejects_nothing() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let store = TrustStore::new();
        assert!(store.is_empty());
        assert_eq!(
            store.check(PROVIDER, &frame, &attestation),
            AttestationState::NoTrustedKey {
                key_id: KEY_ID.to_string()
            }
        );
    }

    #[test]
    fn revoking_a_key_stops_it_attesting_and_reports_whether_it_removed_one() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let mut store = store_trusting(&SEED);
        assert!(store.check(PROVIDER, &frame, &attestation).is_attested());
        assert!(store.revoke(PROVIDER, KEY_ID));
        assert!(!store.revoke(PROVIDER, KEY_ID), "already gone");
        assert!(store.is_empty());
        assert!(!store.check(PROVIDER, &frame, &attestation).is_attested());
    }

    #[test]
    fn a_key_whose_hex_does_not_decode_is_refused_at_the_door() {
        assert!(TrustedKey::ed25519_hex("k", "not hex").is_none());
        assert!(
            TrustedKey::ed25519_hex("k", "ab".repeat(16)).is_none(),
            "too short"
        );
        let valid = encode_hex(&public_key_for(&SEED));
        assert!(TrustedKey::ed25519_hex("k", &valid).is_some());
    }

    #[test]
    fn a_stored_key_that_does_not_decode_is_an_operator_bug_named_as_one() {
        // Constructed around `ed25519_hex`'s guard, as a hand-edited persisted
        // store could be.
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let mut store = TrustStore::new();
        store.trust(
            PROVIDER,
            TrustedKey {
                key_id: KEY_ID.into(),
                public_key: "zz".repeat(32),
            },
        );
        assert_eq!(
            store.check(PROVIDER, &frame, &attestation),
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedKey
            }
        );
    }

    #[test]
    fn a_fingerprint_is_over_the_key_bytes_and_survives_a_round_trip() {
        let key = TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED));
        let fingerprint = key.fingerprint().expect("a well-formed key has one");
        assert!(fingerprint.starts_with("sha256:"));
        assert_eq!(fingerprint.len(), COMMITMENT_LEN);
        // The same key spelled in uppercase hex is the same key.
        let shouty = TrustedKey {
            key_id: KEY_ID.into(),
            public_key: key.public_key.to_uppercase(),
        };
        assert_eq!(shouty.fingerprint(), Some(fingerprint));
    }

    #[test]
    fn the_store_round_trips_through_serde_so_a_host_can_persist_it() {
        let store = store_trusting(&SEED);
        let json = serde_json::to_string(&store).expect("serializable");
        let back: TrustStore = serde_json::from_str(&json).expect("deserializable");
        assert_eq!(store, back);
    }

    #[test]
    fn an_empty_ledger_says_not_checked_rather_than_unattested() {
        let ledger = AttestationLedger::new();
        let id = frame("frm_1").identity(PROVIDER);
        assert!(ledger.is_empty());
        assert_eq!(ledger.state_for(&id), AttestationState::NotChecked);
        assert!(!ledger.state_for(&id).was_offered());
    }

    #[test]
    fn a_ledger_collects_outcomes_and_answers_by_identity() {
        let one = frame("frm_1");
        let two = frame("frm_2");
        let ledger: AttestationLedger = vec![
            FrameAttestationOutcome {
                frame: one.identity(PROVIDER),
                state: AttestationState::Attested {
                    key_id: KEY_ID.into(),
                    attester_id: "docs-provider".into(),
                    covers_content: true,
                },
            },
            FrameAttestationOutcome {
                frame: two.identity(PROVIDER),
                state: AttestationState::Unattested,
            },
        ]
        .into_iter()
        .collect();

        assert_eq!(ledger.len(), 2);
        assert!(ledger.state_for(&one.identity(PROVIDER)).is_attested());
        assert_eq!(
            ledger.state_for(&two.identity(PROVIDER)),
            AttestationState::Unattested
        );
        // A frame from a different provider is a different identity.
        assert_eq!(
            ledger.state_for(&one.identity("elsewhere")),
            AttestationState::NotChecked
        );
    }
}
