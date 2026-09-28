package attest

// Record content addressing and record attestation — the lifecycle profile's
// record_hash and RecordAttestation (docs/profiles/context-exchange-provider.md
// §3 and §7, ADR 0017). This is a port of contextgraph_types::record_attest,
// and tests/fixtures/record-hash-vectors.json, record-attestation.json and
// record-attestation-key.json are the vectors record_test.go reconciles it
// against.
//
// Where the rest of this package covers the frame layer a context/query
// returns, this covers the record layer a Context Exchange Provider appends
// and resolves:
//
//  1. record_hash — "sha256:" + hex(SHA-256(JCS(record without its own
//     top-level record_hash member))) (profile LH1). The record's identity:
//     what idempotent replay keys on, what a lineage cites, and what an
//     attestation signs.
//  2. RecordAttestation — a detached Ed25519 signature over that hash under a
//     record-layer domain tag (profile LC4).
//
// The record functions take the record as JSON text ([]byte), not as a
// decoded Go value. A record is an open-ended document, and the only faithful
// way to hash one received from elsewhere is from the bytes it arrived as:
// decoding into map[string]any first turns every number into a float64 and
// resolves a duplicated member silently, and both can change the hash.
// [RecordHashOf] is the convenience for a record you are building in Go.

import (
	"crypto"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/macanderson/context-graph-protocol/sdk/go/contextgraph/jcs"
)

// RecordHashMember is the envelope member a record's own hash lives in, and
// the one member removed from its preimage (profile LH1).
const RecordHashMember = "record_hash"

// RecordAttestationDomain is the domain-separation tag a [RecordAttestation]
// signs under.
//
// Normative: the signed message is these bytes followed by the 32 raw bytes of
// SignedRecordHash (profile LC4). A record_hash is a plain SHA-256 over a JSON
// document — a value any number of unrelated systems also compute — so signing
// it bare would let one signature mean whatever its presenter says. Both
// halves are fixed length, so the concatenation is injective without a length
// prefix.
const RecordAttestationDomain = "contextgraph/attest/1/record"

// ErrMalformedDigest is a digest string that is not the "sha256:<64 lowercase
// hex>" the protocol grammar requires (SPEC.md §6.2).
var ErrMalformedDigest = errors.New("attest: expected a sha256:<64 lowercase hex> digest")

// RecordAttestation is a detached signature over a lifecycle record's
// record_hash (profile §7).
//
// A type of its own rather than a reuse of [ProvenanceAttestation], although
// five of the six fields match, for the reason ADR 0010 gives: the two sign
// different preimages under different domain tags, and a shared type would
// invite presenting one as the other.
type RecordAttestation struct {
	// SignedRecordHash is the "sha256:<hex>" record_hash this signs.
	SignedRecordHash string `json:"signed_record_hash"`
	// KeyID names the signing key. Rotation issues a new id, never reuses one.
	KeyID string `json:"key_id"`
	// Algorithm is the signature scheme, e.g. [AlgorithmEd25519].
	Algorithm string `json:"algorithm"`
	// AttesterID is the accountable authority, as distinct from the key.
	AttesterID string `json:"attester_id"`
	// Signature is the detached signature, lowercase hex.
	Signature string `json:"signature"`
	// IssuedAt is a SPEC.md §F4 protocol timestamp.
	IssuedAt string `json:"issued_at"`
}

// UsesKnownAlgorithm reports whether this names a scheme this revision defines.
func (a RecordAttestation) UsesKnownAlgorithm() bool {
	return a.Algorithm == AlgorithmEd25519
}

// RecordHashPreimage returns the exact bytes a record's record_hash is taken
// over: the RFC 8785 canonical form of the record with its top-level
// record_hash member removed (profile LH1).
//
// Exposed beside [RecordHash] because a hash mismatch between two
// implementations is unreadable and a byte diff of the preimage is not; the
// jcs_utf8 member of each published vector is this output.
//
// The member is removed, not blanked, so a record hashes identically whether
// it carries no record_hash, the right one, or a wrong one. Only the top-level
// member goes: a record_hash nested in extensions or a body member is ordinary
// content and stays in the preimage.
//
// The error wraps [jcs.ErrNotObject] when record is not a JSON object, and the
// jcs package's other named errors when it is not canonicalizable — a lone
// surrogate, a duplicated member, a number no IEEE 754 double holds.
func RecordHashPreimage(record []byte) ([]byte, error) {
	preimage, err := jcs.CanonicalizeWithout(record, RecordHashMember)
	if err != nil {
		return nil, fmt.Errorf("attest: record is not canonicalizable under RFC 8785: %w", err)
	}
	return preimage, nil
}

// RecordHash returns a record's content-addressed identity (profile LH1):
// "sha256:" + hex(SHA-256([RecordHashPreimage](record))).
func RecordHash(record []byte) (string, error) {
	preimage, err := RecordHashPreimage(record)
	if err != nil {
		return "", err
	}
	return DigestString(sha256.Sum256(preimage)), nil
}

// RecordHashOf is [RecordHash] for a record held as a Go value, encoded with
// encoding/json first.
//
// Prefer [RecordHash] on the bytes when you have them. The two agree for any
// value encoding/json encodes faithfully; they part where it does not — a Go
// string holding invalid UTF-8 is coerced to U+FFFD, and a struct field with
// omitempty drops a member the record may have carried.
func RecordHashOf(record any) (string, error) {
	raw, err := json.Marshal(record)
	if err != nil {
		return "", fmt.Errorf("attest: record does not encode to JSON: %w", err)
	}
	return RecordHash(raw)
}

