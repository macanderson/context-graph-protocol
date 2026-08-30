// Package attest implements the provenance-attestation constructions of
// SPEC.md §6.5: the length-prefixed link encoding, the source-first chain
// fold, the frame commitment, the RFC 6962 Merkle root and inclusion proofs,
// and strict Ed25519 verification.
//
// This is a port of contextgraph_types::attest, and the Rust crate is the
// reference: the vectors in tests/vectors/attestation-vectors.json come from
// it, and vectors_test.go reconciles every function here against them.
//
// # Why a Link type of its own
//
// contextgraph.Provenance carries its optional fields as string with
// omitempty, which cannot distinguish an absent URI from a present empty one.
// The §6.5.1 presence byte is normative precisely because those two must
// differ — without it a URI could be deleted from a signed chain without
// disturbing the hash — so this package takes pointers and
// [LinkFromProvenance] states the collapse it performs rather than hiding it.
//
// # No JSON canonicalizer
//
// The encoding was chosen over RFC 8785 (JCS) so this port needs none (ADR
// 0010). A provenance link is six optional strings; if you find yourself
// reaching for encoding/json here, re-read §6.5.1.
package attest

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math/big"
	"strings"

	"github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

// AlgorithmEd25519 is the signature algorithm this revision defines (§6.5).
const AlgorithmEd25519 = "ed25519"

// The domain-separation tags and Merkle prefixes the hashing rules use
// (§6.5.1). These exact byte strings are normative — a port that spells one
// differently computes different commitments and interoperates with nothing.
var (
	domainGenesis     = []byte("contextgraph/attest/1/genesis")
	domainLink        = []byte("contextgraph/attest/1/link")
	domainFrame       = []byte("contextgraph/attest/1/frame")
	domainMerkleEmpty = []byte("contextgraph/attest/1/merkle-empty")

	// RFC 6962 prefixes, distinct so a leaf hash can never be reinterpreted
	// as an interior node — the second-preimage defense that makes a Merkle
	// proof mean what it claims.
	merkleLeafPrefix = []byte{0x00}
	merkleNodePrefix = []byte{0x01}
)

// Link is one provenance link, with its optional fields as pointers so absent
// stays distinct from present-and-empty (§6.5.1).
type Link struct {
	// Type is the link kind ("file", "derivation", …); always present.
	Type string
	// URI, Range, Digest, Method and By are the five optional fields, in the
	// normative encoding order.
	URI    *string
	Range  *string
	Digest *string
	Method *string
	By     *string
}

// Str is a convenience for building an optional field.
func Str(s string) *string { return &s }

// LinkFromProvenance converts a wire link.
//
// It collapses an empty string to absent, because contextgraph.Provenance
// cannot represent the difference: its fields are string with omitempty, so a
// present-but-empty URI never survives a JSON round trip in the first place.
// A verifier handling frames produced elsewhere — where `"uri": ""` is
// representable — must decode into [Link] directly rather than through this
// helper, or it will compute a chain head the signer did not.
func LinkFromProvenance(p contextgraph.Provenance) Link {
	optional := func(s string) *string {
		if s == "" {
			return nil
		}
		return &s
	}
	return Link{
		Type:   p.Type,
		URI:    optional(p.URI),
		Range:  optional(p.Range),
		Digest: optional(p.Digest),
		Method: optional(p.Method),
		By:     optional(p.By),
	}
}

// Frame is the part of a frame a commitment covers.
//
// Only these three fields enter the preimage, so a caller holding a frame from
// elsewhere does not have to fabricate a Score and a TokenCost to compute a
// commitment.
type Frame struct {
	ID            string
	ContentDigest *string
	Provenance    []Link
}

// InclusionStep is one step of an [InclusionProof]: the sibling hash and which
// side it sits on.
type InclusionStep struct {
	// Sibling is the sibling subtree hash, "sha256:<hex>".
	Sibling string `json:"sibling"`
	// SiblingIsLeft reports whether the sibling is the left operand here.
	SiblingIsLeft bool `json:"sibling_is_left"`
}

// InclusionProof proves one frame commitment is a leaf of a signed
// [MerkleRoot] (§6.5.3).
type InclusionProof struct {
	// LeafIndex is the leaf's index in canonical order.
	LeafIndex int `json:"leaf_index"`
	// LeafCount is how many leaves the tree held. Part of the proof because a
	// root alone does not pin the tree's size.
	LeafCount int `json:"leaf_count"`
	// Path holds the sibling hashes from the leaf upward.
	Path []InclusionStep `json:"path"`
}

