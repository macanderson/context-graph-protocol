// The record layer, reconciled against the published lifecycle vectors
// (issue #119).
//
// Every expected value is read from tests/fixtures/: record-hash-vectors.json
// for each fixture's canonical text and hash, record-attestation.json for a
// real signature, and record-attestation-key.json for the key that made it.
// contextgraph-conformance's lifecycle_profile_examples suite recomputes the
// same values from the Rust reference, so this suite and that one are held to
// the same bytes.
package attest

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/macanderson/context-graph-protocol/sdk/go/contextgraph/jcs"
)

// readFixture walks up from the working directory until tests/fixtures/<name>
// appears, for the reason loadVectors does.
func readFixture(t *testing.T, name string) []byte {
	t.Helper()
	dir, err := os.Getwd()
	if err != nil {
		t.Fatalf("working directory: %v", err)
	}
	for i := 0; i < 10; i++ {
		if raw, err := os.ReadFile(filepath.Join(dir, "tests", "fixtures", name)); err == nil {
			return raw
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	t.Fatalf("tests/fixtures/%s not found above the test directory", name)
	return nil
}

type recordHashVectors struct {
	Vectors []struct {
		RecordFile string `json:"record_file"`
		JCSUTF8    string `json:"jcs_utf8"`
		RecordHash string `json:"record_hash"`
	} `json:"vectors"`
}

type recordAttestationKey struct {
	Algorithm        string `json:"algorithm"`
	KeyID            string `json:"key_id"`
	PublicKey        string `json:"public_key"`
	SignedMessageHex string `json:"signed_message_hex"`
	SigningKeySeed   string `json:"signing_key_seed"`
	Signs            string `json:"signs"`
}

func loadRecordHashVectors(t *testing.T) recordHashVectors {
	t.Helper()
	var v recordHashVectors
	if err := json.Unmarshal(readFixture(t, "record-hash-vectors.json"), &v); err != nil {
		t.Fatalf("record-hash-vectors.json: %v", err)
	}
	if len(v.Vectors) == 0 {
		t.Fatal("record-hash-vectors.json publishes no vectors")
	}
	return v
}

func loadRecordAttestation(t *testing.T) (RecordAttestation, recordAttestationKey) {
	t.Helper()
	var attestation RecordAttestation
	decoder := json.NewDecoder(bytes.NewReader(readFixture(t, "record-attestation.json")))
	// Strict, so a json tag spelled differently from the wire fails here
	// rather than decoding to an empty field.
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&attestation); err != nil {
		t.Fatalf("record-attestation.json: %v", err)
	}
	var key recordAttestationKey
	if err := json.Unmarshal(readFixture(t, "record-attestation-key.json"), &key); err != nil {
		t.Fatalf("record-attestation-key.json: %v", err)
	}
	return attestation, key
}

// edit decodes a record, applies change, and re-encodes it.
func edit(t *testing.T, record []byte, change func(map[string]any)) []byte {
	t.Helper()
	var members map[string]any
	if err := json.Unmarshal(record, &members); err != nil {
		t.Fatal(err)
	}
	change(members)
	out, err := json.Marshal(members)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func mustRecordHash(t *testing.T, record []byte) string {
	t.Helper()
	hash, err := RecordHash(record)
	if err != nil {
		t.Fatalf("record_hash: %v", err)
	}
	return hash
}

// TestRecordHashReproducesEveryPublishedVector is the witness #119 names: the
// canonical text and the hash, for every fixture. The text is asserted first
// because when a hash disagrees, the text says where.
func TestRecordHashReproducesEveryPublishedVector(t *testing.T) {
	for _, vector := range loadRecordHashVectors(t).Vectors {
		record := readFixture(t, vector.RecordFile)
		preimage, err := RecordHashPreimage(record)
		if err != nil {
			t.Errorf("%s: %v", vector.RecordFile, err)
			continue
		}
		if string(preimage) != vector.JCSUTF8 {
			t.Errorf("%s canonical text\n got %s\nwant %s", vector.RecordFile, preimage, vector.JCSUTF8)
		}
		if got := mustRecordHash(t, record); got != vector.RecordHash {
			t.Errorf("%s record_hash\n got %s\nwant %s", vector.RecordFile, got, vector.RecordHash)
		}
		current, err := RecordHashIsCurrent(record)
		if err != nil || !current {
			t.Errorf("%s carries its own record_hash, which must be current: %v, %v", vector.RecordFile, current, err)
		}
	}
}

// TestThePublishedRecordAttestationVerifies is the second witness #119 names.
func TestThePublishedRecordAttestationVerifies(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	record := readFixture(t, key.Signs)
	result, err := VerifyRecordAttestation(record, attestation, mustHex(t, key.PublicKey))
	if err != nil {
		t.Fatalf("%s: %v", key.Signs, err)
	}
	if !result.IsValid() {
		t.Fatalf("the published record attestation must verify, got %+v", result)
	}
	if !attestation.UsesKnownAlgorithm() || attestation.KeyID != key.KeyID {
		t.Errorf("the attestation and its key file disagree: %+v", attestation)
	}
}

func TestTheSignedMessageIsTheDomainTagThenTheRawDigest(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	message, err := RecordAttestationMessage(attestation.SignedRecordHash)
	if err != nil {
		t.Fatal(err)
	}
	if got := hex.EncodeToString(message); got != key.SignedMessageHex {
		t.Errorf("signed message\n got %s\nwant %s", got, key.SignedMessageHex)
	}
	if !bytes.HasPrefix(message, []byte("contextgraph/attest/1/record")) || len(message) != len(RecordAttestationDomain)+32 {
		t.Errorf("the message is the 28-byte tag and 32 raw digest bytes, got %d bytes", len(message))
	}
}

// TestSigningThePublishedRecordReproducesThePublishedAttestation pins record
// signing (#127) to the published record vector, byte for byte.
func TestSigningThePublishedRecordReproducesThePublishedAttestation(t *testing.T) {
	want, key := loadRecordAttestation(t)
	signer, err := PrivateKeyFromSeed(mustHex(t, key.SigningKeySeed))
	if err != nil {
		t.Fatal(err)
	}
	public, err := PublicKeyFromSeed(mustHex(t, key.SigningKeySeed))
	if err != nil {
		t.Fatal(err)
	}
	if hex.EncodeToString(public) != key.PublicKey {
		t.Errorf("public key\n got %x\nwant %s", []byte(public), key.PublicKey)
	}
	got, err := SignRecord(readFixture(t, key.Signs), signer, want.KeyID, want.AttesterID, want.IssuedAt)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Errorf("record attestation\n got %+v\nwant %+v", got, want)
	}
}

func TestARecordsOwnHashIsRemovedNotBlanked(t *testing.T) {
	record := readFixture(t, "observation.json")
	absent := edit(t, record, func(m map[string]any) { delete(m, RecordHashMember) })
	wrong := edit(t, record, func(m map[string]any) { m[RecordHashMember] = "sha256:" + strings.Repeat("f", 64) })
	hash := mustRecordHash(t, record)
	if mustRecordHash(t, absent) != hash || mustRecordHash(t, wrong) != hash {
		t.Error("a record must hash identically with no record_hash, the right one, or a wrong one")
	}
	preimage, err := RecordHashPreimage(record)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(preimage, []byte(RecordHashMember)) {
		t.Errorf("a record must never hash over its own hash: %s", preimage)
	}
	if current, _ := RecordHashIsCurrent(wrong); current {
		t.Error("a wrong stored hash is not current")
	}
	if current, err := RecordHashIsCurrent(absent); current || err != nil {
		t.Errorf("an unhashed record is not current and not an error: %v, %v", current, err)
	}
}

func TestOnlyTheTopLevelHashMemberIsRemoved(t *testing.T) {
	// A record_hash nested inside an extension is ordinary content; dropping
	// it would let a producer hide a value from the signature.
	record := readFixture(t, "observation.json")
	nested := edit(t, record, func(m map[string]any) {
		m["extensions"] = map[string]any{"record_hash": "sha256:nested"}
	})
	preimage, err := RecordHashPreimage(nested)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(preimage, []byte("sha256:nested")) {
		t.Errorf("the nested member must stay in the preimage: %s", preimage)
	}
	if mustRecordHash(t, nested) == mustRecordHash(t, record) {
		t.Error("adding nested content must change the hash")
	}
}

func TestRecordHashOfAGoValueMatchesTheBytes(t *testing.T) {
	record := readFixture(t, "observation.json")
	var members map[string]any
	if err := json.Unmarshal(record, &members); err != nil {
		t.Fatal(err)
	}
	got, err := RecordHashOf(members)
	if err != nil {
		t.Fatal(err)
	}
	if want := mustRecordHash(t, record); got != want {
		t.Errorf("RecordHashOf\n got %s\nwant %s", got, want)
	}
}

func TestARecordThatIsNotCanonicalizableIsAnError(t *testing.T) {
	cases := []struct {
		input string
		want  error
	}{
		{`[1,2,3]`, jcs.ErrNotObject},
		{`{"record_id":"a","record_id":"b"}`, jcs.ErrDuplicateMember},
		{`{"statement":"\ud800"}`, jcs.ErrLoneSurrogate},
		{`{"confidence":1e400}`, jcs.ErrNumberOutOfRange},
		{`{"statement":`, jcs.ErrSyntax},
	}
	for _, c := range cases {
		if _, err := RecordHash([]byte(c.input)); !errors.Is(err, c.want) {
			t.Errorf("%s: got %v, want %v", c.input, err, c.want)
		}
		if _, err := VerifyRecordAttestation([]byte(c.input), RecordAttestation{}, nil); !errors.Is(err, c.want) {
			t.Errorf("verifying %s: got %v, want %v", c.input, err, c.want)
		}
	}
}

func TestEditingARecordAfterSigningIsAMismatch(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	tampered := edit(t, readFixture(t, key.Signs), func(m map[string]any) {
		m["statement"] = "the api handler never retries"
	})
	result, err := VerifyRecordAttestation(tampered, attestation, mustHex(t, key.PublicKey))
	if err != nil {
		t.Fatal(err)
	}
	if result.Verdict != VerdictCommitmentMismatch {
		t.Fatalf("got %s, want %s", result.Verdict, VerdictCommitmentMismatch)
	}
	if result.Signed != attestation.SignedRecordHash || result.Expected != mustRecordHash(t, tampered) {
		t.Errorf("the mismatch must name both hashes: %+v", result)
	}
}

// TestRewritingTheStoredHashDoesNotLaunderATamperedRecord is profile LC5's
// reason to exist: edit the content, then rewrite record_hash so the record is
// internally consistent again. Verifying against the stored member would pass
// it; verifying against the recomputed one cannot.
func TestRewritingTheStoredHashDoesNotLaunderATamperedRecord(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	laundered := edit(t, readFixture(t, key.Signs), func(m map[string]any) {
		m["statement"] = "the api handler never retries"
	})
	restated := mustRecordHash(t, laundered)
	laundered = edit(t, laundered, func(m map[string]any) { m[RecordHashMember] = restated })
	if current, err := RecordHashIsCurrent(laundered); err != nil || !current {
		t.Fatalf("precondition: the laundered record is internally consistent: %v, %v", current, err)
	}
	result, err := VerifyRecordAttestation(laundered, attestation, mustHex(t, key.PublicKey))
	if err != nil {
		t.Fatal(err)
	}
	if result.Verdict != VerdictCommitmentMismatch {
		t.Errorf("got %s, want %s", result.Verdict, VerdictCommitmentMismatch)
	}
}

func TestAWrongKeyIsABadSignatureNotAMismatch(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	v := loadVectors(t)
	// A real, usable key that did not sign this record.
	other := mustHex(t, v.Signature.PublicKeyHex)
	result, err := VerifyRecordAttestation(readFixture(t, key.Signs), attestation, other)
	if err != nil {
		t.Fatal(err)
	}
	if result.Verdict != VerdictBadSignature {
		t.Errorf("the hash is intact and only the key is wrong: got %s", result.Verdict)
	}
}

// TestAFrameSignatureOverTheSameDigestIsNotARecordAttestation is what the
// domain tag buys: sign the record's own 32 hash bytes at the frame layer, and
// the signature must still not verify as a record attestation.
func TestAFrameSignatureOverTheSameDigestIsNotARecordAttestation(t *testing.T) {
	want, key := loadRecordAttestation(t)
	signer, err := PrivateKeyFromSeed(mustHex(t, key.SigningKeySeed))
	if err != nil {
		t.Fatal(err)
	}
	frameSigned, err := SignCommitment(mustDigest(t, want.SignedRecordHash), signer, want.KeyID, want.AttesterID, want.IssuedAt)
	if err != nil {
		t.Fatal(err)
	}
	lifted := RecordAttestation{
		SignedRecordHash: frameSigned.SignedCommitment,
		KeyID:            frameSigned.KeyID,
		Algorithm:        frameSigned.Algorithm,
		AttesterID:       frameSigned.AttesterID,
		Signature:        frameSigned.Signature,
		IssuedAt:         frameSigned.IssuedAt,
	}
	result, err := VerifyRecordAttestation(readFixture(t, key.Signs), lifted, mustHex(t, key.PublicKey))
	if err != nil {
		t.Fatal(err)
	}
	if result.Verdict != VerdictBadSignature {
		t.Errorf("a signature from another layer must not pass as a record attestation: got %s", result.Verdict)
	}
}

func TestEveryRecordVerificationFailureIsNamed(t *testing.T) {
	attestation, key := loadRecordAttestation(t)
	hash := attestation.SignedRecordHash
	public := mustHex(t, key.PublicKey)

	unknown := attestation
	unknown.Algorithm = "dilithium3"
	if got := VerifySignedRecordHash(hash, unknown, public); got.Verdict != VerdictUnknownAlgorithm || got.Algorithm != "dilithium3" {
		t.Errorf("unknown algorithm: got %+v", got)
	}

	for _, malformed := range []string{"not-a-digest", "sha256:short", "sha256:" + strings.ToUpper(strings.TrimPrefix(hash, "sha256:"))} {
		bad := attestation
		bad.SignedRecordHash = malformed
		if got := VerifySignedRecordHash(hash, bad, public); got.Verdict != VerdictMalformedCommitment {
			t.Errorf("signed hash %q: got %s", malformed, got.Verdict)
		}
	}

	for _, signature := range []string{"abcd", strings.ToUpper(attestation.Signature), "zz" + attestation.Signature[2:]} {
		bad := attestation
		bad.Signature = signature
		if got := VerifySignedRecordHash(hash, bad, public); got.Verdict != VerdictMalformedSignature {
			t.Errorf("signature %q: got %s", signature, got.Verdict)
		}
	}

	if got := VerifySignedRecordHash(hash, attestation, make([]byte, 5)); got.Verdict != VerdictMalformedKey {
		t.Errorf("malformed key: got %s", got.Verdict)
	}
	for _, weak := range loadVectors(t).VerifierStrictness.SmallOrderPublicKeysHex {
		if got := VerifySignedRecordHash(hash, attestation, mustHex(t, weak)); got.Verdict != VerdictMalformedKey {
			t.Errorf("small-order key %s: got %s", weak, got.Verdict)
		}
	}

	if _, err := RecordAttestationMessage("sha256:short"); !errors.Is(err, ErrMalformedDigest) {
		t.Errorf("a malformed digest has no message: got %v", err)
	}
	if _, err := SignRecordAttestation("sha256:short", ed25519.PrivateKey(nil), "k", "a", "2026-07-29T14:00:05Z"); !errors.Is(err, ErrMalformedDigest) {
		t.Errorf("a malformed digest cannot be signed: got %v", err)
	}
}

func TestARecordAttestationRoundTripsThroughJSON(t *testing.T) {
	attestation, _ := loadRecordAttestation(t)
	raw, err := json.Marshal(attestation)
	if err != nil {
		t.Fatal(err)
	}
	canonical, err := jcs.Canonicalize(raw)
	if err != nil {
		t.Fatal(err)
	}
	fixture, err := jcs.Canonicalize(readFixture(t, "record-attestation.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(canonical, fixture) {
		t.Errorf("re-encoding must reproduce the fixture\n got %s\nwant %s", canonical, fixture)
	}
}
