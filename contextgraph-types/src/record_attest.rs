//! Record content addressing and record attestation — the lifecycle profile's
//! `record_hash` and [`RecordAttestation`], implemented
//! ([`docs/profiles/context-exchange-provider.md`][profile] §3 and §7,
//! [ADR 0017](../../docs/adr/0017-record-hash-and-record-attestation.md)).
//!
//! [profile]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/profiles/context-exchange-provider.md
//!
//! Where [`attest`](crate::attest) covers the *frame* layer a `context/query`
//! returns, this covers the *record* layer a Context Exchange Provider appends,
//! gets, and resolves. Two constructions:
//!
//! 1. **`record_hash`** ([`record_hash`]) — `sha256:<hex>` over the RFC 8785
//!    (JCS) canonicalization of a record **with its own `record_hash` member
//!    removed from the preimage** (profile LH1). This is the record's identity:
//!    what idempotent replay keys on, what a lineage cites, and what an
//!    attestation signs.
//! 2. **[`RecordAttestation`]** ([`verify_record_attestation`]) — a detached
//!    Ed25519 signature over that hash under a record-layer domain tag.
//!
//! # Why JCS here and not at the frame layer
//!
//! [ADR 0010](../../docs/adr/0010-provenance-attestation.md) §3 rejects JCS for
//! a provenance link and it is right to: a link is six optional strings, and
//! requiring every implementer to obtain a conforming JSON canonicalizer to hash
//! six strings is a tax with no return. A record is the opposite shape — an
//! open-ended JSON document with an extensible body, a `BTreeMap` of extensions,
//! and floating-point confidences. There is no typed encoding to write down that
//! stays correct as the profile grows a member, so the canonicalization has to be
//! generic, and RFC 8785 is the one generic rule with cross-language
//! implementations to reconcile against.
//!
//! The cost is real and this module does not hide it: JCS number serialization
//! is ECMAScript `Number::toString`, whose exponent thresholds and
//! shortest-round-trip digits are where independent implementations quietly
//! disagree. So this crate delegates rather than hand-rolls, and
//! `contextgraph-conformance` pins the RFC's own published vectors
//! (`tests/fixtures/record-hash-vectors.json`) as bytes a third party can diff
//! against when their hash comes out different.
//!
//! # Why the omitted member is *removed*, not blanked
//!
//! Profile LH1 says the member is removed from the preimage, and the observable
//! consequence is worth stating: a record hashes identically whether it carries
//! no `record_hash` at all, the right one, or a wrong one. A producer therefore
//! computes the hash of the record it is about to publish without first having
//! to invent a placeholder, and a verifier never has to know which placeholder
//! the producer chose. A blanking rule would have made the placeholder itself
//! part of the interop contract — one more thing to get wrong in another
//! language for no gain.
//!
//! Only the **top-level** member is removed. A `record_hash` nested inside
//! `extensions` or a body member is ordinary content and stays in the preimage.
//!
//! # Why the signature is domain-separated
//!
//! A frame commitment is already domain-bound by construction: it is
//! `SHA256(domain::FRAME ‖ …)`, so nothing else in this protocol produces those
//! 32 bytes. A `record_hash` is a plain SHA-256 over a JSON document, which any
//! number of unrelated systems also compute. Signing it raw would make one
//! Ed25519 signature mean whatever the presenter says it means, so the signed
//! message is [`RECORD_ATTESTATION_DOMAIN`] followed by the hash's 32 raw bytes.
//! Both halves are fixed length, so the encoding is injective without a length
//! prefix, and any language can build it from the digest string alone.

use std::collections::BTreeMap;

use crate::attest::AttestationVerdict;
use crate::key_validity::{KeyValidity, WindowPosition};
// Only the signing and verifying code names the type; the hashing half and the
// ungated constants below do not, so an import at file scope would be unused in
// a default build.
#[cfg(feature = "record-attestation")]
use crate::record::RecordAttestation;

/// The envelope member a record's own hash lives in, and the one member removed
/// from its preimage (profile LH1).
pub const RECORD_HASH_MEMBER: &str = "record_hash";

/// The domain-separation tag a [record attestation](crate::record::RecordAttestation)
/// signs under.
///
/// Normative: the signed message is these bytes followed by the 32 raw bytes of
/// `signed_record_hash`. A reimplementation in another language that signs
/// anything else produces signatures this protocol will not accept, which is the
/// point — a record attestation must not be interchangeable with a frame
/// attestation or with a signature some unrelated system produced over the same
/// SHA-256.
pub const RECORD_ATTESTATION_DOMAIN: &[u8] = b"contextgraph/attest/1/record";

/// Why a `record_hash` could not be computed or used.
///
/// Named rather than a bare string because the three cases call for different
/// responses: a non-object is a caller bug, a canonicalization failure is a
/// record carrying something JCS refuses (a NaN, a lone surrogate), and a
/// malformed digest is a wire value that failed its grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordHashError {
    /// The value handed in was not a JSON object, so it has no members to
    /// remove and is not a record.
    NotAnObject,
    /// The record could not be canonicalized under RFC 8785. JCS **must**
    /// refuse `NaN`, `Infinity`, and lone surrogates (RFC 8785 §3.2.2.2,
    /// §3.2.2.3), so this is a real finding about the record, not a library
    /// hiccup.
    NotCanonicalizable(String),
    /// A typed record would not serialize to JSON.
    NotSerializable(String),
    /// A digest string was not the `sha256:<64 lowercase hex>` the protocol
    /// grammar requires (`SPEC.md` §6.2).
    MalformedDigest(String),
}

impl core::fmt::Display for RecordHashError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "a record must be a JSON object"),
            Self::NotCanonicalizable(why) => {
                write!(f, "record is not canonicalizable under RFC 8785: {why}")
            }
            Self::NotSerializable(why) => write!(f, "record does not serialize to JSON: {why}"),
            Self::MalformedDigest(found) => {
                write!(
                    f,
                    "expected a sha256:<64 lowercase hex> digest, found {found}"
                )
            }
        }
    }
}

