//! Provenance attestation — turning "we have a trace" into "we have evidence"
//! (`SPEC.md` §6.5, [ADR 0010](../../docs/adr/0010-provenance-attestation.md)).
//!
//! A [`Provenance`] link carries a `digest`. A digest is **tamper-evident only
//! to a party that already trusts whoever recorded it**: it proves the bytes
//! did not change *since someone wrote that number down*, and says nothing
//! about who wrote it or whether they were entitled to. For a host reading its
//! own cache that is enough. For the auditor asking "prove this citation is
//! what the provider actually served," it is not — the digest and the frame it
//! describes were produced by the same unauthenticated party, so a provider
//! that fabricates a frame simply fabricates a matching digest.
//!
//! A signature closes that gap, and it is the only thing that does. This module
//! defines the three constructions that make a frame's provenance verifiable
//! **offline**, by a third party, with no network and no trust in the host that
//! stored it:
//!
//! 1. A **provenance chain hash** ([`provenance_chain_head`]) — a hash chain
//!    over a frame's ordered [`Provenance`] links, folded source-first, so no
//!    link can be inserted, removed, reordered, or edited without changing the
//!    head.
//! 2. A **frame commitment** ([`frame_commitment`]) — the chain head bound to
//!    the frame's full [`FrameId`] identity.
//! 3. A **Merkle root** ([`merkle_root`]) over a whole result set, with
//!    [`InclusionProof`]s, so one frame can be proven a member of a signed
//!    answer without disclosing its siblings.
//!
//! [`ProvenanceAttestation`] is the detached Ed25519 signature over (1)–(3).
//!
//! # Why the frame identity is inside the signed preimage
//!
//! Signing a bare chain head would be a forgery primitive, not a defense. Two
//! frames citing the same source share a chain head, so a signature over the
//! head alone can be lifted from an innocuous frame and stapled onto a
//! fabricated one: the signature verifies, the evidence is invented. The signed
//! preimage therefore commits to `(provider_id, frame_id, content_digest)` —
//! the whole [`FrameId`] triple — *and* the chain head. A signature binds to one
//! frame served by one provider, or it binds to nothing.
//!
//! # Why the encoding is length-prefixed rather than canonical JSON
//!
//! The lifecycle profile's `record_hash` canonicalizes with RFC 8785 (JCS),
//! which is the right choice there: a record's hash covers a whole open-ended
//! JSON document. A provenance chain is a fixed list of six optional strings,
//! and for that shape JCS is a liability — it makes every implementation depend
//! on a conforming JSON canonicalizer, whose number formatting and Unicode
//! escaping rules are exactly where cross-language implementations silently
//! disagree.
//!
//! This module encodes the typed fields directly, each length-prefixed
//! (`SPEC.md` §6.5.1). Length prefixing is not decoration:
//! naive concatenation is ambiguous, and a chain with `uri: "ab", range: "c"`
//! would otherwise hash identically to one with `uri: "a", range: "bc"` — a
//! collision an adversary chooses, not one they have to find. A four-byte
//! big-endian length in front of every field makes the encoding injective, and
//! any language can produce it from the typed fields with no library at all.
//!
//! # Cryptography is optional; the preimage rule is not
//!
//! Hashing and signature verification live behind the off-by-default
//! `attestation` feature, so `contextgraph-types` keeps its "zero dependencies
//! beyond serde" promise for the pure wire consumer. [`ProvenanceAttestation`]
//! itself is a **wire type and always compiles** — a host must be able to parse,
//! relay, and store an attestation it has not been built to check, exactly as it
//! relays a frame kind it does not recognize.
//!
//! The protocol defines the *preimage*; it does not define your signing
//! backend. [`frame_commitment`] and [`merkle_root`] are public so a provider
//! holding keys in an HSM, a KMS, or a hardware token signs the bytes itself
//! and never hands this crate a secret. [`sign_frame_attestation`] exists for
//! providers and tests that are content to sign in-process.

use serde::{Deserialize, Serialize};

use crate::frame::Provenance;

/// The signature algorithm this revision defines. `algorithm` is a string, not
/// an enum, precisely so a post-quantum successor is an additive change rather
/// than a new major family — see [`ProvenanceAttestation::algorithm`].
pub const ALGORITHM_ED25519: &str = "ed25519";

