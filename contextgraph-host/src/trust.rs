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
//! # Attested means the key, not the name the attestation prints
//!
//! A signature covers the commitment and nothing else (`SPEC.md` §6.5.2, F18).
//! `attester_id` and `issued_at` sit outside the signed preimage: anyone who
//! relays an attestation can rewrite either and it still verifies. So an
//! [`AttestationState::Attested`] vouches for exactly one identity — the
//! `key_id` whose key, trusted by this operator for this provider, verified the
//! signature. The `attester_id` it also carries is echoed from the attestation
//! as its *unverified claim*, reachable through
//! [`AttestationState::unverified_attester_id`] under a name that says so, and
//! `issued_at` is not carried at all: this host makes no decision on it, and a
//! state that exposed it would invite one.
//!
//! # A key's validity window is evaluated when the evidence arrives
//!
//! A [`TrustedKey`] may carry a [`KeyValidity`] window — `not_before` and
//! `not_after`, both inclusive
//! ([ADR 0028](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0028-key-validity-windows-are-evaluated-at-receipt.md)).
//! The window is evaluated at **the instant this host received the answer**,
//! never at the attestation's `issued_at`, for the reason above: `issued_at` is
//! unsigned, so the holder of a lapsed key would simply write an in-window date.
//! A receipt instant is one the signer cannot choose, and it bounds the signing
//! time from above — nothing is received before it exists. A live fan-out reads
//! the host clock once per answer and records it on
//! [`ProviderOutcome::received_at`](crate::ProviderOutcome::received_at); an
//! auditor replaying archived evidence passes that recorded instant to
//! [`TrustStore::check_result_signed_as_at`], so evidence received while a key
//! was in service verifies forever and evidence received after it lapsed never
//! did. A key outside its window reads as
//! [`AttestationState::KeyNotInService`] — and, like every other state, never
//! removes the frame (F9). A key with no window behaves exactly as before.
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
    FrameId, InclusionProof, KeyValidity, MAX_INCLUSION_PATH_STEPS, ProvenanceAttestation,
    WindowPosition, frame_inclusion_under_verified_root, verify_commitment,
    verify_frame_attestation,
};
use serde::{Deserialize, Serialize};

use crate::consent::now_protocol_timestamp;
use crate::wire::AttesterKey;

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
    /// When this key is in service (ADR 0028). Evaluated at the instant the
    /// host received the evidence, never at the attestation's unsigned
    /// `issued_at`; an attestation received outside it reads as
    /// [`AttestationState::KeyNotInService`] and its frame is still served.
    ///
    /// Unbounded by default, and omitted from the serialized form when
    /// unbounded — so a key with no window behaves, and persists, exactly as a
    /// key did before windows existed.
    #[serde(default, skip_serializing_if = "KeyValidity::is_unbounded")]
    pub validity: KeyValidity,
    /// How this key came to be trusted: [`Configured`](TrustTier::Configured)
    /// by an operator (the default, and the only tier ADR 0016 adopted), or
    /// [`Pinned`](TrustTier::Pinned) on first use from the provider's own
    /// handshake (ADR 0030). A signature that verifies against a pinned key
    /// reads as [`AttestationState::Pinned`], never as
    /// [`Attested`](AttestationState::Attested).
    ///
    /// Omitted from the serialized form when configured, so a store written
    /// before tiers existed reads back identically.
    #[serde(default, skip_serializing_if = "TrustTier::is_configured")]
    pub tier: TrustTier,
}

/// How a [`TrustedKey`] came to be trusted — the two tiers of ADR 0030,
/// ordered so that `Pinned < Configured`.
///
/// The tiers answer different questions. A **configured** key is one a person
/// put in the trust store: a verification against it says "signed by a key
/// this operator chose to trust". A **pinned** key is one the host recorded the
/// first time the provider published it in `handshake_ack.attester_keys`: a
/// verification against it says only "signed by the same key this provider
/// used when this host first met it". That is **continuity, never identity** —
/// an attacker present at first contact is pinned and then trusted for as long
/// as the pin stands — and the host never presents the second as the first.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Recorded by the host on first use from the provider's handshake. Proves
    /// continuity with that first contact and nothing more.
    Pinned,
    /// Put in the store by the operator. The tier ADR 0016 makes the trust
    /// root.
    #[default]
    Configured,
}

impl TrustTier {
    /// Whether this is the operator-configured tier.
    pub fn is_configured(&self) -> bool {
        matches!(self, Self::Configured)
    }
}

/// The most keys one provider may have **pinned** at once
/// ([`TrustStore::pin`]). Configured keys do not count against it: they are
/// an operator's, and an operator is not the party being bounded.
pub const MAX_PINNED_KEYS_PER_PROVIDER: usize = 16;

/// What [`TrustStore::pin`] did with one published key (ADR 0030).
///
/// Two outcomes are **alarms** ([`is_alarm`](Self::is_alarm)):
/// [`KeyChanged`](Self::KeyChanged) and
/// [`ConflictsWithConfigured`](Self::ConflictsWithConfigured). Each means a
/// provider is now publishing different key material under a `key_id` this
/// host already holds — a rotation done wrong at best, and at worst exactly
/// the substitution pinning exists to catch. Neither changes the store. A host
/// that pins **must** surface them to a person; that is why this type is
/// `#[must_use]`.
#[must_use = "a changed or conflicting key is an alarm the caller has to surface"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOutcome {
    /// Nothing was held under this `key_id`; the key is now pinned.
    Pinned {
        /// The key pinned.
        key_id: String,
        /// Its fingerprint, for the log line or prompt that reports the pin.
        fingerprint: String,
    },
    /// The same key is already pinned. Nothing changed.
    AlreadyPinned {
        /// The key.
        key_id: String,
    },
    /// The operator already configured this exact key. Nothing changed, and
    /// the key keeps its configured tier.
    AlreadyConfigured {
        /// The key.
        key_id: String,
    },
    /// **Alarm.** A key is pinned under this `key_id` and the provider now
    /// publishes different bytes under it. The pin stands; replacing it is an
    /// operator act.
    KeyChanged {
        /// The `key_id` whose bytes changed.
        key_id: String,
        /// The fingerprint of the key this host pinned.
        pinned_fingerprint: String,
        /// The fingerprint of the key the provider now publishes.
        offered_fingerprint: String,
    },
    /// **Alarm.** The operator configured a key under this `key_id`, and the
    /// provider publishes different bytes under it. The configured key stands.
    ConflictsWithConfigured {
        /// The `key_id` in conflict.
        key_id: String,
        /// The fingerprint of the operator's key.
        configured_fingerprint: String,
        /// The fingerprint of the key the provider publishes.
        offered_fingerprint: String,
    },
    /// The key could not be pinned.
    Refused {
        /// The `key_id` as published (truncated if oversized; empty if empty).
        key_id: String,
        /// Why.
        reason: PinRefusal,
    },
}

impl PinOutcome {
    /// Whether this outcome is one a person must hear about:
    /// [`KeyChanged`](Self::KeyChanged) or
    /// [`ConflictsWithConfigured`](Self::ConflictsWithConfigured).
    pub fn is_alarm(&self) -> bool {
        matches!(
            self,
            Self::KeyChanged { .. } | Self::ConflictsWithConfigured { .. }
        )
    }

    /// The `key_id` this outcome is about.
    pub fn key_id(&self) -> &str {
        match self {
            Self::Pinned { key_id, .. }
            | Self::AlreadyPinned { key_id }
            | Self::AlreadyConfigured { key_id }
            | Self::KeyChanged { key_id, .. }
            | Self::ConflictsWithConfigured { key_id, .. }
            | Self::Refused { key_id, .. } => key_id,
        }
    }
}