impl std::error::Error for RecordHashError {}

/// The message an Ed25519 record attestation signs: the domain tag followed by
/// the digest's 32 raw bytes.
///
/// Public and ungated because it is the normative rule, not an implementation
/// detail — a provider signing in an HSM builds these bytes, signs them with its
/// own backend, and never hands this crate a secret.
pub fn record_attestation_message(record_hash: &str) -> Result<Vec<u8>, RecordHashError> {
    let raw = raw_digest(record_hash)?;
    let mut message = Vec::with_capacity(RECORD_ATTESTATION_DOMAIN.len() + raw.len());
    message.extend_from_slice(RECORD_ATTESTATION_DOMAIN);
    message.extend_from_slice(&raw);
    Ok(message)
}

/// Parse a `sha256:<64 lowercase hex>` digest into its 32 raw bytes.
fn raw_digest(digest: &str) -> Result<[u8; 32], RecordHashError> {
    if !crate::validate::is_well_formed_digest(digest) {
        return Err(RecordHashError::MalformedDigest(digest.to_string()));
    }
    let hex = digest
        .split_once(':')
        .map(|(_, hex)| hex)
        .ok_or_else(|| RecordHashError::MalformedDigest(digest.to_string()))?;
    let mut out = [0u8; 32];
    let (pairs, _) = hex.as_bytes().as_chunks::<2>();
    for (slot, pair) in out.iter_mut().zip(pairs) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| RecordHashError::MalformedDigest(digest.to_string()))?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| RecordHashError::MalformedDigest(digest.to_string()))?;
        *slot = (hi * 16 + lo) as u8;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Key-id validity windows (profile LC3, ADR 0028).
// ---------------------------------------------------------------------------

/// One record-layer signing key a verifier holds: its `key_id`, its raw public
/// key, and the [`KeyValidity`] window it was in service for.
///
/// Profile LC3 says rotation is "by key-id validity windows". This is the
/// verifier's half of that sentence: a key is retired by closing its window,
/// not by deleting it, so evidence received while it was in service keeps
/// verifying and evidence received after it lapsed never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKey {
    /// The `key_id` a [record attestation](crate::RecordAttestation) must name
    /// to be checked against this key.
    pub key_id: String,
    /// The raw public key, in the form the named algorithm's verifier takes
    /// (32 bytes for `ed25519`).
    pub public_key: Vec<u8>,
    /// When the key is in service. [`KeyValidity::unbounded`] for a key that
    /// never lapses — the behavior before windows existed.
    pub validity: KeyValidity,
}

impl RecordKey {
    /// A key with no window: in service at every instant.
    pub fn unbounded(key_id: impl Into<String>, public_key: impl Into<Vec<u8>>) -> Self {
        Self {
            key_id: key_id.into(),
            public_key: public_key.into(),
            validity: KeyValidity::unbounded(),
        }
    }

    /// The same key, in service only within `validity`.
    pub fn with_validity(mut self, validity: KeyValidity) -> Self {
        self.validity = validity;
        self
    }
}

/// A verifier's record-layer keys, by `key_id` — the registry profile LC3's
/// "key-id validity windows" presuppose.
///
/// Held by the **verifier**, never carried by the wire: which keys a verifier
/// believes, and for which instants, is its own decision about a producer, as
/// [ADR 0016](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0016-attestation-trust-roots.md)
/// decides for the frame layer. The protocol fixes only how a window is spelled
/// ([`KeyValidity`]) and which instant it is evaluated at: the one at which the
/// verifier **received** the attestation — never its unsigned `issued_at`
/// ([ADR 0028](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0028-key-validity-windows-are-evaluated-at-receipt.md)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordKeyRing {
    keys: BTreeMap<String, RecordKey>,
}

impl RecordKeyRing {
    /// An empty ring: every attestation resolves to
    /// [`RecordKeyVerdict::UnknownKey`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Hold `key`, returning the key previously held under the same `key_id`.
    pub fn insert(&mut self, key: RecordKey) -> Option<RecordKey> {
        self.keys.insert(key.key_id.clone(), key)
    }

    /// The key held under `key_id`, if any.
    pub fn get(&self, key_id: &str) -> Option<&RecordKey> {
        self.keys.get(key_id)
    }

    /// Stop holding the key under `key_id`. Retiring a key is better done by
    /// closing its window: removal also stops it verifying evidence received
    /// while it was in service.
    pub fn remove(&mut self, key_id: &str) -> Option<RecordKey> {
        self.keys.remove(key_id)
    }

    /// Every key held, in `key_id` order.
    pub fn keys(&self) -> impl Iterator<Item = &RecordKey> {
        self.keys.values()
    }

    /// How many keys are held.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether no key is held.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// What a [`RecordKeyRing`] found when it checked one record attestation.
///
/// Named outcomes, for the reason [`AttestationVerdict`] is named: "I hold no
/// such key", "the key was not in service when I received this", and "the
/// signature is forged" call for three different responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordKeyVerdict {
    /// The ring holds no key under the attestation's `key_id`. Nothing was
    /// checked, so nothing is known about the signature.
    UnknownKey {
        /// The `key_id` the attestation named.
        key_id: String,
    },
    /// The key is known, and the instant the verifier received the attestation
    /// falls outside its window — or the window could not be evaluated. The
    /// signature was not checked: no key was in service to check it against.
    ///
    /// This is the outcome rotation exists to produce. A compromised key whose
    /// window has closed stops verifying new evidence however its holder
    /// back-dates `issued_at`, because `issued_at` is never consulted.
    KeyNotInService {
        /// The key that was not in service.
        key_id: String,
        /// The receipt instant the window was evaluated at.
        received_at: String,
        /// Where that instant fell: before the window, after it, or
        /// unevaluable.
        position: WindowPosition,
    },
    /// The key was in service; this is what checking the signature found.
    Checked {
        /// The key that was checked.
        key_id: String,
        /// The signature verdict. Only
        /// [`Valid`](crate::AttestationVerdict::Valid) is acceptable (LC5).
        verdict: AttestationVerdict,
    },
}