/// The domain-separation tags and Merkle prefixes the hashing rules use
/// (`SPEC.md` §6.5.1). Only referenced by the gated hashing code, but normative:
/// a reimplementation in another language must use these exact byte strings or
/// it will compute different commitments and interoperate with nothing.
#[cfg(feature = "attestation")]
mod domain {
    /// Domain-separation tag for the hash-chain genesis.
    pub(super) const GENESIS: &[u8] = b"contextgraph/attest/1/genesis";
    /// Domain-separation tag for one provenance link.
    pub(super) const LINK: &[u8] = b"contextgraph/attest/1/link";
    /// Domain-separation tag for a frame commitment.
    pub(super) const FRAME: &[u8] = b"contextgraph/attest/1/frame";
    /// Domain-separation tag for an empty Merkle tree.
    pub(super) const MERKLE_EMPTY: &[u8] = b"contextgraph/attest/1/merkle-empty";
    /// RFC 6962 leaf prefix. Distinct from [`MERKLE_NODE`] so a leaf hash can
    /// never be reinterpreted as an interior node — the second-preimage defense
    /// that makes a Merkle proof mean what it claims.
    pub(super) const MERKLE_LEAF: &[u8] = &[0x00];
    /// RFC 6962 interior-node prefix.
    pub(super) const MERKLE_NODE: &[u8] = &[0x01];
}

/// A detached attestation binding one frame's provenance to a signing identity
/// (`SPEC.md` §6.5).
///
/// **Detached, always.** Like the lifecycle profile's
/// [`RecordAttestation`](crate::RecordAttestation), this never travels inside
/// the preimage it signs. Re-signing after a key rotation, or a second attester
/// countersigning the same frame, must not perturb the frame's content-addressed
/// identity — and it cannot, because the attestation is metadata beside the
/// frame rather than a field within it.
///
/// It is a **distinct type** from `RecordAttestation` even though five of six
/// fields match. The two sign different preimages under different domain tags,
/// and a shared type would invite the one mistake the domain separation exists
/// to prevent: presenting a record attestation as a frame attestation. The
/// cryptography already refuses that; the type system should make it unsayable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceAttestation {
    /// The `sha256:<hex>` commitment this attestation signs — a
    /// [`frame_commitment`] for a single frame, or a [`merkle_root`] for a
    /// result set.
    pub signed_commitment: String,
    /// The signing key's id. Rotation is expressed by a new `key_id`, never by
    /// reusing one, so an archived attestation always names the exact key that
    /// produced it.
    pub key_id: String,
    /// The signature scheme, e.g. [`ALGORITHM_ED25519`].
    ///
    /// A string rather than an enum: a verifier that does not recognize the
    /// value returns [`AttestationVerdict::UnknownAlgorithm`] and declines,
    /// which is a *safe* failure. Freezing the set into an enum would make
    /// adopting a post-quantum scheme a breaking wire change, and this protocol
    /// promises no flag day inside a major family.
    pub algorithm: String,
    /// The attesting authority — who is accountable for the claim, as distinct
    /// from which key mechanically produced it.
    pub attester_id: String,
    /// The detached signature, lowercase hex.
    ///
    /// Hex rather than base64 to match the `sha256:<hex>` convention every other
    /// digest in this protocol already uses; one encoding across the wire
    /// surface is worth more than the 40 bytes base64 would save.
    pub signature: String,
    /// When the attestation was issued (a `SPEC.md` §F4 protocol timestamp).
    pub issued_at: String,
}

impl ProvenanceAttestation {
    /// Build an attestation from its parts.
    pub fn new(
        signed_commitment: impl Into<String>,
        key_id: impl Into<String>,
        algorithm: impl Into<String>,
        attester_id: impl Into<String>,
        signature: impl Into<String>,
        issued_at: impl Into<String>,
    ) -> Self {
        Self {
            signed_commitment: signed_commitment.into(),
            key_id: key_id.into(),
            algorithm: algorithm.into(),
            attester_id: attester_id.into(),
            signature: signature.into(),
            issued_at: issued_at.into(),
        }
    }

    /// Whether this attestation names a scheme this revision defines.
    ///
    /// Advisory: a verifier reports [`AttestationVerdict::UnknownAlgorithm`]
    /// rather than treating an unrecognized scheme as a failure to *validate*.
    /// The distinction matters to an auditor — "I cannot check this" is a
    /// different finding from "this is forged."
    pub fn uses_known_algorithm(&self) -> bool {
        self.algorithm == ALGORITHM_ED25519
    }

    /// Whether `issued_at` is a well-formed protocol timestamp (`SPEC.md` §F4).
    pub fn has_well_formed_issued_at(&self) -> bool {
        crate::validate::is_protocol_timestamp(&self.issued_at)
    }
}

/// One step of a Merkle [`InclusionProof`]: the sibling hash, and which side it
/// sits on.
///
/// RFC 6962 lets a verifier recover the side from index arithmetic. This carries
/// it explicitly instead. The redundancy costs one bool per step and removes an
/// entire class of verifier bug — an off-by-one in the index recursion produces
/// a *wrong root* rather than a silently-accepted proof, and a hand-written
/// verifier in another language is far likelier to get a stated side right than
/// to re-derive the split correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionStep {
    /// The sibling subtree hash, `sha256:<hex>`.
    pub sibling: String,
    /// Whether the sibling is the **left** operand at this level.
    pub sibling_is_left: bool,
}