// RecordHashIsCurrent reports whether a record's stored record_hash is the one
// its content produces.
//
// false is the interesting answer: the record was edited after it was hashed,
// or was hashed by an implementation that canonicalizes differently. A record
// with no record_hash member, or one that is not a string, is false rather
// than an error — it is unhashed, not malformed.
func RecordHashIsCurrent(record []byte) (bool, error) {
	computed, err := RecordHash(record)
	if err != nil {
		return false, err
	}
	// The record canonicalized, so it is one object with unique member names
	// and decoding the top level cannot resolve anything ambiguously.
	var members map[string]json.RawMessage
	if err := json.Unmarshal(record, &members); err != nil {
		return false, fmt.Errorf("attest: reading the stored record_hash: %w", err)
	}
	var stored string
	if raw, present := members[RecordHashMember]; !present || json.Unmarshal(raw, &stored) != nil {
		return false, nil
	}
	return stored == computed, nil
}

// RecordAttestationMessage returns the bytes an Ed25519 record attestation
// signs: [RecordAttestationDomain] followed by the digest's 32 raw bytes
// (profile LC4).
//
// Public because it is the normative rule, not an implementation detail: a
// provider signing with a backend that is not a crypto.Signer builds these
// bytes, signs them itself, and assembles the [RecordAttestation].
func RecordAttestationMessage(recordHash string) ([]byte, error) {
	raw, ok := ParseDigest(recordHash)
	if !ok {
		return nil, fmt.Errorf("%w, found %q", ErrMalformedDigest, recordHash)
	}
	message := make([]byte, 0, len(RecordAttestationDomain)+len(raw))
	message = append(message, RecordAttestationDomain...)
	message = append(message, raw[:]...)
	return message, nil
}

// VerifySignedRecordHash checks a detached attestation against an
// already-computed record_hash.
//
// The primitive an auditor uses when they hold the hash and the signature but
// not the record, which is the point of a detached attestation over a content
// address. The checks run in the reference's order: an unknown algorithm is
// declined before anything is parsed, and the hashes are compared before the
// signature is touched, so a record changed after signing is reported as a
// commitment_mismatch rather than the bad_signature a naive order would say.
func VerifySignedRecordHash(expectedRecordHash string, attestation RecordAttestation, publicKey []byte) Result {
	if attestation.Algorithm != AlgorithmEd25519 {
		return Result{Verdict: VerdictUnknownAlgorithm, Algorithm: attestation.Algorithm}
	}
	message, err := RecordAttestationMessage(attestation.SignedRecordHash)
	if err != nil {
		return Result{Verdict: VerdictMalformedCommitment}
	}
	if attestation.SignedRecordHash != expectedRecordHash {
		return Result{
			Verdict:  VerdictCommitmentMismatch,
			Expected: expectedRecordHash,
			Signed:   attestation.SignedRecordHash,
		}
	}
	// The same strict key check the frame layer applies: Go's crypto/ed25519
	// accepts a small-order public key, and a signature under one verifies
	// against arbitrary messages.
	if !usableVerificationKey(publicKey) {
		return Result{Verdict: VerdictMalformedKey}
	}
	signature, decoded := decodeStrictHex(attestation.Signature)
	if !decoded || len(signature) != ed25519.SignatureSize {
		return Result{Verdict: VerdictMalformedSignature}
	}
	if ed25519.Verify(ed25519.PublicKey(publicKey), message, signature) {
		return Result{Verdict: VerdictValid}
	}
	return Result{Verdict: VerdictBadSignature}
}

// VerifyRecordAttestation checks a detached attestation against the record it
// claims to sign (profile LC5).
//
// It recomputes the record's hash from its content rather than reading the
// stored record_hash member, so a record whose content was edited and whose
// record_hash was then rewritten to match is caught as a
// commitment_mismatch — verifying against the stored member would pass it.
//
// A non-nil error means the record could not be hashed at all. That is a
// distinct outcome from every verdict: "this document is not a record" is not
// a statement about the signature.
func VerifyRecordAttestation(record []byte, attestation RecordAttestation, publicKey []byte) (Result, error) {
	expected, err := RecordHash(record)
	if err != nil {
		return Result{}, err
	}
	return VerifySignedRecordHash(expected, attestation, publicKey), nil
}

// SignRecordAttestation signs a record_hash under [RecordAttestationDomain]
// and returns the detached attestation.
//
// signer is any crypto.Signer holding an Ed25519 key — an ed25519.PrivateKey
// from [PrivateKeyFromSeed], or a handle on a key an HSM or KMS keeps. See
// [SignCommitment] for the checks run before the attestation is returned.
func SignRecordAttestation(recordHash string, signer crypto.Signer, keyID, attesterID, issuedAt string) (RecordAttestation, error) {
	message, err := RecordAttestationMessage(recordHash)
	if err != nil {
		return RecordAttestation{}, err
	}
	signature, err := signEd25519(signer, message)
	if err != nil {
		return RecordAttestation{}, err
	}
	return RecordAttestation{
		SignedRecordHash: recordHash,
		KeyID:            keyID,
		Algorithm:        AlgorithmEd25519,
		AttesterID:       attesterID,
		Signature:        signature,
		IssuedAt:         issuedAt,
	}, nil
}

// SignRecord computes a record's own hash and signs it — the convenience a
// provider appending a record wants, so the signed hash cannot drift from the
// content by a copy-paste.
func SignRecord(record []byte, signer crypto.Signer, keyID, attesterID, issuedAt string) (RecordAttestation, error) {
	hash, err := RecordHash(record)
	if err != nil {
		return RecordAttestation{}, err
	}
	return SignRecordAttestation(hash, signer, keyID, attesterID, issuedAt)
}