// ProvenanceAttestation is a detached signature binding one frame's provenance
// to a signing identity (§6.5).
type ProvenanceAttestation struct {
	// SignedCommitment is the "sha256:<hex>" commitment this signs.
	SignedCommitment string `json:"signed_commitment"`
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
func (a ProvenanceAttestation) UsesKnownAlgorithm() bool {
	return a.Algorithm == AlgorithmEd25519
}

// Verdict is the outcome of checking a [ProvenanceAttestation] (§6.5.4).
//
// Every failure is named. §6.5.4 requires a verifier to distinguish them: "the
// frame changed after signing" and "the key is wrong" send an operator in
// opposite directions, and F8 treats "I cannot check this" as a third answer
// again — so a bool is not an acceptable return type here.
type Verdict string

// The named outcomes a [Verdict] can carry.
const (
	VerdictValid               Verdict = "valid"
	VerdictCommitmentMismatch  Verdict = "commitment_mismatch"
	VerdictBadSignature        Verdict = "bad_signature"
	VerdictUnknownAlgorithm    Verdict = "unknown_algorithm"
	VerdictMalformedKey        Verdict = "malformed_key"
	VerdictMalformedSignature  Verdict = "malformed_signature"
	VerdictMalformedCommitment Verdict = "malformed_commitment"
)

// Result carries a [Verdict] plus the detail the named outcome needs.
type Result struct {
	Verdict Verdict
	// Expected and Signed are set for VerdictCommitmentMismatch: the
	// commitment recomputed from the frame in hand, and the one the
	// attestation claims to sign.
	Expected string
	Signed   string
	// Algorithm is set for VerdictUnknownAlgorithm.
	Algorithm string
}

// IsValid reports whether the signature verified.
//
// No other verdict is provisionally acceptable: the point of an attestation is
// that "I could not check it" and "it is good" are never the same answer.
func (r Result) IsValid() bool { return r.Verdict == VerdictValid }

// ---------------------------------------------------------------------------
// Canonical encoding (§6.5.1)
// ---------------------------------------------------------------------------

// encString appends uint32be(utf8_byte_length(s)) || utf8(s).
//
// A Go string is already UTF-8 bytes, so len(s) is the right number here where
// it would be wrong in JavaScript or Python — which is exactly why this is
// spelled out rather than left as an incidental len call.
func encString(out *bytes.Buffer, s string) {
	if uint64(len(s)) > uint64(^uint32(0)) {
		// Unreachable on any real input, and a silent truncation to 32 bits
		// would be a divergence rather than an overflow.
		panic(fmt.Sprintf(
			"a provenance field of %d bytes overflows the §6.5.1 uint32 length prefix",
			len(s)))
	}
	var prefix [4]byte
	// Big-endian, normative. binary.BigEndian rather than a host-order write,
	// because the two differ on every machine this SDK runs on.
	binary.BigEndian.PutUint32(prefix[:], uint32(len(s)))
	out.Write(prefix[:])
	out.WriteString(s)
}

// encOptional appends 0x00 for absent, 0x01 || encString for present.
//
// The presence byte is what keeps absent distinct from empty. Without it a nil
// URI and a pointer to "" would encode identically.
func encOptional(out *bytes.Buffer, s *string) {
	if s == nil {
		out.WriteByte(0x00)
		return
	}
	out.WriteByte(0x01)
	encString(out, *s)
}

// EncodeLink returns the canonical encoding of one provenance link (§6.5.1).
//
// Field order is normative: type, uri, range, digest, method, by.
func EncodeLink(link Link) []byte {
	var out bytes.Buffer
	encString(&out, link.Type)
	encOptional(&out, link.URI)
	encOptional(&out, link.Range)
	encOptional(&out, link.Digest)
	encOptional(&out, link.Method)
	encOptional(&out, link.By)
	return out.Bytes()
}

func sha256Of(parts ...[]byte) [32]byte {
	h := sha256.New()
	for _, part := range parts {
		h.Write(part)
	}
	var out [32]byte
	copy(out[:], h.Sum(nil))
	return out
}

// DigestString renders 32 raw bytes as this protocol's "sha256:<hex>" form.
func DigestString(raw [32]byte) string {
	return "sha256:" + hex.EncodeToString(raw[:])
}

// decodeStrictHex decodes lowercase hex, or reports false.
//
// Strict on purpose. encoding/hex accepts uppercase, and the protocol's
// grammar is lowercase (contextgraph_types::is_well_formed_digest). One
// spelling per value is what keeps two implementations from disagreeing about
// whether a given attestation is well-formed.
func decodeStrictHex(text string) ([]byte, bool) {
	if text != strings.ToLower(text) {
		return nil, false
	}
	raw, err := hex.DecodeString(text)
	if err != nil {
		return nil, false
	}
	return raw, true
}

// ParseDigest parses a "sha256:<hex>" digest string. ok is false if malformed.
func ParseDigest(digest string) (out [32]byte, ok bool) {
	rest, found := strings.CutPrefix(digest, "sha256:")
	if !found || len(rest) != 64 {
		return out, false
	}
	raw, decoded := decodeStrictHex(rest)
	if !decoded {
		return out, false
	}
	copy(out[:], raw)
	return out, true
}

// ---------------------------------------------------------------------------
// Chain head, frame commitment, Merkle tree (§6.5.2–§6.5.3)
// ---------------------------------------------------------------------------

// ChainHead returns the head of a frame's provenance hash chain (§6.5.2).
//
// Links fold source-first, in the order §6 requires them to be carried, so
// each step consumes the previous head and no link can be inserted, dropped,
// reordered or edited without changing the result. An empty chain hashes to
// the genesis value rather than to zero, so "no provenance" is a stated claim
// a signature can cover.
func ChainHead(links []Link) [32]byte {
	head := sha256Of(domainGenesis)
	for _, link := range links {
		head = sha256Of(domainLink, head[:], EncodeLink(link))
	}
	return head
}

// FrameCommitment binds one frame's identity to its provenance chain (§6.5.2).
//
// The (providerID, frame id, content digest) triple is not optional: two
// frames citing the same source share a chain head, so a signature over the
// head alone lifts from one frame onto another.
func FrameCommitment(providerID string, frame Frame) [32]byte {
	var preimage bytes.Buffer
	encString(&preimage, providerID)
	encString(&preimage, frame.ID)
	encOptional(&preimage, frame.ContentDigest)
	chainHead := ChainHead(frame.Provenance)
	return sha256Of(domainFrame, preimage.Bytes(), chainHead[:])
}

func leafHash(commitment [32]byte) [32]byte {
	return sha256Of(merkleLeafPrefix, commitment[:])
}

func nodeHash(left, right [32]byte) [32]byte {
	return sha256Of(merkleNodePrefix, left[:], right[:])
}

// splitPoint is the largest power of two strictly less than n (RFC 6962's
// split). Only meaningful for n >= 2.
func splitPoint(n int) int {
	k := 1
	for k*2 < n {
		k *= 2
	}
	return k
}

// MerkleRoot returns the root over a set of frame commitments (§6.5.3).
//
// RFC 6962's shape, not the "duplicate the last leaf on an odd level"
// shortcut, which admits two distinct leaf sets with the same root. The two
// agree on any power-of-two leaf count, which is why the published vectors
// include three and seven.
func MerkleRoot(commitments [][32]byte) [32]byte {
	switch len(commitments) {
	case 0:
		return sha256Of(domainMerkleEmpty)
	case 1:
		return leafHash(commitments[0])
	default:
		k := splitPoint(len(commitments))
		return nodeHash(MerkleRoot(commitments[:k]), MerkleRoot(commitments[k:]))
	}
}

// BuildInclusionProof builds a proof for leafIndex. ok is false if the index is
// out of range.
func BuildInclusionProof(commitments [][32]byte, leafIndex int) (proof InclusionProof, ok bool) {
	if leafIndex < 0 || leafIndex >= len(commitments) {
		return proof, false
	}
	path := make([]InclusionStep, 0)
	collectPath(commitments, leafIndex, &path)
	return InclusionProof{
		LeafIndex: leafIndex,
		LeafCount: len(commitments),
		Path:      path,
	}, true
}

// collectPath walks down the tree accumulating sibling hashes, leaf-upward.
func collectPath(commitments [][32]byte, index int, path *[]InclusionStep) {
	if len(commitments) <= 1 {
		return
	}
	k := splitPoint(len(commitments))
	if index < k {
		collectPath(commitments[:k], index, path)
		*path = append(*path, InclusionStep{
			Sibling:       DigestString(MerkleRoot(commitments[k:])),
			SiblingIsLeft: false,
		})
		return
	}
	collectPath(commitments[k:], index-k, path)
	*path = append(*path, InclusionStep{
		Sibling:       DigestString(MerkleRoot(commitments[:k])),
		SiblingIsLeft: true,
	})
}

// RootFromProof recomputes a Merkle root from a leaf commitment and its proof.
//
// The whole offline story: an auditor holding one frame, its proof and a
// signed root needs nothing else. ok is false if any sibling is malformed, or
// the index does not sit inside the stated leaf count.
func RootFromProof(commitment [32]byte, proof InclusionProof) (root [32]byte, ok bool) {
	if proof.LeafIndex < 0 || proof.LeafIndex >= proof.LeafCount {
		return root, false
	}
	acc := leafHash(commitment)
	for _, step := range proof.Path {
		sibling, parsed := ParseDigest(step.Sibling)
		if !parsed {
			return root, false
		}
		if step.SiblingIsLeft {
			acc = nodeHash(sibling, acc)
		} else {
			acc = nodeHash(acc, sibling)
		}
	}
	return acc, true
}

// ---------------------------------------------------------------------------
// Verification (§6.5.4)
// ---------------------------------------------------------------------------

// smallOrderPublicKeys holds the eight canonical encodings of a point P with
// 8P = identity.
//
// §6.5.4 asks for a verifier that rejects small-order public keys, and Go's
// crypto/ed25519 does not — it checks a canonical S and stops there. A
// signature under a small-order key verifies against arbitrary messages, so
// accepting one turns "this attestation is valid" into a statement about
// nothing. Published in tests/vectors/attestation-vectors.json under
// verifier_strictness, where the Python suite recomputes 8P = identity for
// every entry from its own field arithmetic rather than trusting the list.
var smallOrderPublicKeys = map[string]struct{}{
	"0000000000000000000000000000000000000000000000000000000000000000": {},
	"0000000000000000000000000000000000000000000000000000000000000080": {},
	"0100000000000000000000000000000000000000000000000000000000000000": {},
	"26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05": {},
	"26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85": {},
	"c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a": {},
	"c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa": {},
	"ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f": {},
}

// fieldPrime is 2^255 - 19. A key whose y reaches it is not canonically
// encoded, and a verifier that reduces would read p and p+1 as the small-order
// points y = 0 and y = 1.
var fieldPrime = new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 255), big.NewInt(19))