/// A proof that one frame commitment is a leaf of a signed [`merkle_root`]
/// (`SPEC.md` §6.5.3).
///
/// This is what makes a signed answer *selectively* disclosable. A host that
/// served twelve frames can prove to an auditor that one specific frame was in
/// the signed set — and prove the provider committed to it before knowing which
/// one would be questioned — while disclosing nothing about the other eleven
/// beyond their hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    /// The leaf's index in canonical order.
    pub leaf_index: usize,
    /// How many leaves the tree held. Part of the proof because a root alone
    /// does not pin the tree's size, and a verifier that ignores it can be shown
    /// a proof from a differently-shaped tree.
    pub leaf_count: usize,
    /// Sibling hashes from the leaf upward.
    pub path: Vec<InclusionStep>,
}

/// The outcome of checking a [`ProvenanceAttestation`] (`SPEC.md` §6.5.4).
///
/// Every failure is *named*. A boolean would collapse "this signature is
/// forged" into "I was handed a truncated key," and those call for opposite
/// responses: the first is an incident, the second is a configuration bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationVerdict {
    /// The signature verifies against the recomputed commitment.
    Valid,
    /// The signature is well-formed and verifies, but over a *different*
    /// commitment than this frame produces — the frame or its provenance was
    /// altered after signing. The loudest possible finding.
    CommitmentMismatch {
        /// The commitment recomputed from the frame in hand.
        expected: String,
        /// The commitment the attestation claims to sign.
        signed: String,
    },
    /// The commitment matches but the signature does not verify under the
    /// supplied key: a forgery, or the wrong key.
    BadSignature,
    /// The named algorithm is not one this build can check. Not a failure to
    /// validate — a refusal to guess.
    UnknownAlgorithm(String),
    /// The public key was not a well-formed key for the named algorithm.
    MalformedKey,
    /// The signature field was not well-formed for the named algorithm.
    MalformedSignature,
    /// `signed_commitment` was not a well-formed `sha256:<hex>` digest.
    MalformedCommitment,
}

impl AttestationVerdict {
    /// Whether this verdict is [`Valid`](Self::Valid).
    ///
    /// A host **MUST NOT** treat any other verdict as provisionally acceptable:
    /// the point of an attestation is that "I could not check it" and "it is
    /// good" are never the same answer.
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }
}

// ---------------------------------------------------------------------------
// Canonical encoding (`SPEC.md` §6.5.1) — dependency-free, so the rule is
// readable and reimplementable even in a build with `attestation` disabled.
// ---------------------------------------------------------------------------

/// Append a length-prefixed string: `u32be(len) || utf8`.
fn enc_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Append a length-prefixed optional string: `0x00` for absent, `0x01 ||
/// enc_str` for present.
///
/// The presence byte is what keeps absent distinct from empty. Without it
/// `uri: None` and `uri: Some("")` would encode identically, and a provider
/// could drop a URI from a signed chain without disturbing the hash.
fn enc_opt(out: &mut Vec<u8>, s: Option<&str>) {
    match s {
        None => out.push(0x00),
        Some(s) => {
            out.push(0x01);
            enc_str(out, s);
        }
    }
}

/// The canonical encoding of one provenance link (`SPEC.md` §6.5.1).
///
/// Field order is fixed by the struct's declaration order and pinned by the
/// spec — it is part of the normative rule, not an implementation detail, and
/// changing it is a breaking wire change.
pub fn encode_provenance_link(link: &Provenance) -> Vec<u8> {
    let mut out = Vec::new();
    enc_str(&mut out, &link.kind);
    enc_opt(&mut out, link.uri.as_deref());
    enc_opt(&mut out, link.range.as_deref());
    enc_opt(&mut out, link.digest.as_deref());
    enc_opt(&mut out, link.method.as_deref());
    enc_opt(&mut out, link.by.as_deref());
    out
}

/// Render 32 raw bytes as this protocol's `sha256:<hex>` digest string.
pub fn digest_string(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(7 + 64);
    s.push_str("sha256:");
    for b in bytes {
        // Lowercase hex, two chars per byte — the form `is_well_formed_digest`
        // accepts and every other digest in the protocol already uses.
        s.push(char::from_digit((b >> 4) as u32, 16).expect("nibble is < 16"));
        s.push(char::from_digit((b & 0x0f) as u32, 16).expect("nibble is < 16"));
    }
    s
}

