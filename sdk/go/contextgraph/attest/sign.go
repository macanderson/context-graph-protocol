package attest

// Signing (§6.5, ADR 0033).
//
// The protocol specifies the preimage and never the custody of the key, so
// every signing entry point here takes a crypto.Signer rather than key bytes.
// That is the interface Go's own ed25519.PrivateKey implements, and the one
// KMS clients, PKCS#11 wrappers and ssh-agent bindings implement as well — so
// one function serves the provider content to hold a key in memory and the
// provider whose key never leaves an HSM, and the second is not asked to
// assemble the attestation by hand.
//
// [PrivateKeyFromSeed] and [PublicKeyFromSeed] mirror the Rust reference's
// seed-based API (contextgraph_types::attest::sign_commitment and
// public_key_for), which is what lets vectors_test.go pin signing to the
// published seed and signature byte for byte rather than round-tripping it
// against itself.

import (
	"crypto"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
)

// The named reasons signing is refused.
var (
	// ErrSeedLength is a signing seed that is not the 32 bytes RFC 8032
	// defines. A shorter seed is not padded and a longer one is not truncated:
	// either would sign under a key the caller did not name.
	ErrSeedLength = errors.New("attest: an Ed25519 signing seed is exactly 32 bytes")
	// ErrNotEd25519Signer is a crypto.Signer whose public key is not an
	// ed25519.PublicKey. This revision defines one algorithm (§6.5), and an
	// attestation naming "ed25519" over some other scheme's signature would
	// verify nowhere.
	ErrNotEd25519Signer = errors.New("attest: the signer's public key is not an Ed25519 key")
	// ErrUnusableSigningKey is a signer whose public key a strict verifier
	// declines — a small-order point, or a y that is not reduced. Signing
	// under it would publish an attestation no conforming verifier accepts.
	ErrUnusableSigningKey = errors.New("attest: the signer's public key is one a strict verifier declines (§6.5.4)")
	// ErrSignatureDoesNotVerify is a signer that returned a signature its own
	// public key does not verify: a faulty backend, or a signer that hashed
	// the message first. Caught here, before the attestation leaves the
	// process, rather than by a consumer later.
	ErrSignatureDoesNotVerify = errors.New("attest: the signer produced a signature its own public key does not verify")
	// ErrFrameHasNoContentDigest is a frame with no content_digest handed to
	// [SignFrameAttestation]. §6.5.2 requires an attester to populate it: the
	// commitment of a digest-less frame binds its identity and provenance but
	// not its content, so the signature would still verify after the frame's
	// content was replaced (ADR 0018).
	ErrFrameHasNoContentDigest = errors.New("attest: a frame must declare a content_digest before it is signed (§6.5.2, ADR 0018)")
)

// PrivateKeyFromSeed returns the Ed25519 private key a 32-byte RFC 8032 seed
// derives — the key the Rust reference's SigningKey::from_bytes builds from
// the same seed. It implements crypto.Signer, so it can be handed straight to
// [SignCommitment], [SignFrameAttestation] or [SignRecord].
//
// Holding the result in application memory is a decision with a cost: the
// key lives as long as the process does, and anything that can read the
// process's memory — a core dump, a debugger, a heap-disclosure bug — can sign
// as you for as long as the key stays unrotated. The README's key-custody
// section says when that is acceptable and what to do instead.
func PrivateKeyFromSeed(seed []byte) (ed25519.PrivateKey, error) {
	if len(seed) != ed25519.SeedSize {
		return nil, fmt.Errorf("%w, got %d", ErrSeedLength, len(seed))
	}
	return ed25519.NewKeyFromSeed(seed), nil
}

// PublicKeyFromSeed returns the public key a 32-byte seed derives, in the raw
// form [VerifyCommitment] and [VerifyRecordAttestation] accept. It mirrors the
// Rust reference's public_key_for.
func PublicKeyFromSeed(seed []byte) (ed25519.PublicKey, error) {
	key, err := PrivateKeyFromSeed(seed)
	if err != nil {
		return nil, err
	}
	return key.Public().(ed25519.PublicKey), nil
}

// signEd25519 signs message with signer and returns the signature as
// lowercase hex, after checking everything a consumer would check.
func signEd25519(signer crypto.Signer, message []byte) (string, error) {
	if signer == nil {
		return "", fmt.Errorf("%w: the signer is nil", ErrNotEd25519Signer)
	}
	public, ok := signer.Public().(ed25519.PublicKey)
	if !ok {
		return "", fmt.Errorf("%w: got %T", ErrNotEd25519Signer, signer.Public())
	}
	if !usableVerificationKey(public) {
		return "", ErrUnusableSigningKey
	}
	// crypto.Hash(0) asks for pure Ed25519 over the message itself, which is
	// what §6.5 signs. Ed25519 is deterministic and ignores the random source;
	// it is passed because some crypto.Signer backends require a non-nil one.
	signature, err := signer.Sign(rand.Reader, message, crypto.Hash(0))
	if err != nil {
		return "", fmt.Errorf("attest: signing: %w", err)
	}
	if len(signature) != ed25519.SignatureSize || !ed25519.Verify(public, message, signature) {
		return "", ErrSignatureDoesNotVerify
	}
	return hex.EncodeToString(signature), nil
}

// SignCommitment signs an already-computed commitment — a [FrameCommitment]
// or a [MerkleRoot] over a result set — and returns the detached attestation.
//
// signer is anything that implements crypto.Signer with an Ed25519 key: an
// ed25519.PrivateKey from [PrivateKeyFromSeed], or a handle on a key held by
// an HSM or KMS. The 32 commitment bytes are signed as they are, with no
// prehash, exactly as contextgraph_types::attest::sign_commitment signs them.
//
// keyID names the signing key, attesterID the accountable authority, and
// issuedAt is a SPEC.md §F4 protocol timestamp; all three are copied into the
// attestation as given.
func SignCommitment(commitment [32]byte, signer crypto.Signer, keyID, attesterID, issuedAt string) (ProvenanceAttestation, error) {
	signature, err := signEd25519(signer, commitment[:])
	if err != nil {
		return ProvenanceAttestation{}, err
	}
	return ProvenanceAttestation{
		SignedCommitment: DigestString(commitment),
		KeyID:            keyID,
		Algorithm:        AlgorithmEd25519,
		AttesterID:       attesterID,
		Signature:        signature,
		IssuedAt:         issuedAt,
	}, nil
}

// SignFrameAttestation computes one frame's [FrameCommitment] and signs it
// (§6.5.2).
//
// It refuses a frame with no ContentDigest, returning
// [ErrFrameHasNoContentDigest]: §6.5.2 requires an attester to populate it,
// because a digest-less commitment does not bind the frame's content (ADR
// 0018). [FrameCommitment] itself still computes such a commitment, because a
// verifier has to be able to check signatures made before that rule.
func SignFrameAttestation(providerID string, frame Frame, signer crypto.Signer, keyID, attesterID, issuedAt string) (ProvenanceAttestation, error) {
	if frame.ContentDigest == nil {
		return ProvenanceAttestation{}, ErrFrameHasNoContentDigest
	}
	return SignCommitment(FrameCommitment(providerID, frame), signer, keyID, attesterID, issuedAt)
}