impl RecordKeyVerdict {
    /// Whether the attestation verified under a key that was in service when
    /// it was received. Every other outcome is `false` (profile LC5: "I cannot
    /// check this" and "this is good" are never the same answer).
    pub fn is_valid(&self) -> bool {
        matches!(
            self,
            Self::Checked {
                verdict: AttestationVerdict::Valid,
                ..
            }
        )
    }
}

// ---------------------------------------------------------------------------
// Hashing — gated, because RFC 8785 needs a conforming canonicalizer and SHA-256.
// ---------------------------------------------------------------------------

#[cfg(feature = "record-hash")]
mod hashing {
    use super::*;
    use crate::record::ContextRecord;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    /// The exact bytes a record's `record_hash` is taken over: the RFC 8785
    /// (JCS) canonicalization of the record with its top-level `record_hash`
    /// member removed (profile LH1).
    ///
    /// Exposed alongside [`record_hash`] because a hash mismatch between two
    /// implementations is unreadable and a byte diff of the preimage is not —
    /// this is the function an implementer reaches for at 2am, and the one the
    /// golden vectors pin.
    pub fn record_hash_preimage(record: &Value) -> Result<Vec<u8>, RecordHashError> {
        let mut preimage = record.clone();
        preimage
            .as_object_mut()
            .ok_or(RecordHashError::NotAnObject)?
            .remove(RECORD_HASH_MEMBER);
        serde_json_canonicalizer::to_vec(&preimage)
            .map_err(|error| RecordHashError::NotCanonicalizable(error.to_string()))
    }

    /// A record's content-addressed identity (profile LH1):
    /// `"sha256:" + hex(sha256(JCS(record without its record_hash member)))`.
    pub fn record_hash(record: &Value) -> Result<String, RecordHashError> {
        let preimage = record_hash_preimage(record)?;
        let digest: [u8; 32] = Sha256::digest(&preimage).into();
        Ok(crate::attest::digest_string(&digest))
    }

    /// [`record_hash`] for a record already parsed into the reference type.
    ///
    /// Equal to [`record_hash`] of the bytes the record was parsed from — for
    /// **every** record the profile permits, not only the ones this crate
    /// happens to model. A member outside the reference types survives the
    /// round-trip in [`ContextRecord::extra`], so a host that relays an
    /// extended record and content-addresses it here computes the identity the
    /// record was published with rather than a silently different one
    /// (issue #118).
    ///
    /// That guarantee is checked, not asserted: `contextgraph-conformance`'s
    /// `record_hash_typed_wire_parity` suite hashes every lifecycle fixture
    /// both ways and compares, and does it again with a namespaced member no
    /// reference type models injected into each one.
    ///
    /// [`record_hash`] remains the normative primitive (profile LH1). Prefer it
    /// when you already hold the wire value — this is the convenience for when
    /// you hold the type.
    pub fn record_hash_of(record: &ContextRecord) -> Result<String, RecordHashError> {
        let value = serde_json::to_value(record)
            .map_err(|error| RecordHashError::NotSerializable(error.to_string()))?;
        record_hash(&value)
    }

    /// Whether a record's stored `record_hash` is the one its content produces.
    ///
    /// `Ok(false)` is the interesting answer: the record was edited after it was
    /// hashed, or was hashed by an implementation that canonicalizes
    /// differently. A record with no `record_hash` member at all is `Ok(false)`
    /// rather than an error — it is unhashed, not malformed.
    pub fn record_hash_is_current(record: &Value) -> Result<bool, RecordHashError> {
        let stored = record.get(RECORD_HASH_MEMBER).and_then(Value::as_str);
        Ok(stored == Some(record_hash(record)?.as_str()))
    }
}

#[cfg(feature = "record-hash")]
pub use hashing::{record_hash, record_hash_is_current, record_hash_of, record_hash_preimage};

// ---------------------------------------------------------------------------
// Attestation — gated further, because signatures need Ed25519.
// ---------------------------------------------------------------------------

#[cfg(feature = "record-attestation")]
mod crypto {
    use super::*;
    use crate::attest::ALGORITHM_ED25519;
    use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
    use serde_json::Value;

    /// Verify a detached attestation against the record it claims to sign.
    ///
    /// Recomputes the record's hash rather than trusting the stored member, so a
    /// record whose `record_hash` was rewritten to match a stolen signature is
    /// caught here and not merely at the hash check.
    ///
    /// `Err` means the record could not be hashed at all — a distinct outcome
    /// from any verdict, because "this document is not a record" is not a
    /// statement about the signature.
    pub fn verify_record_attestation(
        record: &Value,
        attestation: &RecordAttestation,
        public_key: &[u8],
    ) -> Result<AttestationVerdict, RecordHashError> {
        let expected = super::hashing::record_hash(record)?;
        Ok(verify_signed_record_hash(
            &expected,
            attestation,
            public_key,
        ))
    }

