package attest_test

import (
	"bytes"
	"crypto"
	"crypto/ed25519"
	"fmt"
	"io"
	"log"
	"strings"

	"github.com/macanderson/context-graph-protocol/sdk/go/contextgraph/attest"
)

// Signing a frame in-process, from a seed. Fine for tests and for a provider
// that has decided the cost is acceptable; the README's key-custody section
// says what that cost is.
func ExampleSignFrameAttestation() {
	// 32 bytes of 0x07: the published test seed from
	// tests/vectors/attestation-vectors.json. Never sign anything real with it.
	seed := bytes.Repeat([]byte{0x07}, 32)
	key, err := attest.PrivateKeyFromSeed(seed)
	if err != nil {
		log.Fatal(err)
	}

	digest := "sha256:" + strings.Repeat("ab", 32)
	frame := attest.Frame{
		ID:            "retry-policy",
		ContentDigest: &digest, // required: §6.5.2 and ADR 0018
		Provenance:    []attest.Link{{Type: "file", URI: attest.Str("src/retry.rs")}},
	}
	attestation, err := attest.SignFrameAttestation("repo-graph", frame, key, "key-1", "oxagen", "2026-08-27T00:00:00Z")
	if err != nil {
		log.Fatal(err)
	}

	public, err := attest.PublicKeyFromSeed(seed)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(attest.VerifyFrameAttestation("repo-graph", frame, attestation, public).Verdict)
	// Output: valid
}

// remoteKey stands in for a key held by a KMS or an HSM. All the attest
// package ever sees is crypto.Signer: a public key, and a call that returns a
// signature over the bytes it is given. The key itself never enters the
// process.
type remoteKey struct {
	public ed25519.PublicKey
	sign   func(message []byte) ([]byte, error) // the round trip to the backend
}

func (k remoteKey) Public() crypto.PublicKey { return k.public }

func (k remoteKey) Sign(_ io.Reader, message []byte, _ crypto.SignerOpts) ([]byte, error) {
	return k.sign(message)
}

// Signing with a key that never leaves its backend: the same call, with a
// crypto.Signer in place of an ed25519.PrivateKey.
func ExampleSignCommitment_keyOutsideTheProcess() {
	backend, err := attest.PrivateKeyFromSeed(bytes.Repeat([]byte{0x07}, 32))
	if err != nil {
		log.Fatal(err)
	}
	signer := remoteKey{
		public: backend.Public().(ed25519.PublicKey),
		sign:   func(message []byte) ([]byte, error) { return ed25519.Sign(backend, message), nil },
	}

	root := attest.MerkleRoot([][32]byte{{1}, {2}, {3}})
	attestation, err := attest.SignCommitment(root, signer, "kms-key-2026-09", "oxagen", "2026-09-28T00:00:00Z")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(attest.VerifyCommitment(root, attestation, signer.public).Verdict)
	// Output: valid
}

// Content-addressing a lifecycle record and attesting it. The record is
// hashed from its JSON text, with its own record_hash member left out of the
// preimage, so the hash can be computed before the member is filled in.
func ExampleSignRecord() {
	record := []byte(`{
		"record_id": "rec_obs_0001",
		"record_kind": "observation",
		"confidence": 0.82,
		"statement": "the api handler retries three times before surfacing a 502"
	}`)

	preimage, err := attest.RecordHashPreimage(record)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(string(preimage))

	seed := bytes.Repeat([]byte{0x07}, 32)
	key, err := attest.PrivateKeyFromSeed(seed)
	if err != nil {
		log.Fatal(err)
	}
	attestation, err := attest.SignRecord(record, key, "key-1", "oxagen", "2026-07-29T14:00:05Z")
	if err != nil {
		log.Fatal(err)
	}
	public, err := attest.PublicKeyFromSeed(seed)
	if err != nil {
		log.Fatal(err)
	}
	result, err := attest.VerifyRecordAttestation(record, attestation, public)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(result.Verdict)
	// Output:
	// {"confidence":0.82,"record_id":"rec_obs_0001","record_kind":"observation","statement":"the api handler retries three times before surfacing a 502"}
	// valid
}