/// Parse lowercase hex into bytes. `None` on any non-hex byte or odd length.
#[cfg(feature = "attestation")]
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Parse a `sha256:<hex>` digest string into its 32 raw bytes.
#[cfg(feature = "attestation")]
fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    let bytes = from_hex(hex)?;
    bytes.try_into().ok()
}

// ---------------------------------------------------------------------------
// Hashing and signing — gated, because they need real cryptography.
// ---------------------------------------------------------------------------

#[cfg(feature = "attestation")]
mod crypto {
    use super::*;
    use crate::frame::ContextFrame;
    use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
    use sha2::{Digest, Sha256};

    /// SHA-256 over a sequence of parts, hashed in order without any separator
    /// beyond the parts' own length prefixes.
    fn sha256(parts: &[&[u8]]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update(part);
        }
        hasher.finalize().into()
    }

    /// The head of a frame's provenance hash chain (`SPEC.md` §6.5.2).
    ///
    /// Links fold **source-first**, matching the order [`Provenance`] is
    /// documented to carry (closest-to-source first), so each link commits to
    /// everything nearer the source than itself:
    ///
    /// ```text
    /// h₋₁ = SHA256(domain::GENESIS)
    /// hᵢ  = SHA256(domain::LINK ‖ hᵢ₋₁ ‖ encode(linkᵢ))
    /// head = hₙ₋₁          (or h₋₁ for an empty chain)
    /// ```
    ///
    /// Because every step consumes the previous head, no link can be inserted,
    /// dropped, reordered, or edited without changing the result — which is the
    /// property a bare per-link digest never had. An empty chain hashes to the
    /// genesis value rather than to zero or to a sentinel, so "no provenance" is
    /// a *stated* claim a signature can cover, not a gap.
    pub fn provenance_chain_head(links: &[Provenance]) -> [u8; 32] {
        let mut head = sha256(&[domain::GENESIS]);
        for link in links {
            let encoded = encode_provenance_link(link);
            head = sha256(&[domain::LINK, &head, &encoded]);
        }
        head
    }

    /// The commitment binding one frame's identity to its provenance chain
    /// (`SPEC.md` §6.5.2) — the preimage a single-frame attestation signs.
    ///
    /// ```text
    /// SHA256(
    ///   domain::FRAME ‖ enc(provider_id) ‖ enc(frame.id)
    ///                ‖ enc_opt(frame.content_digest) ‖ chain_head
    /// )
    /// ```
    ///
    /// `content_digest` is included so the signature covers the frame's *bytes*,
    /// not merely its name: without it, a provider could re-serve different
    /// content under the same frame id and the old signature would still check
    /// out. It is an `Option` because a frame is permitted to declare no digest
    /// — such a frame is unverifiable by design
    /// (`docs/context-reuse.md` §4), and the encoding records that absence
    /// honestly rather than substituting a placeholder.
    pub fn frame_commitment(provider_id: &str, frame: &ContextFrame) -> [u8; 32] {
        let chain_head = provenance_chain_head(&frame.provenance);
        let mut preimage = Vec::new();
        enc_str(&mut preimage, provider_id);
        enc_str(&mut preimage, &frame.id);
        enc_opt(&mut preimage, frame.content_digest.as_deref());
        sha256(&[domain::FRAME, &preimage, &chain_head])
    }

    /// A Merkle leaf hash, RFC 6962 style: `SHA256(0x00 ‖ commitment)`.
    fn leaf_hash(commitment: &[u8; 32]) -> [u8; 32] {
        sha256(&[domain::MERKLE_LEAF, commitment])
    }

    /// A Merkle interior node, RFC 6962 style: `SHA256(0x01 ‖ left ‖ right)`.
    fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        sha256(&[domain::MERKLE_NODE, left, right])
    }

    /// The largest power of two strictly less than `n` (RFC 6962's split point).
    /// Only meaningful for `n >= 2`.
    fn split_point(n: usize) -> usize {
        let mut k = 1;
        while k * 2 < n {
            k *= 2;
        }
        k
    }

    /// The Merkle root over a set of frame commitments (`SPEC.md` §6.5.3).
    ///
    /// RFC 6962's tree shape, chosen over "duplicate the last leaf on an odd
    /// level" because that shortcut admits two distinct leaf sets with the same
    /// root — an ambiguity that is fine for a checksum and disqualifying for
    /// evidence. Callers pass commitments in the protocol's canonical
    /// [`FrameId`](crate::FrameId) order so the root is reproducible.
    pub fn merkle_root(commitments: &[[u8; 32]]) -> [u8; 32] {
        match commitments.len() {
            0 => sha256(&[domain::MERKLE_EMPTY]),
            1 => leaf_hash(&commitments[0]),
            n => {
                let k = split_point(n);
                node_hash(
                    &merkle_root(&commitments[..k]),
                    &merkle_root(&commitments[k..]),
                )
            }
        }
    }

    /// Build an [`InclusionProof`] for `leaf_index` within `commitments`.
    /// `None` if the index is out of range.
    pub fn inclusion_proof(commitments: &[[u8; 32]], leaf_index: usize) -> Option<InclusionProof> {
        if leaf_index >= commitments.len() {
            return None;
        }
        let mut path = Vec::new();
        collect_path(commitments, leaf_index, &mut path);
        Some(InclusionProof {
            leaf_index,
            leaf_count: commitments.len(),
            path,
        })
    }

    /// Walk down the tree accumulating sibling hashes, leaf-upward.
    fn collect_path(commitments: &[[u8; 32]], index: usize, path: &mut Vec<InclusionStep>) {
        if commitments.len() <= 1 {
            return;
        }
        let k = split_point(commitments.len());
        if index < k {
            collect_path(&commitments[..k], index, path);
            path.push(InclusionStep {
                sibling: digest_string(&merkle_root(&commitments[k..])),
                sibling_is_left: false,
            });
        } else {
            collect_path(&commitments[k..], index - k, path);
            path.push(InclusionStep {
                sibling: digest_string(&merkle_root(&commitments[..k])),
                sibling_is_left: true,
            });
        }
    }

    /// Recompute a Merkle root from a leaf commitment and its proof.
    ///
    /// This is the whole offline story: an auditor holding one frame, its proof,
    /// and a signed root needs nothing else — no network, no host, no provider.
    /// `None` if any sibling in the path is malformed.
    pub fn root_from_proof(commitment: &[u8; 32], proof: &InclusionProof) -> Option<[u8; 32]> {
        if proof.leaf_index >= proof.leaf_count {
            return None;
        }
        let mut acc = leaf_hash(commitment);
        for step in &proof.path {
            let sibling = parse_digest(&step.sibling)?;
            acc = if step.sibling_is_left {
                node_hash(&sibling, &acc)
            } else {
                node_hash(&acc, &sibling)
            };
        }
        Some(acc)
    }

    /// Verify a detached attestation over a single frame (`SPEC.md` §6.5.4).
    ///
    /// Pure and offline. `public_key` is raw bytes rather than an
    /// `ed25519_dalek` type on purpose: the public API of this crate names no
    /// cryptography library, so the backend can be replaced — or a
    /// post-quantum scheme added — without a breaking change to callers.
    pub fn verify_frame_attestation(
        provider_id: &str,
        frame: &ContextFrame,
        attestation: &ProvenanceAttestation,
        public_key: &[u8],
    ) -> AttestationVerdict {
        let expected = frame_commitment(provider_id, frame);
        verify_commitment(&expected, attestation, public_key)
    }

    /// Verify a detached attestation over an already-computed commitment — a
    /// [`merkle_root`] for a result set, or a [`frame_commitment`].
    pub fn verify_commitment(
        expected: &[u8; 32],
        attestation: &ProvenanceAttestation,
        public_key: &[u8],
    ) -> AttestationVerdict {
        if attestation.algorithm != ALGORITHM_ED25519 {
            return AttestationVerdict::UnknownAlgorithm(attestation.algorithm.clone());
        }
        let Some(signed) = parse_digest(&attestation.signed_commitment) else {
            return AttestationVerdict::MalformedCommitment;
        };
        // Compare commitments *before* touching the signature. A mismatch means
        // the frame changed after signing, and saying so is far more useful to
        // an operator than the "bad signature" a naive order would report.
        if signed != *expected {
            return AttestationVerdict::CommitmentMismatch {
                expected: digest_string(expected),
                signed: attestation.signed_commitment.clone(),
            };
        }
        let Ok(key_bytes) = <[u8; 32]>::try_from(public_key) else {
            return AttestationVerdict::MalformedKey;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
            return AttestationVerdict::MalformedKey;
        };
        let Some(sig_bytes) = from_hex(&attestation.signature) else {
            return AttestationVerdict::MalformedSignature;
        };
        let Ok(sig_bytes) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            return AttestationVerdict::MalformedSignature;
        };
        let signature = Signature::from_bytes(&sig_bytes);
        // `verify_strict` rejects small-order public keys and the malleable
        // signature forms `verify` tolerates. For evidence, the strict variant
        // is the only defensible choice: a signature that two verifiers can
        // disagree about is not evidence.
        match verifying_key.verify_strict(&signed, &signature) {
            Ok(()) => AttestationVerdict::Valid,
            Err(_) => AttestationVerdict::BadSignature,
        }
    }

    /// Sign a frame's commitment in-process, for providers content to hold key
    /// material in memory.
    ///
    /// A provider using an HSM or KMS instead calls [`frame_commitment`],
    /// signs the 32 bytes with its own backend, and assembles the
    /// [`ProvenanceAttestation`] by hand — the protocol specifies the preimage,
    /// never the custody of the key.
    pub fn sign_frame_attestation(
        provider_id: &str,
        frame: &ContextFrame,
        signing_key_seed: &[u8; 32],
        key_id: impl Into<String>,
        attester_id: impl Into<String>,
        issued_at: impl Into<String>,
    ) -> ProvenanceAttestation {
        let commitment = frame_commitment(provider_id, frame);
        sign_commitment(
            &commitment,
            signing_key_seed,
            key_id,
            attester_id,
            issued_at,
        )
    }

    /// Sign an arbitrary commitment (a frame commitment or a Merkle root).
    pub fn sign_commitment(
        commitment: &[u8; 32],
        signing_key_seed: &[u8; 32],
        key_id: impl Into<String>,
        attester_id: impl Into<String>,
        issued_at: impl Into<String>,
    ) -> ProvenanceAttestation {
        let signing_key = SigningKey::from_bytes(signing_key_seed);
        let signature = signing_key.sign(commitment);
        let mut hex = String::with_capacity(128);
        for b in signature.to_bytes() {
            hex.push(char::from_digit((b >> 4) as u32, 16).expect("nibble is < 16"));
            hex.push(char::from_digit((b & 0x0f) as u32, 16).expect("nibble is < 16"));
        }
        ProvenanceAttestation::new(
            digest_string(commitment),
            key_id,
            ALGORITHM_ED25519,
            attester_id,
            hex,
            issued_at,
        )
    }

    /// The public key matching a signing seed, as raw bytes — the form
    /// [`verify_frame_attestation`] accepts.
    pub fn public_key_for(signing_key_seed: &[u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(signing_key_seed)
            .verifying_key()
            .to_bytes()
    }
}