/// Why [`TrustStore::pin`] refused a published key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinRefusal {
    /// The `key_id` is empty.
    EmptyKeyId,
    /// The `key_id` is longer than this host echoes into an audit record.
    OversizedKeyId,
    /// A scheme this build cannot verify with (F8); the named scheme, truncated.
    UnsupportedAlgorithm(String),
    /// Not a 32-byte Ed25519 key as 64 hex characters.
    MalformedKey,
    /// The provider already has [`MAX_PINNED_KEYS_PER_PROVIDER`] pinned keys,
    /// or published more than that in one handshake.
    TooManyKeys,
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
            validity: KeyValidity::unbounded(),
            tier: TrustTier::Configured,
        })
    }

    /// A trusted Ed25519 key from raw public-key bytes — the form
    /// [`contextgraph_types::public_key_for`] returns.
    pub fn ed25519_bytes(key_id: impl Into<String>, public_key: &[u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            public_key: encode_hex(public_key),
            validity: KeyValidity::unbounded(),
            tier: TrustTier::Configured,
        }
    }

    /// The same key, in service only within `validity` (ADR 0028).
    ///
    /// Build the window with [`KeyValidity::new`], which refuses a malformed or
    /// inverted one where a person is reading the error. A window assembled
    /// around it is still checked each time it is evaluated and fails closed:
    /// the key then reads as [`AttestationState::KeyNotInService`] with
    /// [`WindowPosition::MalformedWindow`].
    ///
    /// ```
    /// use contextgraph_host::TrustedKey;
    /// use contextgraph_types::KeyValidity;
    ///
    /// let key = TrustedKey::ed25519_bytes("docs-2026", &[7u8; 32]).with_validity(
    ///     KeyValidity::new(
    ///         Some("2026-01-01T00:00:00Z".into()),
    ///         Some("2026-12-31T23:59:59Z".into()),
    ///     )
    ///     .expect("a well-formed window"),
    /// );
    /// assert!(!key.validity.is_unbounded());
    /// ```
    pub fn with_validity(mut self, validity: KeyValidity) -> Self {
        self.validity = validity;
        self
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
/// machine. Nothing populates it implicitly — there is no discovery and no
/// fetching. Trust-on-first-use exists only as an explicit call,
/// [`pin`](Self::pin), into a labelled tier strictly below a configured key
/// (ADR 0030). An empty store is a host that verifies nothing and loses
/// nothing, which is the default posture.
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

    /// Pin one key a provider published in its handshake
    /// (`handshake_ack.attester_keys`) — the trust-on-first-use tier of
    /// [ADR 0030](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0030-a-pinned-trust-tier-below-configured.md).
    ///
    /// `local_id` is the operator's id for the provider, the one trust is keyed
    /// on, so a provider can never pin a key under another provider's name.
    ///
    /// A key is pinned only when nothing is held under its `key_id` for this
    /// provider. Nothing is ever replaced here:
    ///
    /// - the same bytes already held → [`AlreadyPinned`](PinOutcome::AlreadyPinned)
    ///   or [`AlreadyConfigured`](PinOutcome::AlreadyConfigured), and nothing
    ///   changes;
    /// - **different bytes under a pinned `key_id`** →
    ///   [`KeyChanged`](PinOutcome::KeyChanged), an alarm, and the pin stands.
    ///   Rotation is a new `key_id`, never a reused one (`SPEC.md` §6.5), so a
    ///   key that changes under the same id is exactly what pinning exists to
    ///   notice — and silently re-pinning it would hand the pin to whoever
    ///   changed it. Replacing a pin is an operator act:
    ///   [`revoke`](Self::revoke) it, or [`trust`](Self::trust) a configured
    ///   key.
    /// - different bytes under a *configured* `key_id` →
    ///   [`ConflictsWithConfigured`](PinOutcome::ConflictsWithConfigured), an
    ///   alarm, and the operator's key stands. A pin never outranks a person.
    ///
    /// A key this host cannot verify with (another algorithm, malformed hex,
    /// an empty or oversized id) is [`Refused`](PinOutcome::Refused), as is a
    /// provider that already has [`MAX_PINNED_KEYS_PER_PROVIDER`] pinned keys:
    /// the handshake is provider-controlled, and pinning must not let it grow
    /// the store without bound.
    pub fn pin(&mut self, local_id: &str, offered: &AttesterKey) -> PinOutcome {
        if offered.key_id.is_empty() {
            return PinOutcome::Refused {
                key_id: String::new(),
                reason: PinRefusal::EmptyKeyId,
            };
        }
        if offered.key_id.len() > MAX_ECHOED_IDENTIFIER {
            return PinOutcome::Refused {
                key_id: echoed(&offered.key_id),
                reason: PinRefusal::OversizedKeyId,
            };
        }
        if offered.algorithm != ALGORITHM_ED25519 {
            return PinOutcome::Refused {
                key_id: offered.key_id.clone(),
                reason: PinRefusal::UnsupportedAlgorithm(echoed(&offered.algorithm)),
            };
        }
        let Some(candidate) = TrustedKey::ed25519_hex(
            offered.key_id.clone(),
            offered.public_key.to_ascii_lowercase(),
        ) else {
            return PinOutcome::Refused {
                key_id: offered.key_id.clone(),
                reason: PinRefusal::MalformedKey,
            };
        };
        // `ed25519_hex` accepted it, so it decodes and has a fingerprint.
        let offered_fingerprint = candidate.fingerprint().unwrap_or_default();
        let key_id = offered.key_id.clone();

        if let Some(held) = self.key(local_id, &offered.key_id) {
            // A held key whose hex does not decode has no fingerprint, and so
            // matches nothing: it is reported, never quietly overwritten.
            let held_fingerprint = held.fingerprint().unwrap_or_default();
            let same = held_fingerprint == offered_fingerprint;
            return match (held.tier, same) {
                (TrustTier::Configured, true) => PinOutcome::AlreadyConfigured { key_id },
                (TrustTier::Pinned, true) => PinOutcome::AlreadyPinned { key_id },
                (TrustTier::Configured, false) => PinOutcome::ConflictsWithConfigured {
                    key_id,
                    configured_fingerprint: held_fingerprint,
                    offered_fingerprint,
                },
                (TrustTier::Pinned, false) => PinOutcome::KeyChanged {
                    key_id,
                    pinned_fingerprint: held_fingerprint,
                    offered_fingerprint,
                },
            };
        }

        let pinned = self
            .keys_for(local_id)
            .filter(|key| key.tier == TrustTier::Pinned)
            .count();
        if pinned >= MAX_PINNED_KEYS_PER_PROVIDER {
            return PinOutcome::Refused {
                key_id,
                reason: PinRefusal::TooManyKeys,
            };
        }
        self.trust(
            local_id,
            TrustedKey {
                tier: TrustTier::Pinned,
                ..candidate
            },
        );
        PinOutcome::Pinned {
            key_id,
            fingerprint: offered_fingerprint,
        }
    }

    /// [`pin`](Self::pin) every key a provider published, in order, and report
    /// each outcome.
    ///
    /// At most [`MAX_PINNED_KEYS_PER_PROVIDER`] entries are considered. A
    /// handshake publishing more gets one further
    /// [`Refused`](PinOutcome::Refused) with
    /// [`TooManyKeys`](PinRefusal::TooManyKeys), naming the first key it
    /// ignored, so the work a provider can buy is bounded however long the
    /// list it sends.
    pub fn pin_all(&mut self, local_id: &str, offered: &[AttesterKey]) -> Vec<PinOutcome> {
        let mut outcomes: Vec<PinOutcome> = offered
            .iter()
            .take(MAX_PINNED_KEYS_PER_PROVIDER)
            .map(|key| self.pin(local_id, key))
            .collect();
        if let Some(first_ignored) = offered.get(MAX_PINNED_KEYS_PER_PROVIDER) {
            outcomes.push(PinOutcome::Refused {
                key_id: echoed(&first_ignored.key_id),
                reason: PinRefusal::TooManyKeys,
            });
        }
        outcomes
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

    /// Every provider this store trusts at least one key for, in id order —
    /// what a host renders when it shows an operator what they have trusted,
    /// and what [`to_trust_file_json`](Self::to_trust_file_json) walks.
    pub fn providers(&self) -> impl Iterator<Item = &str> {
        let mut ids: Vec<&str> = self
            .keys
            .iter()
            .filter(|(_, keys)| !keys.is_empty())
            .map(|(id, _)| id.as_str())
            .collect();
        ids.sort_unstable();
        ids.into_iter()
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
        self.check_signed_as(provider_id, provider_id, frame, attestation)
    }

    /// [`check`](Self::check) with the trust-lookup id and the signing id told
    /// apart. See [`check_result_signed_as`](Self::check_result_signed_as) for
    /// why a host needs both: `local_id` decides *whose key may sign this*, and
    /// `signing_id` — the handshake-declared `provider.name` — decides *what
    /// bytes were signed* (`SPEC.md` §6.5.2).
    ///
    /// Key validity windows are evaluated at the host's clock now, which is
    /// right for an attestation that has just arrived. Replaying one received
    /// earlier is [`check_signed_as_at`](Self::check_signed_as_at).
    pub fn check_signed_as(
        &self,
        local_id: &str,
        signing_id: &str,
        frame: &ContextFrame,
        attestation: &ProvenanceAttestation,
    ) -> AttestationState {
        self.check_signed_as_at(
            local_id,
            signing_id,
            frame,
            attestation,
            &now_protocol_timestamp(),
        )
    }

    /// [`check_signed_as`](Self::check_signed_as) as of `received_at`: the
    /// instant this host **received** the attestation, at which the trusted
    /// key's [`KeyValidity`] window is evaluated (ADR 0028).
    ///
    /// This is the auditor-replay door. Evidence archived with the instant it
    /// arrived is re-checked against that instant, so a key that has since
    /// lapsed still vouches for what it signed while in service — and a key
    /// that had lapsed on arrival never does, whatever the attestation's
    /// unsigned `issued_at` claims. `received_at` is the verifier's own record,
    /// never a value read off the attestation.
    pub fn check_signed_as_at(
        &self,
        local_id: &str,
        signing_id: &str,
        frame: &ContextFrame,
        attestation: &ProvenanceAttestation,
        received_at: &str,
    ) -> AttestationState {
        // F8, first: a scheme this build cannot check is *uncheckable*, which is
        // a different finding from invalid and is not improved by holding a key.
        if attestation.algorithm != ALGORITHM_ED25519 {
            return AttestationState::UnknownAlgorithm {
                algorithm: echoed(&attestation.algorithm),
            };
        }

        // No key in service ⇒ no signature check. This is also the bound that
        // keeps an unknown peer from spending the host's CPU: reaching the
        // verifier at all requires an operator to have trusted a key under this
        // exact id, in service at the instant the evidence arrived.
        let key = match self.key_in_service(local_id, &attestation.key_id, received_at) {
            Ok(key) => key,
            Err(state) => return state,
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
        // The commitment is recomputed with `signing_id`, never `local_id`: the
        // preimage §6.5.2 defines contains the name the provider declared, which
        // is the only provider identifier both ends of the wire observe.
        match verify_frame_attestation(signing_id, frame, attestation, &public_key) {
            verdict @ (AttestationVerdict::Valid | AttestationVerdict::ValidIdentityOnly) => {
                verified(key.tier, attestation, verdict.binds_content())
            }
            verdict => AttestationState::Invalid { verdict },
        }
    }

    /// The key trusted for `(local_id, key_id)` if it is in service at
    /// `received_at`; otherwise the state that says why not.
    ///
    /// The one place a key is resolved for verification, so the lookup and the
    /// window can never be consulted in different orders on different paths.
    /// The window is evaluated only for a key the operator actually trusts: an
    /// unknown `key_id` is [`AttestationState::NoTrustedKey`] and the clock is
    /// never read for it.
    fn key_in_service(
        &self,
        local_id: &str,
        key_id: &str,
        received_at: &str,
    ) -> Result<&TrustedKey, AttestationState> {
        let Some(key) = self.key(local_id, key_id) else {
            return Err(AttestationState::NoTrustedKey {
                key_id: echoed(key_id),
            });
        };
        match key.validity.position(received_at) {
            WindowPosition::Within => Ok(key),
            position => Err(AttestationState::KeyNotInService {
                key_id: key.key_id.clone(),
                received_at: echoed(received_at),
                position,
                validity: key.validity.clone(),
            }),
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
        self.check_result_signed_as(provider_id, provider_id, result)
    }

    /// [`check_result`](Self::check_result) with the two provider identities
    /// told apart: `local_id` is the host's own key for this provider, and
    /// `signing_id` is the name the provider signs under.
    ///
    /// # Why there are two
    ///
    /// `SPEC.md` §6.5.2 puts the provider id inside the signed preimage, and is
    /// explicit about *which* id: the handshake-declared `provider.name`, because
    /// a host's local id "is not a string the provider ever sees — so it is not
    /// one a provider could sign against". A host that recomputes the commitment
    /// with its own local id gets a different preimage and therefore a different
    /// digest, and reports [`AttestationVerdict::CommitmentMismatch`] — the
    /// verdict §6.5.4 reserves for *a frame that changed after signing*. An
    /// operator who merely named the provider something else in their config
    /// would be handed a tampering incident over honest evidence.
    ///
    /// The two ids cannot be collapsed in the other direction either. Trust is
    /// keyed on `local_id` because that is the id the *operator* chose, in the
    /// same act as the consent grant; keying it on the declared name would let a
    /// provider claim another's trusted key by declaring its name, which is the
    /// substitution the identity binding exists to prevent. So the local id
    /// answers "whose key may sign this?" and the declared name answers "what
    /// bytes were signed?" — different questions with different right answers.
    ///
    /// # The same split decides matching, not only verifying
    ///
    /// A provider has no notion of this host's local routing id, so every
    /// [`FrameAttestation::frame`] it puts on the wire is built from the same
    /// name it signs under (the reference provider builds both from its one
    /// `attestation_provider_id()`). Matching `offered` against
    /// `frame.identity(local_id)` would therefore miss every entry whenever an
    /// operator's config id differs from the provider's declared name — the
    /// same false negative this split exists to close, one layer above the
    /// signature check itself. The lookup key is `frame.identity(signing_id)`
    /// for that reason.
    ///
    /// The **returned** [`FrameId`]s are keyed by `local_id`, not
    /// `signing_id`: [`FrameId::provider_id`] is documented as "the same
    /// routing/consent key the host registered it under", and it is the id
    /// the rest of the composition path (`compose.rs`, `usage_report`) already
    /// builds its own `FrameId`s from. Echoing `signing_id` here instead would
    /// desynchronize this ledger from every other identity the host emits for
    /// the same frame.
    ///
    /// Key validity windows are evaluated at the host's clock now — the
    /// instant a result that has just arrived was received. A caller that
    /// recorded the receipt instant itself, or replays archived evidence, uses
    /// [`check_result_signed_as_at`](Self::check_result_signed_as_at).
    pub fn check_result_signed_as(
        &self,
        local_id: &str,
        signing_id: &str,
        result: &ContextQueryResult,
    ) -> Vec<FrameAttestationOutcome> {
        self.check_result_signed_as_at(local_id, signing_id, result, &now_protocol_timestamp())
    }

    /// [`check_result_signed_as`](Self::check_result_signed_as) as of
    /// `received_at`, the instant this host received `result` (ADR 0028).
    ///
    /// One instant for the whole result: every attestation in one answer
    /// arrived together, so every key it names is judged at the same moment,
    /// and an auditor replaying the answer reproduces every state exactly by
    /// passing the instant the host recorded
    /// ([`ProviderOutcome::received_at`](crate::ProviderOutcome::received_at)).
    pub fn check_result_signed_as_at(
        &self,
        local_id: &str,
        signing_id: &str,
        result: &ContextQueryResult,
        received_at: &str,
    ) -> Vec<FrameAttestationOutcome> {
        let mut offered: HashMap<&FrameId, &FrameAttestation> = HashMap::new();
        for entry in result.frame_attestations.iter().take(result.frames.len()) {
            // First entry wins: a flood of duplicates for one frame cannot
            // multiply the verification work.
            offered.entry(&entry.frame).or_insert(entry);
        }

        // The answer-level root signature is checked at most once for the
        // whole result, and only if some entry actually leans on it: `None`
        // until the first proof-only entry asks, then the one verdict every
        // later entry reuses. This is the bound that makes a signed root worth
        // having — n frames cost one signature verification plus n proof walks,
        // never n signature verifications (#133).
        let mut root_check: Option<RootCheck> = None;
        let answer = ResultContext {
            local_id,
            signing_id,
            received_at,
            root: result.result_attestation.as_ref(),
            leaf_count: result.frames.len(),
        };

        result
            .frames
            .iter()
            .map(|frame| {
                // Match on the whole identity triple, never on the frame id
                // alone: two frames sharing an id but not a digest are
                // different bytes, and handing the first one's evidence to the
                // second is the substitution the binding exists to prevent
                // (`SPEC.md` §6.5.2). Built from `signing_id` — see this
                // method's doc comment for why.
                let signing_identity = frame.identity(signing_id);
                let state = match offered.get(&signing_identity) {
                    Some(entry) => self.check_entry(&answer, frame, entry, &mut root_check),
                    None => AttestationState::Unattested,
                };
                FrameAttestationOutcome {
                    // Built from `local_id`, deliberately different from the
                    // lookup key above — see this method's doc comment.
                    frame: frame.identity(local_id),
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
        answer: &ResultContext<'_>,
        frame: &ContextFrame,
        entry: &FrameAttestation,
        root_check: &mut Option<RootCheck>,
    ) -> AttestationState {
        if let Some(attestation) = &entry.attestation {
            return self.check_signed_as_at(
                answer.local_id,
                answer.signing_id,
                frame,
                attestation,
                answer.received_at,
            );
        }
        match (&entry.inclusion_proof, answer.root) {
            (Some(proof), Some(root)) => {
                let checked = root_check.get_or_insert_with(|| {
                    self.check_root(answer.local_id, root, answer.received_at)
                });
                check_inclusion(answer, frame, proof, root, checked)
            }
            // A proof of membership of a root the answer never carried, or an
            // entry naming a frame and asserting nothing about it. Neither can
            // be turned into a check, and F9 makes that a degradation to
            // unattested rather than a reason to withhold the frame.
            _ => AttestationState::UnusableEvidence,
        }
    }

    /// Check the answer-level `result_attestation` — the signature over the
    /// §6.5.3 Merkle root — **once**, for every proof-only entry in the result
    /// to share.
    ///
    /// Ordered exactly as [`check`](Self::check) is, and for the same reason:
    /// the scheme, then the key (and its window), then the structural lengths,
    /// and only then any cryptography. The key lookup in particular is the
    /// bound that keeps an untrusted peer from spending this host's CPU:
    /// reaching the signature check, or any proof walk, requires an operator
    /// to have trusted a key under the root's exact `key_id`, in service when
    /// the answer arrived.
    ///
    /// A failure here is every proof-only frame's state: a root that does not
    /// verify attests nothing, and F9 serves each of those frames regardless.
    fn check_root(
        &self,
        local_id: &str,
        root: &ProvenanceAttestation,
        received_at: &str,
    ) -> RootCheck {
        if root.algorithm != ALGORITHM_ED25519 {
            return RootCheck::Failed(AttestationState::UnknownAlgorithm {
                algorithm: echoed(&root.algorithm),
            });
        }
        let key = match self.key_in_service(local_id, &root.key_id, received_at) {
            Ok(key) => key,
            Err(state) => return RootCheck::Failed(state),
        };
        let invalid = |verdict| RootCheck::Failed(AttestationState::Invalid { verdict });
        if root.signed_commitment.len() != COMMITMENT_LEN {
            return invalid(AttestationVerdict::MalformedCommitment);
        }
        if root.signature.len() != ED25519_SIGNATURE_HEX_LEN {
            return invalid(AttestationVerdict::MalformedSignature);
        }
        let Some(public_key) = decode_hex(&key.public_key) else {
            return invalid(AttestationVerdict::MalformedKey);
        };
        // The root's own bytes. `verify_commitment` below re-parses
        // `signed_commitment` under the protocol's lowercase-only grammar and
        // compares it with these, so a spelling this lenient decode accepts
        // and the grammar does not still ends as `MalformedCommitment`.
        let Some(signed_root) = root
            .signed_commitment
            .strip_prefix("sha256:")
            .and_then(decode_hex)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        else {
            return invalid(AttestationVerdict::MalformedCommitment);
        };

        note_root_signature_check();
        match verify_commitment(&signed_root, root, &public_key) {
            AttestationVerdict::Valid => RootCheck::Verified {
                root: signed_root,
                tier: key.tier,
            },
            verdict => invalid(verdict),
        }
    }
}

/// What checking a result's `result_attestation` found — computed at most once
/// per result by [`TrustStore::check_result_signed_as_at`] and shared by every
/// proof-only entry in it.
enum RootCheck {
    /// The root's signature verified against a key this host trusts, in
    /// service when the answer arrived.
    Verified {
        /// The signed root's 32 bytes, which each proof must recompute.
        root: [u8; 32],
        /// The tier of the key that verified it.
        tier: TrustTier,
    },
    /// The root did not verify, or could not be checked. This state is every
    /// proof-only frame's state.
    Failed(AttestationState),
}

/// The per-result facts every entry's check reads, gathered once so the
/// per-entry functions take one argument rather than six.
struct ResultContext<'a> {
    /// The operator's id for the provider: what trust is keyed on.
    local_id: &'a str,
    /// The provider's declared name: what the commitments were built under.
    signing_id: &'a str,
    /// The instant the answer was received, for key validity windows.
    received_at: &'a str,
    /// The answer-level root signature, if the answer carried one.
    root: Option<&'a ProvenanceAttestation>,
    /// How many frames the answer carries — the only `leaf_count` a proof of
    /// membership of its root may state (F12).
    leaf_count: usize,
}

/// Check one frame's membership of the signed result-set root
/// (`SPEC.md` §6.5.3, F13), given the root's already-computed [`RootCheck`].
///
/// Everything that can be refused without hashing is refused first, in this
/// order: a root that did not verify, a proof whose `leaf_count` is not the
/// number of frames the answer carries (F12: the root is over *exactly* those
/// frames), a path longer than [`MAX_INCLUSION_PATH_STEPS`], and a path whose
/// length or sides are not the RFC 6962 shape for its
/// `(leaf_index, leaf_count)` ([`InclusionProof::is_well_shaped`]). Only a
/// proof that passes all of them costs a commitment and a walk of at most
/// `ceil(log2(leaf_count))` hashes.
///
/// `signing_id` recomputes the leaf, not `local_id`, for the reason
/// [`TrustStore::check_signed_as`] gives: the root was built over commitments
/// the provider made under its own declared name.
fn check_inclusion(
    answer: &ResultContext<'_>,
    frame: &ContextFrame,
    proof: &InclusionProof,
    root: &ProvenanceAttestation,
    checked: &RootCheck,
) -> AttestationState {
    let (signed_root, tier) = match checked {
        RootCheck::Verified { root, tier } => (root, *tier),
        RootCheck::Failed(state) => return state.clone(),
    };
    if proof.leaf_count != answer.leaf_count
        || proof.path.len() > MAX_INCLUSION_PATH_STEPS
        || !proof.is_well_shaped()
    {
        return AttestationState::Invalid {
            verdict: AttestationVerdict::MalformedCommitment,
        };
    }

    // `covers_content` is read off the verdict rather than re-derived, for the
    // reason `check` gives: the rule for what a commitment binds lives in
    // `contextgraph_types::attest`, and this stays downstream of it (#128).
    match frame_inclusion_under_verified_root(answer.signing_id, frame, proof, signed_root) {
        verdict @ (AttestationVerdict::Valid | AttestationVerdict::ValidIdentityOnly) => {
            verified(tier, root, verdict.binds_content())
        }
        verdict => AttestationState::Invalid { verdict },
    }
}

// The documentation sits inside the macro: a doc comment on a macro
// invocation documents nothing, and rustc's `unused_doc_comments` lint says so.
#[cfg(test)]
thread_local! {
    /// How many times this thread has checked a result-set root's signature —
    /// the witness for the once-per-result bound (#133). Test builds only, and
    /// per-thread so parallel tests cannot see each other's counts.
    static ROOT_SIGNATURE_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one root signature check, in a test build; nothing otherwise.
#[cfg(test)]
fn note_root_signature_check() {
    ROOT_SIGNATURE_CHECKS.with(|checks| checks.set(checks.get() + 1));
}

/// Count one root signature check, in a test build; nothing otherwise.
#[cfg(not(test))]
fn note_root_signature_check() {}

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
    /// second host that holds no key (ADR 0016 Consequences). Nor does it mean
    /// anything about `attester_id` or `issued_at`, which the signature does
    /// not cover (`SPEC.md` §6.5.2, F18).
    Attested {
        /// The key that verified it — the one identity this state vouches for.
        /// `key_id` is not itself signed, but it is what selected the verifying
        /// key from this host's trust store, so a rewritten one verifies against
        /// nothing.
        key_id: String,
        /// The attesting authority the attestation **names**, echoed verbatim
        /// (truncated) and **unverified**.
        ///
        /// Outside the signed preimage (F18): anyone relaying the attestation
        /// can rewrite it to name any authority and this state is unchanged
        /// apart from the echo. It is the attestation's claim about who is
        /// accountable, never evidence of it — that answer is whoever this
        /// operator trusted `key_id` for. A host that renders it **SHOULD**
        /// mark it unverified; read it through
        /// [`unverified_attester_id`](AttestationState::unverified_attester_id),
        /// whose name carries the mark to every call site.
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
    /// The signature verified against a key this host **pinned on first use**
    /// — the [`TrustTier::Pinned`] tier of ADR 0030 — rather than one an
    /// operator configured.
    ///
    /// This proves **continuity, not identity**: the key that signed this is
    /// the key the provider published when this host first met it. It does not
    /// prove who that was — an attacker present at first contact is pinned too
    /// — so it is deliberately a separate variant from
    /// [`Attested`](Self::Attested), and [`is_attested`](Self::is_attested)
    /// is **false** here. Every existing reading of "attested" therefore keeps
    /// meaning "a key a person checked"; a host that wants to credit
    /// continuity asks for it by name, through
    /// [`signature_verified`](Self::signature_verified) or
    /// [`trust_tier`](Self::trust_tier), and renders it as the weaker claim it
    /// is.
    ///
    /// The fields mean what they mean on [`Attested`](Self::Attested):
    /// `attester_id` is the attestation's unverified claim (F18), and
    /// `covers_content` says whether the signature binds the frame's bytes.
    Pinned {
        /// The pinned key that verified it.
        key_id: String,
        /// The attesting authority the attestation names — unverified.
        attester_id: String,
        /// Whether the signature covers the frame's content bytes.
        covers_content: bool,
    },
    /// An attestation was offered, but this host holds no trusted key under
    /// that `key_id` for that provider. A configuration gap, **not** a forgery
    /// finding: the signature was never checked, so nothing is known about it.
    NoTrustedKey {
        /// The `key_id` the attestation named, so an operator knows what to add.
        key_id: String,
    },
    /// This host trusts a key under that `key_id` for that provider, and the
    /// key was **not in service** at the instant the evidence arrived: before
    /// its `not_before`, after its `not_after`, or under a window that could
    /// not be evaluated (ADR 0028). The signature was not checked — no key was
    /// in service to check it against.
    ///
    /// Distinct from [`NoTrustedKey`](Self::NoTrustedKey), which is a gap in
    /// the operator's configuration, and from [`Invalid`](Self::Invalid), which
    /// is a finding about the signature. This is a finding about *time*: the
    /// state key rotation exists to produce, and — for an `Expired` key that
    /// keeps signing — the one worth an operator's attention.
    ///
    /// Decided at the receipt instant, never at the attestation's `issued_at`,
    /// which is unsigned (F18) and which the holder of a lapsed key would
    /// simply back-date. F9 holds: the frame is served, and every decision
    /// treats this as unattested.
    KeyNotInService {
        /// The trusted key that was out of service.
        key_id: String,
        /// The instant the window was evaluated at — when this host received
        /// the evidence, or the recorded instant an auditor replayed it at.
        received_at: String,
        /// Where that instant fell relative to the key's window.
        position: WindowPosition,
        /// The window as this host holds it, so the audit says which bound was
        /// crossed without a second lookup.
        validity: KeyValidity,
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
    /// Whether this frame is attested by a key this host's **operator**
    /// trusts. Every other state — including [`NotChecked`](Self::NotChecked),
    /// and [`Pinned`](Self::Pinned) — is `false`, because "I could not check
    /// it" is never "it is good" (`SPEC.md` F8), and "the same key as last
    /// time" is not "a key a person checked" (ADR 0030).
    pub fn is_attested(&self) -> bool {
        matches!(self, Self::Attested { .. })
    }

    /// Whether the signature verified against a key this host holds, at
    /// **either** tier: [`Attested`](Self::Attested) or
    /// [`Pinned`](Self::Pinned). The question to ask on purpose when
    /// continuity is worth crediting; read [`trust_tier`](Self::trust_tier)
    /// beside it to say which claim is being made.
    pub fn signature_verified(&self) -> bool {
        matches!(self, Self::Attested { .. } | Self::Pinned { .. })
    }

    /// The tier of the key that verified the signature —
    /// [`TrustTier::Configured`] for [`Attested`](Self::Attested),
    /// [`TrustTier::Pinned`] for [`Pinned`](Self::Pinned) — and `None` for
    /// every state in which nothing verified.
    pub fn trust_tier(&self) -> Option<TrustTier> {
        match self {
            Self::Attested { .. } => Some(TrustTier::Configured),
            Self::Pinned { .. } => Some(TrustTier::Pinned),
            _ => None,
        }
    }

    /// Whether a verified signature covers the frame's content bytes as well
    /// as its identity and provenance — see
    /// [`Attested::covers_content`](Self::Attested). This is a fact about the
    /// signature, not about the key's tier, so it holds for
    /// [`Pinned`](Self::Pinned) as well as [`Attested`](Self::Attested); read
    /// [`trust_tier`](Self::trust_tier) for how far the key is trusted.
    /// `false` for every state in which no signature verified.
    pub fn covers_content(&self) -> bool {
        matches!(
            self,
            Self::Attested {
                covers_content: true,
                ..
            } | Self::Pinned {
                covers_content: true,
                ..
            }
        )
    }

    /// The `attester_id` an [`Attested`](Self::Attested) or
    /// [`Pinned`](Self::Pinned) attestation names — **unverified**, because
    /// the signature does not cover it (`SPEC.md` §6.5.2, F18). `None` for
    /// every other state.
    ///
    /// Named for what it is so a caller cannot surface it as signed by
    /// accident: render it beside [`Attested::key_id`](Self::Attested), which
    /// is what verified, and label it as the attestation's claim.
    pub fn unverified_attester_id(&self) -> Option<&str> {
        match self {
            Self::Attested { attester_id, .. } | Self::Pinned { attester_id, .. } => {
                Some(attester_id)
            }
            _ => None,
        }
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

/// The state for a signature that verified against a key of `tier`:
/// [`Attested`](AttestationState::Attested) for an operator-configured key,
/// [`Pinned`](AttestationState::Pinned) for one pinned on first use. The one
/// place a tier becomes a state, so no path can report a pinned key as
/// configured.
fn verified(
    tier: TrustTier,
    attestation: &ProvenanceAttestation,
    covers_content: bool,
) -> AttestationState {
    let key_id = attestation.key_id.clone();
    let attester_id = echoed(&attestation.attester_id);
    match tier {
        TrustTier::Configured => AttestationState::Attested {
            key_id,
            attester_id,
            covers_content,
        },
        TrustTier::Pinned => AttestationState::Pinned {
            key_id,
            attester_id,
            covers_content,
        },
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

    /// F18 at the host: `attester_id` and `issued_at` are outside the signed
    /// preimage, so rewriting them in transit leaves the frame attested — and
    /// the state vouches only for the trusted `key_id`, echoing the rewritten
    /// `attester_id` as the unverified claim it is. A rewritten `key_id`, by
    /// contrast, never verifies: it is what selects the key.
    #[test]
    fn unsigned_attestation_metadata_is_echoed_as_a_claim_never_verified() {
        let frame = frame("frm_1");
        let honest = signed(&frame, &SEED);

        let mut relayed = honest.clone();
        relayed.attester_id = "someone-else".into();
        relayed.issued_at = "2099-01-01T00:00:00Z".into();

        let store = store_trusting(&SEED);
        let state = store.check(PROVIDER, &frame, &relayed);
        assert_eq!(
            state,
            AttestationState::Attested {
                key_id: KEY_ID.to_string(),
                attester_id: "someone-else".to_string(),
                covers_content: true,
            },
            "the signature cannot tell: the rewritten metadata was never signed"
        );
        assert_eq!(state.unverified_attester_id(), Some("someone-else"));
        assert_eq!(AttestationState::Unattested.unverified_attester_id(), None);

        // `key_id` fails safe. Pointed at another key this host trusts for the
        // provider, the signature does not verify against it...
        let mut store_with_two = store.clone();
        store_with_two.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes("docs-other", &public_key_for(&OTHER_SEED)),
        );
        let mut rekeyed = honest.clone();
        rekeyed.key_id = "docs-other".into();
        assert_eq!(
            store_with_two.check(PROVIDER, &frame, &rekeyed),
            AttestationState::Invalid {
                verdict: AttestationVerdict::BadSignature
            }
        );
        // ...and pointed at a key it does not trust, nothing is checked at all.
        rekeyed.key_id = "nobody".into();
        assert!(matches!(
            store.check(PROVIDER, &frame, &rekeyed),
            AttestationState::NoTrustedKey { .. }
        ));
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

    /// **The result-level match survives the same split the single-attestation
    /// check does.** A provider builds `FrameAttestation.frame` from its own
    /// declared name — it has no other identity to build it from — so an
    /// operator whose local config id differs from that name must still see
    /// the evidence matched, not silently dropped to `Unattested`. This is
    /// `check_result_signed_as`'s half of #161's fix; the single-attestation
    /// half is
    /// `a_local_id_that_differs_from_the_declared_name_still_verifies` above.
    #[test]
    fn a_result_matches_evidence_built_under_the_declared_name_even_when_the_local_id_differs() {
        const DECLARED: &str = "acme-docs";
        const LOCAL: &str = "docs-1";
        let served = frame("frm_1");
        let attestation = sign_frame_attestation(
            DECLARED,
            &served,
            &SEED,
            KEY_ID,
            DECLARED,
            "2026-08-29T00:00:00Z",
        );
        // The wire identity is built from `DECLARED`, exactly as the reference
        // provider builds it — from its own `attestation_provider_id()`, which
        // is the only identity it has for itself.
        let result = ContextQueryResult {
            frame_attestations: vec![FrameAttestation::signed(
                served.identity(DECLARED),
                attestation,
            )],
            ..ContextQueryResult::unattested(vec![served.clone()], false, None)
        };

        let mut store = TrustStore::new();
        store.trust(
            LOCAL,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)),
        );

        let outcomes = store.check_result_signed_as(LOCAL, DECLARED, &result);
        assert_eq!(outcomes.len(), 1);
        assert!(
            outcomes[0].state.is_attested(),
            "matching must key on the declared name the provider actually used \
             on the wire, got {:?}",
            outcomes[0].state
        );
        assert_eq!(
            outcomes[0].frame,
            served.identity(LOCAL),
            "the returned FrameId is keyed by the operator's local id, matching \
             the rest of the composition path"
        );
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
        use contextgraph_types::InclusionStep;

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

    /// The bound #133 names: a signed root is verified **once per result**,
    /// however many frames lean on it, and only when a proof-only entry
    /// actually needs it. Counted, not asserted in a comment.
    #[test]
    fn a_result_set_root_signature_is_verified_once_per_result_not_once_per_frame() {
        let checks = || ROOT_SIGNATURE_CHECKS.with(|count| count.get());
        let frames: Vec<ContextFrame> = (0..12).map(|i| frame(&format!("frm_{i:02}"))).collect();
        let result = root_signed_result(frames.clone(), &SEED);

        let before = checks();
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &result);
        assert_eq!(outcomes.len(), 12);
        assert!(outcomes.iter().all(|outcome| outcome.state.is_attested()));
        assert_eq!(checks() - before, 1, "twelve frames, one signature check");

        // A forged root is also checked once, and its failure is every
        // proof-only frame's state — each still served (F9).
        let root = contextgraph_types::result_set_root(PROVIDER, &frames);
        let mut forged = result.clone();
        forged.result_attestation = Some(sign_commitment(
            &root,
            &OTHER_SEED,
            KEY_ID,
            "docs-provider",
            "2026-08-29T00:00:00Z",
        ));
        let before = checks();
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &forged);
        assert_eq!(checks() - before, 1);
        assert_eq!(
            outcomes.len(),
            12,
            "F9: every frame has a state and is kept"
        );
        assert!(outcomes.iter().all(|outcome| {
            outcome.state
                == AttestationState::Invalid {
                    verdict: AttestationVerdict::BadSignature,
                }
        }));

        // No trusted key: nothing is verified at all.
        let before = checks();
        let _ = TrustStore::new().check_result(PROVIDER, &result);
        assert_eq!(checks() - before, 0);

        // Per-frame signatures only: the root is never consulted.
        let per_frame = ContextQueryResult {
            frame_attestations: frames
                .iter()
                .map(|f| FrameAttestation::signed(f.identity(PROVIDER), signed(f, &SEED)))
                .collect(),
            ..ContextQueryResult::unattested(frames.clone(), false, None)
        };
        let before = checks();
        let _ = store_trusting(&SEED).check_result(PROVIDER, &per_frame);
        assert_eq!(checks() - before, 0);
    }

    /// `leaf_count` is honored: F12 puts exactly the answer's frames under
    /// the root, so a proof stating any other tree size is refused on that
    /// fact, before a hash is spent — and so is a proof whose path does not
    /// have the shape its own `(leaf_index, leaf_count)` dictates.
    #[test]
    fn a_proof_from_a_differently_shaped_tree_is_refused_before_hashing() {
        let frames = vec![frame("frm_1"), frame("frm_2"), frame("frm_3")];
        let mut resized = root_signed_result(frames.clone(), &SEED);
        let proof = resized.frame_attestations[0]
            .inclusion_proof
            .as_mut()
            .expect("a proof");
        proof.leaf_count = 4;
        let outcomes = store_trusting(&SEED).check_result(PROVIDER, &resized);
        assert_eq!(
            outcomes[0].state,
            AttestationState::Invalid {
                verdict: AttestationVerdict::MalformedCommitment
            }
        );
        assert!(
            outcomes[1].state.is_attested(),
            "its siblings are unaffected"
        );

        let mut reshaped = root_signed_result(frames, &SEED);
        let proof = reshaped.frame_attestations[2]
            .inclusion_proof
            .as_mut()
            .expect("a proof");
        proof.path[0].sibling_is_left = !proof.path[0].sibling_is_left;
        assert_eq!(
            store_trusting(&SEED).check_result(PROVIDER, &reshaped)[2].state,
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
                public_key: "zz".repeat(32),
                ..TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED))
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
            public_key: key.public_key.to_uppercase(),
            ..key.clone()
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

    /// **An operator's choice of local name must not read as tampering.**
    ///
    /// The provider signs over its handshake-declared `provider.name` (§6.5.2 —
    /// the only provider id it can possibly know). The operator registered it in
    /// their host config under a different local id, and trusted its key there.
    /// Recomputing the commitment with the *local* id yields a different
    /// preimage and so `CommitmentMismatch` — the verdict §6.5.4 reserves for a
    /// frame altered after signing. Every test in this module used one string
    /// for both roles, so nothing distinguished them until now.
    #[test]
    fn a_local_id_that_differs_from_the_declared_name_still_verifies() {
        const DECLARED: &str = "acme-docs";
        const LOCAL: &str = "docs-1";
        let frame = frame("frm");
        let attestation = sign_frame_attestation(
            DECLARED,
            &frame,
            &SEED,
            KEY_ID,
            DECLARED,
            "2026-08-29T00:00:00Z",
        );

        // The operator trusts the key under the id they configured, which is the
        // id they also granted consent under.
        let mut store = TrustStore::new();
        store.trust(
            LOCAL,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)),
        );

        let state = store.check_signed_as(LOCAL, DECLARED, &frame, &attestation);
        assert!(
            state.is_attested(),
            "an honest signature must verify regardless of what the operator \
             named the provider locally, got {state:?}"
        );
        assert!(
            state.covers_content(),
            "the frame declares a content_digest"
        );

        // The trust lookup still keys on the local id: a provider cannot reach a
        // key by *declaring* the name it was trusted under.
        let impostor = store.check_signed_as(DECLARED, DECLARED, &frame, &attestation);
        assert!(
            matches!(impostor, AttestationState::NoTrustedKey { .. }),
            "no key is trusted under the declared name, got {impostor:?}"
        );
    }

    // -- key validity windows (#136, ADR 0028) --------------------------------

    /// The window every windowed test uses: the key is in service for 2026.
    fn year_2026() -> KeyValidity {
        KeyValidity::new(
            Some("2026-01-01T00:00:00Z".into()),
            Some("2026-12-31T23:59:59Z".into()),
        )
        .expect("a well-formed window")
    }

    fn store_trusting_for_2026(seed: &[u8; 32]) -> TrustStore {
        let mut store = TrustStore::new();
        store.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(seed)).with_validity(year_2026()),
        );
        store
    }

    #[test]
    fn a_key_is_attested_while_in_service_and_named_out_of_service_after() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let store = store_trusting_for_2026(&SEED);

        assert!(
            store
                .check_signed_as_at(
                    PROVIDER,
                    PROVIDER,
                    &frame,
                    &attestation,
                    "2026-08-29T00:00:01Z"
                )
                .is_attested()
        );

        let lapsed = store.check_signed_as_at(
            PROVIDER,
            PROVIDER,
            &frame,
            &attestation,
            "2027-01-01T00:00:00Z",
        );
        assert_eq!(
            lapsed,
            AttestationState::KeyNotInService {
                key_id: KEY_ID.to_string(),
                received_at: "2027-01-01T00:00:00Z".to_string(),
                position: WindowPosition::Expired,
                validity: year_2026(),
            }
        );
        assert!(!lapsed.is_attested());
        assert!(
            lapsed.was_offered(),
            "an attestation arrived; the key was simply not in service"
        );

        let early = store.check_signed_as_at(
            PROVIDER,
            PROVIDER,
            &frame,
            &attestation,
            "2025-12-31T23:59:59Z",
        );
        assert!(matches!(
            early,
            AttestationState::KeyNotInService {
                position: WindowPosition::NotYetValid,
                ..
            }
        ));
    }

    /// Out of service is its own finding: not "no key" (a configuration gap)
    /// and not "invalid" (a finding about the signature). The attestation
    /// here is signed by an impostor, and the state still names the window,
    /// because the signature is never reached for a key out of service.
    #[test]
    fn out_of_service_is_distinct_from_no_key_and_from_invalid() {
        let frame = frame("frm_1");
        let forged = signed(&frame, &OTHER_SEED);
        let state = store_trusting_for_2026(&SEED).check_signed_as_at(
            PROVIDER,
            PROVIDER,
            &frame,
            &forged,
            "2027-06-01T00:00:00Z",
        );
        assert!(matches!(state, AttestationState::KeyNotInService { .. }));
        assert_ne!(
            state,
            AttestationState::NoTrustedKey {
                key_id: KEY_ID.to_string()
            }
        );
        assert!(!matches!(state, AttestationState::Invalid { .. }));
    }

    /// The attack a window read off `issued_at` would miss: the holder of a
    /// lapsed key back-dates `issued_at` into the window. It is unsigned (F18),
    /// so the signature does not object — and the host never reads it.
    #[test]
    fn back_dating_issued_at_does_not_bring_a_lapsed_key_back_into_service() {
        let frame = frame("frm_1");
        let mut backdated = signed(&frame, &SEED);
        backdated.issued_at = "2026-03-01T00:00:00Z".into();
        let state = store_trusting_for_2026(&SEED).check_signed_as_at(
            PROVIDER,
            PROVIDER,
            &frame,
            &backdated,
            "2028-01-01T00:00:00Z",
        );
        assert!(matches!(
            state,
            AttestationState::KeyNotInService {
                position: WindowPosition::Expired,
                ..
            }
        ));
    }

    /// The auditor-replay case, named. Evidence that arrived in 2026 under a
    /// key that lapsed at the end of 2026 is replayed in a later year with the
    /// receipt instant the host recorded — and is still attested.
    #[test]
    fn an_auditor_replays_archived_evidence_at_its_recorded_receipt_instant() {
        let frames = vec![frame("frm_1"), frame("frm_2")];
        let mut result = root_signed_result(frames.clone(), &SEED);
        // Canonical order sorts `frm_2` second: that entry carries a per-frame
        // signature instead of a proof, so both shapes are replayed.
        result.frame_attestations[1] =
            FrameAttestation::signed(frames[1].identity(PROVIDER), signed(&frames[1], &SEED));
        let store = store_trusting_for_2026(&SEED);
        let recorded = "2026-08-29T00:00:01Z";

        let replayed = store.check_result_signed_as_at(PROVIDER, PROVIDER, &result, recorded);
        assert!(
            replayed.iter().all(|outcome| outcome.state.is_attested()),
            "both shapes — inclusion proof and per-frame signature — replay as attested: \
             {replayed:?}"
        );

        // The same evidence arriving fresh after the lapse is out of service on
        // both paths, the root-signed one included.
        let fresh =
            store.check_result_signed_as_at(PROVIDER, PROVIDER, &result, "2027-02-01T00:00:00Z");
        assert!(
            fresh
                .iter()
                .all(|outcome| matches!(outcome.state, AttestationState::KeyNotInService { .. })),
            "{fresh:?}"
        );
    }

    #[test]
    fn a_key_with_no_window_ignores_the_clock_entirely() {
        // Behaves exactly as before windows existed — even a nonsense instant
        // is never consulted for it.
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let store = store_trusting(&SEED);
        for at in ["1970-01-01T00:00:00Z", "2999-01-01T00:00:00Z", "not a time"] {
            assert!(
                store
                    .check_signed_as_at(PROVIDER, PROVIDER, &frame, &attestation, at)
                    .is_attested(),
                "{at}"
            );
        }
        assert!(store.check(PROVIDER, &frame, &attestation).is_attested());
    }

    #[test]
    fn a_window_that_cannot_be_evaluated_fails_closed() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let store = store_trusting_for_2026(&SEED);
        assert!(matches!(
            store.check_signed_as_at(PROVIDER, PROVIDER, &frame, &attestation, "yesterday"),
            AttestationState::KeyNotInService {
                position: WindowPosition::MalformedInstant,
                ..
            }
        ));

        // A window assembled around `KeyValidity::new`, as a hand-edited store
        // could hold.
        let mut inverted = TrustStore::new();
        inverted.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)).with_validity(KeyValidity {
                not_before: Some("2027-01-01T00:00:00Z".into()),
                not_after: Some("2026-01-01T00:00:00Z".into()),
            }),
        );
        assert!(matches!(
            inverted.check_signed_as_at(
                PROVIDER,
                PROVIDER,
                &frame,
                &attestation,
                "2026-06-01T00:00:00Z"
            ),
            AttestationState::KeyNotInService {
                position: WindowPosition::MalformedWindow,
                ..
            }
        ));
    }

    #[test]
    fn a_windowed_store_round_trips_and_an_unwindowed_one_is_unchanged_on_disk() {
        let windowed = store_trusting_for_2026(&SEED);
        let json = serde_json::to_string(&windowed).expect("serializable");
        assert!(json.contains("not_after"));
        let back: TrustStore = serde_json::from_str(&json).expect("deserializable");
        assert_eq!(back, windowed);

        // A key with no window serializes with no `validity` member at all, so
        // a store persisted before windows existed reads back identically.
        let plain = serde_json::to_string(&store_trusting(&SEED)).expect("serializable");
        assert!(!plain.contains("validity"), "{plain}");
    }

    // -- the pinned tier (#130, ADR 0030) ---------------------------------------

    fn published(seed: &[u8; 32]) -> AttesterKey {
        AttesterKey {
            key_id: KEY_ID.into(),
            algorithm: ALGORITHM_ED25519.into(),
            public_key: encode_hex(&public_key_for(seed)),
        }
    }

    #[test]
    fn a_pinned_key_verifies_as_pinned_never_as_attested() {
        let frame = frame("frm_1");
        let attestation = signed(&frame, &SEED);
        let mut store = TrustStore::new();
        let outcome = store.pin(PROVIDER, &published(&SEED));
        assert!(matches!(outcome, PinOutcome::Pinned { .. }), "{outcome:?}");
        assert!(!outcome.is_alarm());
        assert_eq!(
            store.key(PROVIDER, KEY_ID).map(|key| key.tier),
            Some(TrustTier::Pinned)
        );

        let state = store.check(PROVIDER, &frame, &attestation);
        assert_eq!(
            state,
            AttestationState::Pinned {
                key_id: KEY_ID.to_string(),
                attester_id: "docs-provider".to_string(),
                covers_content: true,
            }
        );
        assert!(
            !state.is_attested(),
            "continuity is not identity: a pin never reads as operator-attested"
        );
        assert!(state.signature_verified());
        assert!(
            state.covers_content(),
            "content binding is a fact about the signature, not the key's tier"
        );
        assert_eq!(state.trust_tier(), Some(TrustTier::Pinned));
        assert_eq!(state.unverified_attester_id(), Some("docs-provider"));
        assert!(state.was_offered());
    }

    /// The DoD witness for #130: a provider that publishes different bytes
    /// under a `key_id` this host pinned is reported loudly, the pin is not
    /// replaced, and the new key's signatures do not verify.
    #[test]
    fn a_changed_key_under_a_pinned_key_id_is_an_alarm_and_is_never_re_pinned() {
        let mut store = TrustStore::new();
        let first = store.pin(PROVIDER, &published(&SEED));
        let pinned_fingerprint = match &first {
            PinOutcome::Pinned { fingerprint, .. } => fingerprint.clone(),
            other => panic!("expected the first sight to pin, got {other:?}"),
        };

        let changed = store.pin(PROVIDER, &published(&OTHER_SEED));
        assert!(changed.is_alarm(), "{changed:?}");
        match &changed {
            PinOutcome::KeyChanged {
                key_id,
                pinned_fingerprint: held,
                offered_fingerprint,
            } => {
                assert_eq!(key_id, KEY_ID);
                assert_eq!(held, &pinned_fingerprint);
                assert_ne!(offered_fingerprint, &pinned_fingerprint);
            }
            other => panic!("expected KeyChanged, got {other:?}"),
        }

        // The pin stands: the original key is still the one held...
        assert_eq!(
            store
                .key(PROVIDER, KEY_ID)
                .and_then(TrustedKey::fingerprint),
            Some(pinned_fingerprint)
        );
        // ...so the new key's signatures are a bad signature, not attested.
        let frame = frame("frm_1");
        assert_eq!(
            store.check(PROVIDER, &frame, &signed(&frame, &OTHER_SEED)),
            AttestationState::Invalid {
                verdict: AttestationVerdict::BadSignature
            }
        );
        // And asking again changes nothing: the alarm repeats rather than
        // wearing off.
        assert!(store.pin(PROVIDER, &published(&OTHER_SEED)).is_alarm());

        // Re-pinning is an operator act: revoke, then pin.
        assert!(store.revoke(PROVIDER, KEY_ID));
        assert!(matches!(
            store.pin(PROVIDER, &published(&OTHER_SEED)),
            PinOutcome::Pinned { .. }
        ));
    }

    #[test]
    fn a_pin_never_outranks_the_operator() {
        // Same bytes as a configured key: nothing changes, and the key stays
        // configured.
        let mut store = store_trusting(&SEED);
        assert_eq!(
            store.pin(PROVIDER, &published(&SEED)),
            PinOutcome::AlreadyConfigured {
                key_id: KEY_ID.to_string()
            }
        );
        assert_eq!(
            store.key(PROVIDER, KEY_ID).map(|key| key.tier),
            Some(TrustTier::Configured)
        );

        // Different bytes under the configured key_id: an alarm, and the
        // operator's key stands.
        let conflict = store.pin(PROVIDER, &published(&OTHER_SEED));
        assert!(matches!(
            conflict,
            PinOutcome::ConflictsWithConfigured { .. }
        ));
        assert!(conflict.is_alarm());
        let frame = frame("frm_1");
        assert!(
            store
                .check(PROVIDER, &frame, &signed(&frame, &SEED))
                .is_attested()
        );

        // And an operator trusting a key under a pinned key_id promotes it.
        let mut promoted = TrustStore::new();
        let _ = promoted.pin(PROVIDER, &published(&SEED));
        promoted.trust(
            PROVIDER,
            TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)),
        );
        assert!(
            promoted
                .check(PROVIDER, &frame, &signed(&frame, &SEED))
                .is_attested()
        );
    }

    #[test]
    fn a_pin_is_keyed_by_the_operators_id_for_the_provider() {
        // A provider cannot reach another provider's pin by publishing under
        // its name: pins live under the local id the host passed.
        let mut store = TrustStore::new();
        let _ = store.pin("docs-a", &published(&SEED));
        let frame = frame("frm_1");
        assert!(matches!(
            store.check("docs-b", &frame, &signed(&frame, &SEED)),
            AttestationState::NoTrustedKey { .. }
        ));
    }

    #[test]
    fn keys_the_host_cannot_verify_with_are_refused_not_pinned() {
        let mut store = TrustStore::new();
        let mut foreign = published(&SEED);
        foreign.algorithm = "ml-dsa-65".into();
        assert!(matches!(
            store.pin(PROVIDER, &foreign),
            PinOutcome::Refused {
                reason: PinRefusal::UnsupportedAlgorithm(_),
                ..
            }
        ));
        let mut garbage = published(&SEED);
        garbage.public_key = "not hex".into();
        assert!(matches!(
            store.pin(PROVIDER, &garbage),
            PinOutcome::Refused {
                reason: PinRefusal::MalformedKey,
                ..
            }
        ));
        let mut anonymous = published(&SEED);
        anonymous.key_id = String::new();
        assert!(matches!(
            store.pin(PROVIDER, &anonymous),
            PinOutcome::Refused {
                reason: PinRefusal::EmptyKeyId,
                ..
            }
        ));
        let mut enormous = published(&SEED);
        enormous.key_id = "k".repeat(10_000);
        match store.pin(PROVIDER, &enormous) {
            PinOutcome::Refused {
                key_id,
                reason: PinRefusal::OversizedKeyId,
            } => assert_eq!(key_id.len(), MAX_ECHOED_IDENTIFIER),
            other => panic!("expected an oversized refusal, got {other:?}"),
        }
        assert!(store.is_empty(), "nothing was pinned");
    }

    #[test]
    fn a_handshake_cannot_grow_the_store_without_bound() {
        let mut store = TrustStore::new();
        let flood: Vec<AttesterKey> = (0..MAX_PINNED_KEYS_PER_PROVIDER + 50)
            .map(|index| AttesterKey {
                key_id: format!("k-{index}"),
                ..published(&SEED)
            })
            .collect();
        let outcomes = store.pin_all(PROVIDER, &flood);
        assert_eq!(
            outcomes.len(),
            MAX_PINNED_KEYS_PER_PROVIDER + 1,
            "the tail is refused once, not walked"
        );
        assert!(matches!(
            outcomes.last(),
            Some(PinOutcome::Refused {
                reason: PinRefusal::TooManyKeys,
                ..
            })
        ));
        assert_eq!(
            store.keys_for(PROVIDER).count(),
            MAX_PINNED_KEYS_PER_PROVIDER
        );
        // And a later handshake adding one more is refused as well.
        let one_more = AttesterKey {
            key_id: "k-late".into(),
            ..published(&SEED)
        };
        assert!(matches!(
            store.pin(PROVIDER, &one_more),
            PinOutcome::Refused {
                reason: PinRefusal::TooManyKeys,
                ..
            }
        ));
    }

    #[test]
    fn a_pinned_tier_survives_serde_and_a_configured_one_is_unchanged_on_disk() {
        let mut store = TrustStore::new();
        let _ = store.pin(PROVIDER, &published(&SEED));
        let json = serde_json::to_string(&store).unwrap();
        assert!(json.contains("\"tier\":\"pinned\""), "{json}");
        assert_eq!(serde_json::from_str::<TrustStore>(&json).unwrap(), store);

        let configured = serde_json::to_string(&store_trusting(&SEED)).unwrap();
        assert!(!configured.contains("tier"), "{configured}");
    }
}
