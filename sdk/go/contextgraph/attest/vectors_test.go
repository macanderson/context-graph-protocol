// The Go attestation port, reconciled against the published vectors.
//
// Every expected value is read from tests/vectors/attestation-vectors.json,
// which contextgraph-types/tests/attestation_vectors.rs mirrors and pins.
// Nothing here asserts against a value this file computed: a port that agrees
// with itself is what this suite exists to catch.
package attest

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

type vectorFile struct {
	Links             map[string]map[string]string `json:"links"`
	LinkEncodingsHex  map[string]string            `json:"link_encodings_hex"`
	UnicodeLengthTrap map[string]struct {
		UTF8Bytes      int `json:"utf8_bytes"`
		CodePoints     int `json:"code_points"`
		UTF16CodeUnits int `json:"utf16_code_units"`
	} `json:"unicode_length_trap"`
	ChainHeads      map[string]string `json:"chain_heads"`
	FrameCommitment struct {
		ProviderID string `json:"provider_id"`
		Frame      struct {
			ID            string   `json:"id"`
			ContentDigest string   `json:"content_digest"`
			Provenance    []string `json:"provenance"`
		} `json:"frame"`
		Commitment string `json:"commitment"`
	} `json:"frame_commitment"`
	Merkle struct {
		ProviderID string `json:"provider_id"`
		LeafFrames []struct {
			ID            string   `json:"id"`
			ContentDigest string   `json:"content_digest"`
			Provenance    []string `json:"provenance"`
		} `json:"leaf_frames"`
		RootsByLeafCount map[string]string `json:"roots_by_leaf_count"`
		InclusionProof   struct {
			LeafCount int             `json:"leaf_count"`
			LeafIndex int             `json:"leaf_index"`
			Path      []InclusionStep `json:"path"`
		} `json:"inclusion_proof"`
	} `json:"merkle"`
	Signature struct {
		PublicKeyHex string                `json:"public_key_hex"`
		Attestation  ProvenanceAttestation `json:"attestation"`
	} `json:"signature"`
	VerifierStrictness struct {
		SmallOrderPublicKeysHex   []string `json:"small_order_public_keys_hex"`
		NonCanonicalPublicKeysHex []string `json:"non_canonical_public_keys_hex"`
	} `json:"verifier_strictness"`
}