    /// Verify a detached attestation against an already-computed `record_hash`.
    ///
    /// The primitive an auditor uses when they hold the hash and the signature
    /// but not the record — which is the whole point of a detached attestation
    /// over a content address.
    pub fn verify_signed_record_hash(
        expected_record_hash: &str,
        attestation: &RecordAttestation,
        public_key: &[u8],
    ) -> AttestationVerdict {
        if attestation.algorithm != ALGORITHM_ED25519 {
            return AttestationVerdict::UnknownAlgorithm(attestation.algorithm.clone());
        }
        let Ok(message) = record_attestation_message(&attestation.signed_record_hash) else {
            return AttestationVerdict::MalformedCommitment;
        };
        // Compare hashes *before* touching the signature: a mismatch means the
        // record changed after signing, and telling an operator that is far more
        // useful than the "bad signature" a naive order would report.
        if attestation.signed_record_hash != expected_record_hash {
            return AttestationVerdict::CommitmentMismatch {
                expected: expected_record_hash.to_string(),
                signed: attestation.signed_record_hash.clone(),
            };
        }
        let Ok(key_bytes) = <[u8; 32]>::try_from(public_key) else {
            return AttestationVerdict::MalformedKey;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
            return AttestationVerdict::MalformedKey;
        };
        let Some(sig_bytes) = hex_bytes(&attestation.signature) else {
            return AttestationVerdict::MalformedSignature;
        };
        let Ok(sig_bytes) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            return AttestationVerdict::MalformedSignature;
        };
        let signature = Signature::from_bytes(&sig_bytes);
        // `verify_strict` rejects small-order keys and the malleable signature
        // forms `verify` tolerates. A signature two verifiers can disagree about
        // is not evidence.
        match verifying_key.verify_strict(&message, &signature) {
            Ok(()) => AttestationVerdict::Valid,
            Err(_) => AttestationVerdict::BadSignature,
        }
    }

    /// Sign a `record_hash` in-process, for providers content to hold key
    /// material in memory.
    ///
    /// A provider using an HSM or KMS calls [`record_attestation_message`]
    /// instead, signs those bytes with its own backend, and assembles the
    /// [`RecordAttestation`] by hand — the protocol specifies the preimage, never
    /// the custody of the key.
    pub fn sign_record_attestation(
        record_hash: &str,
        signing_key_seed: &[u8; 32],
        key_id: impl Into<String>,
        attester_id: impl Into<String>,
        issued_at: impl Into<String>,
    ) -> Result<RecordAttestation, RecordHashError> {
        let message = record_attestation_message(record_hash)?;
        let signing_key = SigningKey::from_bytes(signing_key_seed);
        let signature = signing_key.sign(&message);
        Ok(RecordAttestation::new(
            record_hash,
            key_id,
            ALGORITHM_ED25519,
            attester_id,
            hex_string(&signature.to_bytes()),
            issued_at,
        ))
    }

    /// Sign the record's own recomputed hash — the convenience a provider
    /// appending a record wants, so the signed hash cannot drift from the
    /// content by a copy-paste.
    pub fn sign_record(
        record: &Value,
        signing_key_seed: &[u8; 32],
        key_id: impl Into<String>,
        attester_id: impl Into<String>,
        issued_at: impl Into<String>,
    ) -> Result<RecordAttestation, RecordHashError> {
        let hash = super::hashing::record_hash(record)?;
        sign_record_attestation(&hash, signing_key_seed, key_id, attester_id, issued_at)
    }

    /// Lowercase hex, the encoding every signature and digest on this wire uses.
    fn hex_string(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble is < 16"));
            out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble is < 16"));
        }
        out
    }

    /// Parse lowercase hex into bytes. `None` on any non-hex byte or odd length.
    fn hex_bytes(s: &str) -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) {
            return None;
        }
        let mut out = Vec::with_capacity(s.len() / 2);
        let (pairs, _) = s.as_bytes().as_chunks::<2>();
        for pair in pairs {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
        }
        Some(out)
    }
}

#[cfg(feature = "record-attestation")]
pub use crypto::{
    sign_record, sign_record_attestation, verify_record_attestation, verify_signed_record_hash,
};

#[cfg(feature = "record-attestation")]
impl RecordKeyRing {
    /// Verify a record attestation against the key its `key_id` names, as of
    /// `received_at` — the instant **this verifier received the attestation**
    /// (ADR 0028).
    ///
    /// The order is fixed and cheapest-first: the key is looked up, then its
    /// window is evaluated, and only a key in service at `received_at` reaches
    /// the hash and the signature. An unknown key or a key out of service costs
    /// no hashing at all.
    ///
    /// `issued_at` is never read. It is outside the signed message (profile
    /// LC4), so the holder of a lapsed key can write any date there; the
    /// receipt instant is the one the signer cannot choose, and a signature
    /// cannot be received before it is made, so it bounds the signing time from
    /// above. An auditor replaying archived evidence passes the receipt
    /// instant recorded when the evidence was first taken in.
    ///
    /// `Err` means the record could not be hashed — see
    /// [`verify_record_attestation`].
    pub fn verify_record(
        &self,
        record: &serde_json::Value,
        attestation: &RecordAttestation,
        received_at: &str,
    ) -> Result<RecordKeyVerdict, RecordHashError> {
        match self.key_in_service(attestation, received_at) {
            Ok(key) => Ok(RecordKeyVerdict::Checked {
                key_id: key.key_id.clone(),
                verdict: verify_record_attestation(record, attestation, &key.public_key)?,
            }),
            Err(verdict) => Ok(verdict),
        }
    }

    /// [`verify_record`](Self::verify_record) for a verifier holding the
    /// `record_hash` but not the record — the detached-attestation case
    /// [`verify_signed_record_hash`] serves.
    pub fn verify_record_hash(
        &self,
        expected_record_hash: &str,
        attestation: &RecordAttestation,
        received_at: &str,
    ) -> RecordKeyVerdict {
        match self.key_in_service(attestation, received_at) {
            Ok(key) => RecordKeyVerdict::Checked {
                key_id: key.key_id.clone(),
                verdict: verify_signed_record_hash(
                    expected_record_hash,
                    attestation,
                    &key.public_key,
                ),
            },
            Err(verdict) => verdict,
        }
    }