#[cfg(feature = "attestation")]
pub use crypto::{
    frame_commitment, inclusion_proof, merkle_root, provenance_chain_head, public_key_for,
    root_from_proof, sign_commitment, sign_frame_attestation, verify_commitment,
    verify_frame_attestation,
};

#[cfg(all(test, feature = "attestation"))]
mod tests {
    use super::*;
    use crate::frame::{ContextFrame, FrameKind};

    /// A deterministic seed — tests need reproducible signatures, and this key
    /// signs nothing outside this file.
    const SEED: [u8; 32] = [7u8; 32];

    fn link(kind: &str, uri: Option<&str>, digest: Option<&str>) -> Provenance {
        Provenance {
            kind: kind.into(),
            uri: uri.map(Into::into),
            range: None,
            digest: digest.map(Into::into),
            method: None,
            by: None,
        }
    }

    fn frame_with(id: &str, provenance: Vec<Provenance>) -> ContextFrame {
        let mut frame = ContextFrame::full(id, FrameKind::Doc, "Retry policy", "body", 0.9, 1);
        frame.content_digest = Some("sha256:abcd".into());
        frame.provenance = provenance;
        frame
    }

    #[test]
    fn the_encoding_is_injective_across_field_boundaries() {
        // The attack length-prefixing exists to stop: without it, ("ab", "c")
        // and ("a", "bc") concatenate to the same bytes and an adversary picks
        // the collision rather than searching for one.
        let a = link("file", Some("ab"), Some("c"));
        let b = link("file", Some("a"), Some("bc"));
        assert_ne!(encode_provenance_link(&a), encode_provenance_link(&b));
    }