// loadVectors walks up from the working directory until the shared fixture
// appears, rather than counting ".." segments — `go test ./...` runs each
// package in its own directory and a fixed depth is right for exactly one.
func loadVectors(t *testing.T) vectorFile {
	t.Helper()
	dir, err := os.Getwd()
	if err != nil {
		t.Fatalf("working directory: %v", err)
	}
	for i := 0; i < 10; i++ {
		candidate := filepath.Join(dir, "tests", "vectors", "attestation-vectors.json")
		if raw, err := os.ReadFile(candidate); err == nil {
			var v vectorFile
			if err := json.Unmarshal(raw, &v); err != nil {
				t.Fatalf("%s: %v", candidate, err)
			}
			return v
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	t.Fatal("tests/vectors/attestation-vectors.json not found above the test directory")
	return vectorFile{}
}

// link builds a Link from the fixture. A field absent from the JSON object is
// the encoding's None; the fixture never carries an explicit empty string, and
// this decoder would not be able to tell one from absent if it did — which is
// exactly why encoding/json is not on the encoding path.
func link(v vectorFile, name string) Link {
	obj := v.Links[name]
	optional := func(key string) *string {
		value, present := obj[key]
		if !present {
			return nil
		}
		return &value
	}
	return Link{
		Type:   obj["type"],
		URI:    optional("uri"),
		Range:  optional("range"),
		Digest: optional("digest"),
		Method: optional("method"),
		By:     optional("by"),
	}
}

func merkleLeaves(v vectorFile, count int) [][32]byte {
	out := make([][32]byte, 0, count)
	for _, spec := range v.Merkle.LeafFrames[:count] {
		digest := spec.ContentDigest
		out = append(out, FrameCommitment(v.Merkle.ProviderID, Frame{
			ID:            spec.ID,
			ContentDigest: &digest,
		}))
	}
	return out
}

func mustDigest(t *testing.T, s string) [32]byte {
	t.Helper()
	out, ok := ParseDigest(s)
	if !ok {
		t.Fatalf("%q is not a well-formed digest", s)
	}
	return out
}

func mustHex(t *testing.T, s string) []byte {
	t.Helper()
	raw, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("%q is not hex: %v", s, err)
	}
	return raw
}

func TestLinkEncodingMatchesThePublishedBytes(t *testing.T) {
	v := loadVectors(t)
	for _, name := range []string{"ascii_minimal", "unicode"} {
		got := hex.EncodeToString(EncodeLink(link(v, name)))
		if want := v.LinkEncodingsHex[name]; got != want {
			t.Errorf("%s link encoding\n got %s\nwant %s", name, got, want)
		}
	}
}

func TestTheLengthPrefixCountsUTF8Bytes(t *testing.T) {
	v := loadVectors(t)
	// A Go string is already UTF-8, so this is the one language of the three
	// where the native length is right. Asserted anyway, because the claim
	// "Go cannot get this wrong" is worth an oracle rather than a belief.
	uri := *link(v, "unicode").URI
	by := *link(v, "unicode").By
	trap := v.UnicodeLengthTrap
	if len(uri) != trap["uri"].UTF8Bytes {
		t.Errorf("uri: len() = %d, the fixture says %d UTF-8 bytes", len(uri), trap["uri"].UTF8Bytes)
	}
	if utf8.RuneCountInString(uri) != trap["uri"].CodePoints {
		t.Errorf("uri: %d runes, the fixture says %d", utf8.RuneCountInString(uri), trap["uri"].CodePoints)
	}
	if len(by) != trap["by"].UTF8Bytes {
		t.Errorf("by: len() = %d, the fixture says %d UTF-8 bytes", len(by), trap["by"].UTF8Bytes)
	}
	if trap["by"].UTF8Bytes == trap["by"].UTF16CodeUnits {
		t.Error("the astral character must make the two answers differ, or the vector proves nothing")
	}
}

func TestAnAbsentFieldNeverEncodesLikeAnEmptyOne(t *testing.T) {
	absent := Link{Type: "file"}
	empty := Link{Type: "file", URI: Str("")}
	if string(EncodeLink(absent)) == string(EncodeLink(empty)) {
		t.Error("the presence byte must keep absent distinct from empty")
	}
}

func TestLinkFromProvenanceStatesItsCollapse(t *testing.T) {
	// The wire struct cannot represent a present-but-empty URI, and the
	// helper says so by producing the same link an absent URI would.
	wire := contextgraph.Provenance{Type: "file", URI: ""}
	if string(EncodeLink(LinkFromProvenance(wire))) != string(EncodeLink(Link{Type: "file"})) {
		t.Error("LinkFromProvenance is documented to collapse empty to absent")
	}
	populated := contextgraph.Provenance{Type: "file", URI: "src/retry.rs"}
	if LinkFromProvenance(populated).URI == nil {
		t.Error("a non-empty field must survive the conversion")
	}
}

func TestChainHeadsMatchThePublishedVectors(t *testing.T) {
	v := loadVectors(t)
	cases := []struct {
		key   string
		links []Link
	}{
		{"empty", nil},
		{"file", []Link{link(v, "file")}},
		{"file_then_derivation", []Link{link(v, "file"), link(v, "derivation")}},
		{"unicode", []Link{link(v, "unicode")}},
	}
	for _, c := range cases {
		if got, want := DigestString(ChainHead(c.links)), v.ChainHeads[c.key]; got != want {
			t.Errorf("chain head %s\n got %s\nwant %s", c.key, got, want)
		}
	}
}

func TestReorderingTheChainChangesTheHead(t *testing.T) {
	v := loadVectors(t)
	forward := ChainHead([]Link{link(v, "file"), link(v, "derivation")})
	reversed := ChainHead([]Link{link(v, "derivation"), link(v, "file")})
	if forward == reversed {
		t.Error("a hash chain must bind order; per-link digests never did")
	}
}

func TestFrameCommitmentMatchesThePublishedVector(t *testing.T) {
	v := loadVectors(t)
	spec := v.FrameCommitment
	links := make([]Link, 0, len(spec.Frame.Provenance))
	for _, name := range spec.Frame.Provenance {
		links = append(links, link(v, name))
	}
	digest := spec.Frame.ContentDigest
	got := DigestString(FrameCommitment(spec.ProviderID, Frame{
		ID:            spec.Frame.ID,
		ContentDigest: &digest,
		Provenance:    links,
	}))
	if got != spec.Commitment {
		t.Errorf("frame commitment\n got %s\nwant %s", got, spec.Commitment)
	}
}

func TestMerkleRootsMatchThePublishedVectors(t *testing.T) {
	v := loadVectors(t)
	for count, want := range v.Merkle.RootsByLeafCount {
		n, err := strconv.Atoi(count)
		if err != nil {
			t.Fatalf("leaf count %q: %v", count, err)
		}
		if got := DigestString(MerkleRoot(merkleLeaves(v, n))); got != want {
			t.Errorf("%s-leaf root\n got %s\nwant %s", count, got, want)
		}
	}
}

func TestInclusionProofMatchesAndRecomputesTheRoot(t *testing.T) {
	v := loadVectors(t)
	spec := v.Merkle.InclusionProof
	leaves := merkleLeaves(v, spec.LeafCount)
	proof, ok := BuildInclusionProof(leaves, spec.LeafIndex)
	if !ok {
		t.Fatalf("index %d of %d is in range", spec.LeafIndex, spec.LeafCount)
	}
	if proof.LeafCount != spec.LeafCount || proof.LeafIndex != spec.LeafIndex {
		t.Fatalf("proof shape: got (%d, %d)", proof.LeafIndex, proof.LeafCount)
	}
	if len(proof.Path) != len(spec.Path) {
		t.Fatalf("path length: got %d, want %d", len(proof.Path), len(spec.Path))
	}
	for i, step := range proof.Path {
		if step != spec.Path[i] {
			t.Errorf("path[%d]\n got %+v\nwant %+v", i, step, spec.Path[i])
		}
	}
	root, ok := RootFromProof(leaves[spec.LeafIndex], proof)
	if !ok {
		t.Fatal("a well-formed proof must recompute a root")
	}
	if got, want := DigestString(root), v.Merkle.RootsByLeafCount["7"]; got != want {
		t.Errorf("recomputed root\n got %s\nwant %s", got, want)
	}
}

func TestAProofDoesNotValidateAnOutsider(t *testing.T) {
	v := loadVectors(t)
	leaves := merkleLeaves(v, 7)
	proof, ok := BuildInclusionProof(leaves, 3)
	if !ok {
		t.Fatal("index 3 of 7 is in range")
	}
	outsider := FrameCommitment("repo-graph", Frame{ID: "intruder"})
	root, ok := RootFromProof(outsider, proof)
	if !ok {
		t.Fatal("the proof is well-formed regardless of the leaf")
	}
	if DigestString(root) == v.Merkle.RootsByLeafCount["7"] {
		t.Error("an unsigned frame must not ride someone else's proof")
	}
}

func TestOutOfRangeIndexHasNoProof(t *testing.T) {
	v := loadVectors(t)
	if _, ok := BuildInclusionProof(merkleLeaves(v, 3), 3); ok {
		t.Error("index 3 of 3 is out of range")
	}
	if _, ok := BuildInclusionProof(nil, 0); ok {
		t.Error("an empty set has no leaves to prove")
	}
}

func TestThePublishedSignatureVerifies(t *testing.T) {
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	result := VerifyCommitment(commitment, v.Signature.Attestation, mustHex(t, v.Signature.PublicKeyHex))
	if !result.IsValid() {
		t.Fatalf("the published signature must verify, got %+v", result)
	}
	if !v.Signature.Attestation.UsesKnownAlgorithm() {
		t.Error("the published attestation names ed25519")
	}
}

func TestPerturbingTheCommitmentIsAMismatch(t *testing.T) {
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	commitment[0] ^= 0x01
	result := VerifyCommitment(commitment, v.Signature.Attestation, mustHex(t, v.Signature.PublicKeyHex))
	// §6.5.4's ordering rule: the frame changed after signing, and saying
	// "bad signature" would send an operator after a key-management bug.
	if result.Verdict != VerdictCommitmentMismatch {
		t.Fatalf("got %s, want %s", result.Verdict, VerdictCommitmentMismatch)
	}
	if result.Signed != v.Signature.Attestation.SignedCommitment {
		t.Error("the mismatch must name the commitment the attestation claims")
	}
}

func TestPerturbingTheSignatureIsABadSignature(t *testing.T) {
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	raw := mustHex(t, v.Signature.Attestation.Signature)
	raw[0] ^= 0x01
	forged := v.Signature.Attestation
	forged.Signature = hex.EncodeToString(raw)
	if got := VerifyCommitment(commitment, forged, mustHex(t, v.Signature.PublicKeyHex)); got.Verdict != VerdictBadSignature {
		t.Fatalf("got %s, want %s", got.Verdict, VerdictBadSignature)
	}
}

func TestPerturbingTheFrameIsCaughtThroughVerifyFrameAttestation(t *testing.T) {
	v := loadVectors(t)
	spec := v.FrameCommitment
	links := make([]Link, 0, len(spec.Frame.Provenance))
	for _, name := range spec.Frame.Provenance {
		links = append(links, link(v, name))
	}
	digest := spec.Frame.ContentDigest
	honest := Frame{ID: spec.Frame.ID, ContentDigest: &digest, Provenance: links}
	key := mustHex(t, v.Signature.PublicKeyHex)

	if got := VerifyFrameAttestation(spec.ProviderID, honest, v.Signature.Attestation, key); !got.IsValid() {
		t.Fatalf("precondition: the published attestation signs this frame, got %+v", got)
	}

	// The tamper a bare digest cannot see, because the tamperer rewrites the
	// digest too.
	tamperedLinks := append([]Link(nil), links...)
	tamperedLinks[0].URI = Str("src/evil.rs")
	tampered := Frame{ID: honest.ID, ContentDigest: honest.ContentDigest, Provenance: tamperedLinks}
	if got := VerifyFrameAttestation(spec.ProviderID, tampered, v.Signature.Attestation, key); got.Verdict != VerdictCommitmentMismatch {
		t.Errorf("rewriting a source URI: got %s", got.Verdict)
	}
	// And the identity binding: same bytes, different provider.
	if got := VerifyFrameAttestation("impostor", honest, v.Signature.Attestation, key); got.Verdict != VerdictCommitmentMismatch {
		t.Errorf("the provider id is part of the signed identity: got %s", got.Verdict)
	}
}

func TestEveryFailureIsNamed(t *testing.T) {
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	key := mustHex(t, v.Signature.PublicKeyHex)

	unknown := v.Signature.Attestation
	unknown.Algorithm = "dilithium3"
	if got := VerifyCommitment(commitment, unknown, key); got.Verdict != VerdictUnknownAlgorithm || got.Algorithm != "dilithium3" {
		t.Errorf("unknown algorithm: got %+v", got)
	}

	badCommitment := v.Signature.Attestation
	badCommitment.SignedCommitment = "not-a-digest"
	if got := VerifyCommitment(commitment, badCommitment, key); got.Verdict != VerdictMalformedCommitment {
		t.Errorf("malformed commitment: got %s", got.Verdict)
	}

	badSignature := v.Signature.Attestation
	badSignature.Signature = "abcd"
	if got := VerifyCommitment(commitment, badSignature, key); got.Verdict != VerdictMalformedSignature {
		t.Errorf("malformed signature: got %s", got.Verdict)
	}

	if got := VerifyCommitment(commitment, v.Signature.Attestation, make([]byte, 5)); got.Verdict != VerdictMalformedKey {
		t.Errorf("malformed key: got %s", got.Verdict)
	}
}

func TestHexIsAcceptedInExactlyOneSpelling(t *testing.T) {
	// The protocol's grammar is lowercase (is_well_formed_digest). Accepting
	// uppercase would mean two implementations disagreeing about whether the
	// same attestation is well-formed, which is the class of divergence this
	// whole port exists to close — and encoding/hex accepts it by default.
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	key := mustHex(t, v.Signature.PublicKeyHex)

	upperSignature := v.Signature.Attestation
	upperSignature.Signature = strings.ToUpper(upperSignature.Signature)
	if got := VerifyCommitment(commitment, upperSignature, key); got.Verdict != VerdictMalformedSignature {
		t.Errorf("an uppercase signature: got %s", got.Verdict)
	}

	upperCommitment := v.Signature.Attestation
	upperCommitment.SignedCommitment = "sha256:" +
		strings.ToUpper(strings.TrimPrefix(v.Signature.Attestation.SignedCommitment, "sha256:"))
	if _, ok := ParseDigest(upperCommitment.SignedCommitment); ok {
		t.Error("an uppercase digest is not a well-formed protocol digest")
	}
	if got := VerifyCommitment(commitment, upperCommitment, key); got.Verdict != VerdictMalformedCommitment {
		t.Errorf("an uppercase commitment: got %s", got.Verdict)
	}
}

func TestAStrictVerifierDeclinesWeakKeys(t *testing.T) {
	v := loadVectors(t)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	rejectable := append([]string(nil), v.VerifierStrictness.SmallOrderPublicKeysHex...)
	rejectable = append(rejectable, v.VerifierStrictness.NonCanonicalPublicKeysHex...)
	if len(rejectable) == 0 {
		t.Fatal("the fixture must publish the keys a strict verifier declines")
	}
	for _, hexKey := range rejectable {
		got := VerifyCommitment(commitment, v.Signature.Attestation, mustHex(t, hexKey))
		if got.Verdict != VerdictMalformedKey {
			t.Errorf("%s must not be usable as a verification key: got %s", hexKey, got.Verdict)
		}
	}
}