    /// The key an attestation names, if the ring holds it and it was in
    /// service at `received_at`; otherwise the verdict that says why not.
    fn key_in_service(
        &self,
        attestation: &RecordAttestation,
        received_at: &str,
    ) -> Result<&RecordKey, RecordKeyVerdict> {
        let Some(key) = self.keys.get(&attestation.key_id) else {
            return Err(RecordKeyVerdict::UnknownKey {
                key_id: attestation.key_id.clone(),
            });
        };
        match key.validity.position(received_at) {
            WindowPosition::Within => Ok(key),
            position => Err(RecordKeyVerdict::KeyNotInService {
                key_id: key.key_id.clone(),
                received_at: received_at.to_string(),
                position,
            }),
        }
    }
}

#[cfg(all(test, feature = "record-attestation"))]
mod tests {
    use super::*;
    use crate::attest::{AttestationVerdict, public_key_for, sign_commitment};
    use serde_json::{Value, json};

    /// A deterministic seed. Tests need reproducible signatures, and this key
    /// signs nothing outside this file and the published golden vector.
    const SEED: [u8; 32] = [11u8; 32];

    fn record() -> Value {
        json!({
            "schema_version": "contextgraph/lifecycle/1.0-draft",
            "record_id": "rec_obs_0001",
            "lineage_id": "lin_obs_0001",
            "record_status": "active",
            "scope": { "repository_id": "repo_stella" },
            "sharing_scope": "repository",
            "observed_at": "2026-07-29T14:00:00Z",
            "origin": "observed",
            "record_hash": format!("sha256:{}", "a".repeat(64)),
            "provenance": { "origin_provider_id": "provider_example", "producer_kind": "agent" },
            "confidence": 0.82,
            "record_kind": "observation",
            "statement": "the api handler retries three times before surfacing a 502"
        })
    }

    // -- the omit-self rule (profile LH1) -----------------------------------

    #[test]
    fn a_records_own_hash_is_removed_from_its_preimage() {
        let mut absent = record();
        absent.as_object_mut().unwrap().remove(RECORD_HASH_MEMBER);

        let mut wrong = record();
        wrong[RECORD_HASH_MEMBER] = json!(format!("sha256:{}", "f".repeat(64)));

        // Removal, not blanking: all three preimages are byte-identical, so a
        // producer never has to invent a placeholder and a verifier never has to
        // know which one was chosen.
        assert_eq!(
            record_hash_preimage(&record()).unwrap(),
            record_hash_preimage(&absent).unwrap()
        );
        assert_eq!(
            record_hash_preimage(&record()).unwrap(),
            record_hash_preimage(&wrong).unwrap()
        );
        assert_eq!(record_hash(&record()), record_hash(&absent));
    }

    #[test]
    fn the_preimage_never_contains_the_hash_member() {
        let preimage = String::from_utf8(record_hash_preimage(&record()).unwrap()).unwrap();
        assert!(
            !preimage.contains(RECORD_HASH_MEMBER),
            "a record must never hash over its own hash: {preimage}"
        );
    }

    #[test]
    fn only_the_top_level_hash_member_is_removed() {
        // A `record_hash` nested inside an extension is ordinary content, and
        // dropping it would let a producer hide a value from the signature.
        let mut nested = record();
        nested["extensions"] = json!({ "record_hash": "sha256:nested" });
        let preimage = String::from_utf8(record_hash_preimage(&nested).unwrap()).unwrap();
        assert!(preimage.contains("sha256:nested"), "{preimage}");
        assert_ne!(record_hash(&nested), record_hash(&record()));
    }

    #[test]
    fn editing_any_content_changes_the_hash() {
        let mut edited = record();
        edited["statement"] = json!("the api handler retries four times");
        assert_ne!(record_hash(&edited), record_hash(&record()));
    }