    #[test]
    fn an_absent_field_never_encodes_like_an_empty_one() {
        let absent = link("file", None, None);
        let empty = link("file", Some(""), None);
        assert_ne!(
            encode_provenance_link(&absent),
            encode_provenance_link(&empty),
            "the presence byte must keep None distinct from Some(\"\")"
        );
    }

    #[test]
    fn an_empty_chain_has_a_stated_head_not_a_zero() {
        let head = provenance_chain_head(&[]);
        assert_ne!(head, [0u8; 32], "\"no provenance\" is a claim, not a gap");
        // Stable across calls — the genesis is a constant, not a nonce.
        assert_eq!(head, provenance_chain_head(&[]));
    }

    #[test]
    fn reordering_the_chain_changes_the_head() {
        let a = link("file", Some("src/a.rs"), Some("sha256:aa"));
        let b = link("derivation", Some("summary"), Some("sha256:bb"));
        let forward = provenance_chain_head(&[a.clone(), b.clone()]);
        let reversed = provenance_chain_head(&[b, a]);
        assert_ne!(
            forward, reversed,
            "a hash chain must bind order; per-link digests never did"
        );
    }

    #[test]
    fn dropping_a_link_changes_the_head() {
        let a = link("file", Some("src/a.rs"), Some("sha256:aa"));
        let b = link("derivation", None, None);
        assert_ne!(
            provenance_chain_head(&[a.clone(), b]),
            provenance_chain_head(&[a]),
            "truncating provenance must be detectable"
        );
    }