// usableVerificationKey reports whether a strict verifier will use this key.
func usableVerificationKey(key []byte) bool {
	if len(key) != ed25519.PublicKeySize {
		return false
	}
	if _, weak := smallOrderPublicKeys[hex.EncodeToString(key)]; weak {
		return false
	}
	// y is the little-endian value with the sign bit masked off.
	reversed := make([]byte, len(key))
	for i, b := range key {
		reversed[len(key)-1-i] = b
	}
	reversed[0] &= 0x7f
	return new(big.Int).SetBytes(reversed).Cmp(fieldPrime) < 0
}

// VerifyCommitment checks a detached attestation over an already-computed
// commitment — a [MerkleRoot] for a result set, or a [FrameCommitment].
//
// Pure and offline: a commitment, an attestation and a public key are
// sufficient.
func VerifyCommitment(expected [32]byte, attestation ProvenanceAttestation, publicKey []byte) Result {
	if attestation.Algorithm != AlgorithmEd25519 {
		return Result{Verdict: VerdictUnknownAlgorithm, Algorithm: attestation.Algorithm}
	}
	signed, ok := ParseDigest(attestation.SignedCommitment)
	if !ok {
		return Result{Verdict: VerdictMalformedCommitment}
	}
	// Compare commitments before touching the signature. A mismatch means the
	// frame changed after signing, and reporting that as a bad signature sends
	// an operator hunting a key-management bug when the finding is tampering.
	if signed != expected {
		return Result{
			Verdict:  VerdictCommitmentMismatch,
			Expected: DigestString(expected),
			Signed:   attestation.SignedCommitment,
		}
	}
	if !usableVerificationKey(publicKey) {
		return Result{Verdict: VerdictMalformedKey}
	}
	signature, decoded := decodeStrictHex(attestation.Signature)
	if !decoded || len(signature) != ed25519.SignatureSize {
		return Result{Verdict: VerdictMalformedSignature}
	}
	if ed25519.Verify(ed25519.PublicKey(publicKey), signed[:], signature) {
		return Result{Verdict: VerdictValid}
	}
	return Result{Verdict: VerdictBadSignature}
}

// VerifyFrameAttestation checks a detached attestation over a single frame
// (§6.5.4).
func VerifyFrameAttestation(providerID string, frame Frame, attestation ProvenanceAttestation, publicKey []byte) Result {
	return VerifyCommitment(FrameCommitment(providerID, frame), attestation, publicKey)
}