    #[test]
    fn member_order_does_not_change_the_hash() {
        // JCS sorts members, so two serializations of the same record agree —
        // the property the whole scheme rests on.
        let forward: Value = serde_json::from_str(r#"{"a":1,"b":2,"record_hash":"x"}"#).unwrap();
        let reverse: Value = serde_json::from_str(r#"{"record_hash":"x","b":2,"a":1}"#).unwrap();
        assert_eq!(
            record_hash(&forward).unwrap(),
            record_hash(&reverse).unwrap()
        );
    }

    #[test]
    fn a_non_object_is_not_a_record() {
        assert_eq!(
            record_hash(&json!([1, 2, 3])),
            Err(RecordHashError::NotAnObject)
        );
    }

    #[test]
    fn a_stored_hash_is_checkable_against_the_content() {
        let mut correct = record();
        let computed = record_hash(&correct).unwrap();
        correct[RECORD_HASH_MEMBER] = json!(computed);
        assert!(record_hash_is_current(&correct).unwrap());

        correct["statement"] = json!("edited after hashing");
        assert!(!record_hash_is_current(&correct).unwrap());

        let mut unhashed = record();
        unhashed.as_object_mut().unwrap().remove(RECORD_HASH_MEMBER);
        assert!(
            !record_hash_is_current(&unhashed).unwrap(),
            "an unhashed record is not current; it is unhashed"
        );
    }

    #[test]
    fn the_typed_record_hashes_like_its_wire_form() {
        let typed: crate::ContextRecord = serde_json::from_value(record()).unwrap();
        let wire = serde_json::to_value(&typed).unwrap();
        assert_eq!(record_hash_of(&typed).unwrap(), record_hash(&wire).unwrap());
    }

    // -- RFC 8785 conformance ------------------------------------------------

    /// RFC 8785 §3.2.2–§3.2.4: the specification's own worked example. The
    /// canonical bytes below are Section 3.2.4's hexadecimal listing, entered
    /// verbatim from <https://www.rfc-editor.org/rfc/rfc8785.txt>.
    #[test]
    fn rfc_8785_section_3_2_worked_example_canonicalizes_byte_for_byte() {
        let input: Value = serde_json::from_str(
            r#"{
               "numbers": [333333333.33333329, 1E30, 4.50,
                           2e-3, 0.000000000000000000000000001],
               "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
               "literals": [null, true, false]
             }"#,
        )
        .expect("the RFC's input parses as JSON");

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x7b, 0x22, 0x6c, 0x69, 0x74, 0x65, 0x72, 0x61, 0x6c, 0x73, 0x22, 0x3a, 0x5b, 0x6e,
            0x75, 0x6c, 0x6c, 0x2c, 0x74, 0x72, 0x75, 0x65, 0x2c, 0x66, 0x61, 0x6c, 0x73, 0x65,
            0x5d, 0x2c, 0x22, 0x6e, 0x75, 0x6d, 0x62, 0x65, 0x72, 0x73, 0x22, 0x3a, 0x5b, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x2e, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x2c, 0x31, 0x65, 0x2b, 0x33, 0x30, 0x2c, 0x34, 0x2e, 0x35, 0x2c, 0x30,
            0x2e, 0x30, 0x30, 0x32, 0x2c, 0x31, 0x65, 0x2d, 0x32, 0x37, 0x5d, 0x2c, 0x22, 0x73,
            0x74, 0x72, 0x69, 0x6e, 0x67, 0x22, 0x3a, 0x22, 0xe2, 0x82, 0xac, 0x24, 0x5c, 0x75,
            0x30, 0x30, 0x30, 0x66, 0x5c, 0x6e, 0x41, 0x27, 0x42, 0x5c, 0x22, 0x5c, 0x5c, 0x5c,
            0x5c, 0x5c, 0x22, 0x2f, 0x22, 0x7d,
        ];

        // The canonicalizer takes a whole document; wrapping it in a record whose
        // only member is removed is not what is under test, so canonicalize
        // directly.
        let canonical = serde_json_canonicalizer::to_vec(&input).expect("canonicalizes");
        assert_eq!(
            canonical,
            expected,
            "RFC 8785 §3.2.4 pins these exact bytes; got {}",
            String::from_utf8_lossy(&canonical)
        );
    }

    /// RFC 8785 §3.2.3's property-sorting test data, with the expected order the
    /// RFC states. Sorting is by UTF-16 code unit, which is why the emoji (a
    /// surrogate pair, so a leading code unit of 0xD83D) sorts *before* U+FB33
    /// even though its code point is far higher.
    #[test]
    fn rfc_8785_section_3_2_3_sorts_property_names_by_utf16_code_unit() {
        let input: Value = serde_json::from_str(
            r#"{
               "\u20ac": "Euro Sign",
               "\r": "Carriage Return",
               "\ufb33": "Hebrew Letter Dalet With Dagesh",
               "1": "One",
               "\ud83d\ude00": "Emoji: Grinning Face",
               "\u0080": "Control",
               "\u00f6": "Latin Small Letter O With Diaeresis"
             }"#,
        )
        .expect("the RFC's input parses as JSON");

        let canonical = serde_json_canonicalizer::to_string(&input).expect("canonicalizes");
        let order: Vec<&str> = [
            "Carriage Return",
            "One",
            "Control",
            "Latin Small Letter O With Diaeresis",
            "Euro Sign",
            "Emoji: Grinning Face",
            "Hebrew Letter Dalet With Dagesh",
        ]
        .into_iter()
        .collect();