    #[test]
    fn a_signed_frame_verifies_against_its_own_key() {
        let frame = frame_with(
            "f1",
            vec![link("file", Some("src/a.rs"), Some("sha256:aa"))],
        );
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let key = public_key_for(&SEED);
        assert_eq!(
            verify_frame_attestation("repo-graph", &frame, &attestation, &key),
            AttestationVerdict::Valid
        );
        assert!(attestation.uses_known_algorithm());
        assert!(attestation.has_well_formed_issued_at());
    }

    #[test]
    fn editing_provenance_after_signing_is_caught_as_a_mismatch() {
        let frame = frame_with(
            "f1",
            vec![link("file", Some("src/a.rs"), Some("sha256:aa"))],
        );
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        // Rewrite the source URI — the exact tamper a bare digest cannot see,
        // because the tamperer simply rewrites the digest too.
        let mut tampered = frame.clone();
        tampered.provenance[0].uri = Some("src/evil.rs".into());
        tampered.provenance[0].digest = Some("sha256:ff".into());

        let key = public_key_for(&SEED);
        let verdict = verify_frame_attestation("repo-graph", &tampered, &attestation, &key);
        assert!(
            matches!(verdict, AttestationVerdict::CommitmentMismatch { .. }),
            "expected a commitment mismatch, got {verdict:?}"
        );
        assert!(!verdict.is_valid());
    }

    #[test]
    fn a_signature_cannot_be_lifted_onto_another_frame() {
        // The forgery the FrameId binding exists to prevent. Both frames cite
        // exactly the same source, so they share a chain head; only the identity
        // binding distinguishes them.
        let shared = vec![link("file", Some("src/a.rs"), Some("sha256:aa"))];
        let honest = frame_with("f1", shared.clone());
        let forged = frame_with("f2", shared);
        assert_eq!(
            provenance_chain_head(&honest.provenance),
            provenance_chain_head(&forged.provenance),
            "precondition: identical provenance means an identical chain head"
        );

        let attestation = sign_frame_attestation(
            "repo-graph",
            &honest,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let key = public_key_for(&SEED);
        assert!(
            matches!(
                verify_frame_attestation("repo-graph", &forged, &attestation, &key),
                AttestationVerdict::CommitmentMismatch { .. }
            ),
            "a stolen signature must not validate a different frame"
        );
    }

    #[test]
    fn the_same_frame_from_another_provider_does_not_verify() {
        let frame = frame_with("f1", vec![link("file", Some("src/a.rs"), None)]);
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let key = public_key_for(&SEED);
        assert!(
            matches!(
                verify_frame_attestation("impostor", &frame, &attestation, &key),
                AttestationVerdict::CommitmentMismatch { .. }
            ),
            "the provider id is part of the signed identity"
        );
    }

    #[test]
    fn re_serving_different_bytes_under_the_same_id_is_caught() {
        let frame = frame_with("f1", vec![link("file", Some("src/a.rs"), None)]);
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let mut swapped = frame.clone();
        swapped.content_digest = Some("sha256:0000".into());
        let key = public_key_for(&SEED);
        assert!(
            matches!(
                verify_frame_attestation("repo-graph", &swapped, &attestation, &key),
                AttestationVerdict::CommitmentMismatch { .. }
            ),
            "the signature covers the frame's bytes, not just its name"
        );
    }

    #[test]
    fn a_wrong_key_is_a_bad_signature_not_a_mismatch() {
        let frame = frame_with("f1", vec![]);
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let other = public_key_for(&[9u8; 32]);
        assert_eq!(
            verify_frame_attestation("repo-graph", &frame, &attestation, &other),
            AttestationVerdict::BadSignature,
            "the commitment is intact; only the key is wrong"
        );
    }

    #[test]
    fn an_unknown_algorithm_is_declined_rather_than_failed() {
        let frame = frame_with("f1", vec![]);
        let mut attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        attestation.algorithm = "dilithium3".into();
        let key = public_key_for(&SEED);
        let verdict = verify_frame_attestation("repo-graph", &frame, &attestation, &key);
        assert_eq!(
            verdict,
            AttestationVerdict::UnknownAlgorithm("dilithium3".into())
        );
        assert!(!verdict.is_valid(), "declining is still not accepting");
        assert!(!attestation.uses_known_algorithm());
    }

    #[test]
    fn malformed_keys_and_signatures_are_named_distinctly() {
        let frame = frame_with("f1", vec![]);
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        assert_eq!(
            verify_frame_attestation("repo-graph", &frame, &attestation, &[0u8; 5]),
            AttestationVerdict::MalformedKey
        );

        let mut truncated = attestation.clone();
        truncated.signature = "abcd".into();
        assert_eq!(
            verify_frame_attestation("repo-graph", &frame, &truncated, &public_key_for(&SEED)),
            AttestationVerdict::MalformedSignature
        );

        let mut bad_commitment = attestation;
        bad_commitment.signed_commitment = "not-a-digest".into();
        assert_eq!(
            verify_frame_attestation(
                "repo-graph",
                &frame,
                &bad_commitment,
                &public_key_for(&SEED)
            ),
            AttestationVerdict::MalformedCommitment
        );
    }

    #[test]
    fn an_attestation_round_trips_through_json() {
        let frame = frame_with("f1", vec![link("file", Some("a"), None)]);
        let attestation = sign_frame_attestation(
            "repo-graph",
            &frame,
            &SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        );
        let json = serde_json::to_string(&attestation).unwrap();
        let back: ProvenanceAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, attestation);
    }