        let mut cursor = 0usize;
        for value in &order {
            let at = canonical[cursor..]
                .find(value)
                .unwrap_or_else(|| panic!("{value} missing or out of order in {canonical}"));
            cursor += at + value.len();
        }
    }

    /// RFC 8785 Appendix B, Table 1: IEEE 754 bit patterns and the ECMAScript
    /// number text JCS requires for each. `NaN` and the infinities are omitted
    /// because JSON cannot carry them (the RFC requires a canonicalizer to
    /// refuse them, which `serde_json` enforces one layer earlier by refusing to
    /// build the `Value`).
    #[test]
    fn rfc_8785_appendix_b_number_serialization_samples() {
        const SAMPLES: &[(u64, &str)] = &[
            (0x0000000000000000, "0"),
            (0x8000000000000000, "0"),
            (0x0000000000000001, "5e-324"),
            (0x8000000000000001, "-5e-324"),
            (0x7fefffffffffffff, "1.7976931348623157e+308"),
            (0xffefffffffffffff, "-1.7976931348623157e+308"),
            (0x4340000000000000, "9007199254740992"),
            (0xc340000000000000, "-9007199254740992"),
            (0x4430000000000000, "295147905179352830000"),
            (0x44b52d02c7e14af5, "9.999999999999997e+22"),
            (0x44b52d02c7e14af6, "1e+23"),
            (0x44b52d02c7e14af7, "1.0000000000000001e+23"),
            (0x444b1ae4d6e2ef4e, "999999999999999700000"),
            (0x444b1ae4d6e2ef4f, "999999999999999900000"),
            (0x444b1ae4d6e2ef50, "1e+21"),
            (0x3eb0c6f7a0b5ed8c, "9.999999999999997e-7"),
            (0x3eb0c6f7a0b5ed8d, "0.000001"),
            (0x41b3de4355555553, "333333333.3333332"),
            (0x41b3de4355555554, "333333333.33333325"),
            (0x41b3de4355555555, "333333333.3333333"),
            (0x41b3de4355555556, "333333333.3333334"),
            (0x41b3de4355555557, "333333333.33333343"),
            (0xbecbf647612f3696, "-0.0000033333333333333333"),
            (0x43143ff3c1cb0959, "1424953923781206.2"),
        ];

        for (bits, expected) in SAMPLES {
            let value = Value::from(f64::from_bits(*bits));
            let canonical = serde_json_canonicalizer::to_string(&json!({ "n": value }))
                .unwrap_or_else(|error| panic!("{bits:#018x} could not canonicalize: {error}"));
            assert_eq!(
                canonical,
                format!("{{\"n\":{expected}}}"),
                "RFC 8785 Appendix B pins {bits:#018x} as {expected}"
            );
        }
    }

    // -- attestation ---------------------------------------------------------

    #[test]
    fn a_signed_record_verifies_against_its_own_key() {
        let attestation = sign_record(
            &record(),
            &SEED,
            "cep-signing-key-2026-07",
            "provider_example",
            "2026-07-29T14:00:05Z",
        )
        .unwrap();
        let key = public_key_for(&SEED);
        assert_eq!(
            verify_record_attestation(&record(), &attestation, &key).unwrap(),
            AttestationVerdict::Valid
        );
        assert!(attestation.uses_known_algorithm());
        assert!(attestation.has_well_formed_issued_at());
        assert_eq!(
            attestation.signed_record_hash,
            record_hash(&record()).unwrap()
        );
    }

    #[test]
    fn editing_a_record_after_signing_is_caught_as_a_mismatch() {
        let attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        let mut tampered = record();
        tampered["statement"] = json!("the api handler never retries");
        let key = public_key_for(&SEED);
        let verdict = verify_record_attestation(&tampered, &attestation, &key).unwrap();
        assert!(
            matches!(verdict, AttestationVerdict::CommitmentMismatch { .. }),
            "expected a mismatch, got {verdict:?}"
        );
        assert!(!verdict.is_valid());
    }

    #[test]
    fn rewriting_the_stored_hash_does_not_launder_a_tampered_record() {
        // The attack the recompute exists for: edit the content, then rewrite
        // `record_hash` so the record is internally consistent again. Verifying
        // against the *stored* member would pass; verifying against the
        // recomputed one cannot.
        let attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        let mut laundered = record();
        laundered["statement"] = json!("the api handler never retries");
        let restated = record_hash(&laundered).unwrap();
        laundered[RECORD_HASH_MEMBER] = json!(restated);
        assert!(record_hash_is_current(&laundered).unwrap());

        let key = public_key_for(&SEED);
        assert!(matches!(
            verify_record_attestation(&laundered, &attestation, &key).unwrap(),
            AttestationVerdict::CommitmentMismatch { .. }
        ));
    }

    #[test]
    fn a_wrong_key_is_a_bad_signature_not_a_mismatch() {
        let attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        let other = public_key_for(&[3u8; 32]);
        assert_eq!(
            verify_record_attestation(&record(), &attestation, &other).unwrap(),
            AttestationVerdict::BadSignature,
            "the hash is intact; only the key is wrong"
        );
    }

    #[test]
    fn a_frame_signature_over_the_same_digest_is_not_a_record_attestation() {
        // What the domain tag buys. Hand the frame layer the record's own hash
        // bytes as a commitment and sign them: the resulting signature is over
        // 32 bytes that `signed_record_hash` names exactly, and it must still
        // not verify as a record attestation.
        let hash = record_hash(&record()).unwrap();
        let raw = raw_digest(&hash).unwrap();
        let frame_signed = sign_commitment(&raw, &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z");
        let lifted = RecordAttestation::new(
            hash,
            frame_signed.key_id,
            frame_signed.algorithm,
            frame_signed.attester_id,
            frame_signed.signature,
            frame_signed.issued_at,
        );
        let key = public_key_for(&SEED);
        assert_eq!(
            verify_record_attestation(&record(), &lifted, &key).unwrap(),
            AttestationVerdict::BadSignature,
            "a signature from another layer must not be presentable as a record attestation"
        );
    }

    #[test]
    fn an_unknown_algorithm_is_declined_rather_than_failed() {
        let mut attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        attestation.algorithm = "dilithium3".into();
        let key = public_key_for(&SEED);
        let verdict = verify_record_attestation(&record(), &attestation, &key).unwrap();
        assert_eq!(
            verdict,
            AttestationVerdict::UnknownAlgorithm("dilithium3".into())
        );
        assert!(!verdict.is_valid(), "declining is still not accepting");
        assert!(!attestation.uses_known_algorithm());
    }

    #[test]
    fn malformed_keys_signatures_and_hashes_are_named_distinctly() {
        let attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        assert_eq!(
            verify_record_attestation(&record(), &attestation, &[0u8; 5]).unwrap(),
            AttestationVerdict::MalformedKey
        );

        let mut truncated = attestation.clone();
        truncated.signature = "abcd".into();
        assert_eq!(
            verify_record_attestation(&record(), &truncated, &public_key_for(&SEED)).unwrap(),
            AttestationVerdict::MalformedSignature
        );

        let mut bad_hash = attestation;
        bad_hash.signed_record_hash = "not-a-digest".into();
        assert_eq!(
            verify_record_attestation(&record(), &bad_hash, &public_key_for(&SEED)).unwrap(),
            AttestationVerdict::MalformedCommitment
        );

        assert_eq!(
            record_attestation_message("sha256:short"),
            Err(RecordHashError::MalformedDigest("sha256:short".into()))
        );
    }

    #[test]
    fn the_signed_message_is_the_domain_tag_then_the_raw_digest() {
        let hash = record_hash(&record()).unwrap();
        let message = record_attestation_message(&hash).unwrap();
        assert_eq!(message.len(), RECORD_ATTESTATION_DOMAIN.len() + 32);
        assert!(message.starts_with(RECORD_ATTESTATION_DOMAIN));
        assert_eq!(
            &message[RECORD_ATTESTATION_DOMAIN.len()..],
            &raw_digest(&hash).unwrap()
        );
        assert_eq!(crate::digest_string(&raw_digest(&hash).unwrap()), hash);
    }

    #[test]
    fn an_attestation_round_trips_through_json() {
        let attestation =
            sign_record(&record(), &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z").unwrap();
        let json = serde_json::to_string(&attestation).unwrap();
        let back: RecordAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, attestation);
    }

    #[test]
    fn a_detached_attestation_verifies_from_the_hash_alone() {
        // An auditor holding the hash and the signature, but not the record.
        let hash = record_hash(&record()).unwrap();
        let attestation =
            sign_record_attestation(&hash, &SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z")
                .unwrap();
        assert_eq!(
            verify_signed_record_hash(&hash, &attestation, &public_key_for(&SEED)),
            AttestationVerdict::Valid
        );
    }

    // -- key-id validity windows (profile LC3, ADR 0028) ---------------------

    /// A ring holding `SEED`'s key under `key-2026`, in service for 2026 only.
    fn ring_for_2026() -> RecordKeyRing {
        let mut ring = RecordKeyRing::new();
        ring.insert(
            RecordKey::unbounded("key-2026", public_key_for(&SEED).to_vec()).with_validity(
                KeyValidity::new(
                    Some("2026-01-01T00:00:00Z".into()),
                    Some("2026-12-31T23:59:59Z".into()),
                )
                .unwrap(),
            ),
        );
        ring
    }

    fn signed_under_2026_key() -> RecordAttestation {
        sign_record(
            &record(),
            &SEED,
            "key-2026",
            "provider_example",
            "2026-07-29T14:00:05Z",
        )
        .unwrap()
    }

    #[test]
    fn an_attestation_received_inside_its_keys_window_verifies() {
        let verdict = ring_for_2026()
            .verify_record(&record(), &signed_under_2026_key(), "2026-07-29T14:00:06Z")
            .unwrap();
        assert!(verdict.is_valid(), "{verdict:?}");
        assert_eq!(
            verdict,
            RecordKeyVerdict::Checked {
                key_id: "key-2026".into(),
                verdict: AttestationVerdict::Valid,
            }
        );
    }

    /// The witness #120 asked for: an attestation received after its key's
    /// window closed fails, and fails for that named reason — not as a bad
    /// signature, and not as a pass.
    #[test]
    fn an_attestation_received_outside_its_keys_window_fails_for_that_reason() {
        let ring = ring_for_2026();
        let attestation = signed_under_2026_key();

        let after = ring
            .verify_record(&record(), &attestation, "2027-03-01T00:00:00Z")
            .unwrap();
        assert_eq!(
            after,
            RecordKeyVerdict::KeyNotInService {
                key_id: "key-2026".into(),
                received_at: "2027-03-01T00:00:00Z".into(),
                position: WindowPosition::Expired,
            }
        );
        assert!(!after.is_valid());

        let before = ring
            .verify_record(&record(), &attestation, "2025-06-01T00:00:00Z")
            .unwrap();
        assert!(matches!(
            before,
            RecordKeyVerdict::KeyNotInService {
                position: WindowPosition::NotYetValid,
                ..
            }
        ));
    }

    /// The attack a window checked against `issued_at` would miss: the holder
    /// of a lapsed key signs new evidence and back-dates `issued_at` into the
    /// window. `issued_at` is not in the signed message, so nothing about the
    /// signature objects — and the ring never reads it, so the back-dating buys
    /// nothing.
    #[test]
    fn back_dating_issued_at_into_the_window_does_not_revive_a_lapsed_key() {
        let mut backdated = signed_under_2026_key();
        backdated.issued_at = "2026-02-01T00:00:00Z".into();
        let verdict = ring_for_2026()
            .verify_record(&record(), &backdated, "2027-03-01T00:00:00Z")
            .unwrap();
        assert!(matches!(
            verdict,
            RecordKeyVerdict::KeyNotInService {
                position: WindowPosition::Expired,
                ..
            }
        ));
    }

    /// The auditor-replay case. A key closed at the end of 2026 still verifies
    /// evidence the auditor's records show was received in 2026, replayed years
    /// later — the verifier passes the recorded receipt instant, not its clock.
    #[test]
    fn an_auditor_replaying_archived_evidence_passes_the_recorded_receipt_instant() {
        let ring = ring_for_2026();
        let hash = record_hash(&record()).unwrap();
        let attestation = signed_under_2026_key();
        let recorded_receipt = "2026-07-29T14:00:06Z";
        assert!(
            ring.verify_record_hash(&hash, &attestation, recorded_receipt)
                .is_valid()
        );
    }

    #[test]
    fn a_key_the_ring_does_not_hold_checks_nothing() {
        let mut attestation = signed_under_2026_key();
        attestation.key_id = "key-unknown".into();
        assert_eq!(
            ring_for_2026()
                .verify_record(&record(), &attestation, "2026-07-29T14:00:06Z")
                .unwrap(),
            RecordKeyVerdict::UnknownKey {
                key_id: "key-unknown".into()
            }
        );
    }

    #[test]
    fn a_key_with_no_window_behaves_as_before_windows_existed() {
        let mut ring = RecordKeyRing::new();
        ring.insert(RecordKey::unbounded(
            "key-2026",
            public_key_for(&SEED).to_vec(),
        ));
        // Any receipt instant — even a malformed one — is in service.
        for received_at in ["1999-01-01T00:00:00Z", "2999-01-01T00:00:00Z", "unknown"] {
            assert!(
                ring.verify_record(&record(), &signed_under_2026_key(), received_at)
                    .unwrap()
                    .is_valid(),
                "{received_at}"
            );
        }
    }

    #[test]
    fn a_window_that_cannot_be_evaluated_does_not_pass() {
        let ring = ring_for_2026();
        let verdict = ring
            .verify_record(&record(), &signed_under_2026_key(), "last tuesday")
            .unwrap();
        assert!(matches!(
            verdict,
            RecordKeyVerdict::KeyNotInService {
                position: WindowPosition::MalformedInstant,
                ..
            }
        ));
    }
}