    #[test]
    fn every_leaf_of_a_signed_set_proves_its_own_membership() {
        let commitments: Vec<[u8; 32]> = (0..7)
            .map(|i| frame_commitment("repo-graph", &frame_with(&format!("f{i}"), vec![])))
            .collect();
        let root = merkle_root(&commitments);

        for (index, commitment) in commitments.iter().enumerate() {
            let proof = inclusion_proof(&commitments, index).expect("index is in range");
            assert_eq!(proof.leaf_index, index);
            assert_eq!(proof.leaf_count, 7);
            assert_eq!(
                root_from_proof(commitment, &proof),
                Some(root),
                "leaf {index} must recompute the signed root"
            );
        }
    }

    #[test]
    fn a_proof_does_not_validate_a_commitment_that_was_not_in_the_set() {
        let commitments: Vec<[u8; 32]> = (0..4)
            .map(|i| frame_commitment("repo-graph", &frame_with(&format!("f{i}"), vec![])))
            .collect();
        let root = merkle_root(&commitments);
        let proof = inclusion_proof(&commitments, 1).unwrap();

        let outsider = frame_commitment("repo-graph", &frame_with("intruder", vec![]));
        assert_ne!(
            root_from_proof(&outsider, &proof),
            Some(root),
            "an unsigned frame must not ride someone else's proof"
        );
    }

    #[test]
    fn a_single_frame_set_still_produces_a_usable_proof() {
        let commitments = vec![frame_commitment("repo-graph", &frame_with("only", vec![]))];
        let root = merkle_root(&commitments);
        let proof = inclusion_proof(&commitments, 0).unwrap();
        assert!(proof.path.is_empty(), "a lone leaf needs no siblings");
        assert_eq!(root_from_proof(&commitments[0], &proof), Some(root));
    }

    #[test]
    fn an_empty_set_has_a_distinct_root() {
        let empty = merkle_root(&[]);
        let lone = merkle_root(&[frame_commitment("repo-graph", &frame_with("only", vec![]))]);
        assert_ne!(empty, lone);
        assert!(inclusion_proof(&[], 0).is_none());
    }

    #[test]
    fn leaf_and_node_hashing_are_domain_separated() {
        // Without the RFC 6962 prefixes, an interior node's hash could be
        // presented as a leaf, letting a subtree masquerade as a single frame.
        let a = frame_commitment("repo-graph", &frame_with("a", vec![]));
        let b = frame_commitment("repo-graph", &frame_with("b", vec![]));
        let pair_root = merkle_root(&[a, b]);
        // The two-leaf root must not equal the one-leaf root of any commitment.
        assert_ne!(pair_root, merkle_root(&[a]));
        assert_ne!(pair_root, merkle_root(&[b]));
    }

    #[test]
    fn a_signed_merkle_root_verifies_for_the_whole_result_set() {
        let commitments: Vec<[u8; 32]> = (0..3)
            .map(|i| frame_commitment("repo-graph", &frame_with(&format!("f{i}"), vec![])))
            .collect();
        let root = merkle_root(&commitments);
        let attestation = sign_commitment(&root, &SEED, "key-1", "oxagen", "2026-08-27T00:00:00Z");
        let key = public_key_for(&SEED);
        assert_eq!(
            verify_commitment(&root, &attestation, &key),
            AttestationVerdict::Valid
        );
    }

    #[test]
    fn digest_strings_are_well_formed_protocol_digests() {
        let head = provenance_chain_head(&[link("file", Some("a"), None)]);
        let rendered = digest_string(&head);
        assert!(
            crate::validate::is_well_formed_digest(&rendered),
            "{rendered} must satisfy the protocol digest grammar"
        );
    }
}
