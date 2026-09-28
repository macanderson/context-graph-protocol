// Signing, pinned to the published vector (issue #127).
//
// Ed25519 is deterministic, so a signature is reproducible by anyone holding
// the seed, and tests/vectors/attestation-vectors.json publishes the seed, the
// public key it derives, and the signature it produces over the published
// frame commitment. Every signing test here compares against those bytes —
// a signer that agrees only with this package's own verifier is what the
// vector exists to catch.
package attest

import (
	"bytes"
	"crypto"
	"crypto/ed25519"
	"errors"
	"io"
	"testing"
)

// publishedFrame rebuilds the frame the published attestation signs.
func publishedFrame(v vectorFile) (string, Frame) {
	spec := v.FrameCommitment
	links := make([]Link, 0, len(spec.Frame.Provenance))
	for _, name := range spec.Frame.Provenance {
		links = append(links, link(v, name))
	}
	digest := spec.Frame.ContentDigest
	return spec.ProviderID, Frame{ID: spec.Frame.ID, ContentDigest: &digest, Provenance: links}
}

func publishedKey(t *testing.T, v vectorFile) ed25519.PrivateKey {
	t.Helper()
	key, err := PrivateKeyFromSeed(mustHex(t, v.Signature.SigningKeySeedHex))
	if err != nil {
		t.Fatalf("the published seed: %v", err)
	}
	return key
}

// TestSigningThePublishedCommitmentReproducesThePublishedSignature is the
// witness #127 names: sign the published commitment with the published seed,
// and compare to the published signature byte for byte.
func TestSigningThePublishedCommitmentReproducesThePublishedSignature(t *testing.T) {
	v := loadVectors(t)
	want := v.Signature.Attestation
	got, err := SignCommitment(mustDigest(t, want.SignedCommitment), publishedKey(t, v),
		want.KeyID, want.AttesterID, want.IssuedAt)
	if err != nil {
		t.Fatalf("signing the published commitment: %v", err)
	}
	if !bytes.Equal(mustHex(t, got.Signature), mustHex(t, want.Signature)) {
		t.Errorf("signature\n got %s\nwant %s", got.Signature, want.Signature)
	}
	if got != want {
		t.Errorf("attestation\n got %+v\nwant %+v", got, want)
	}
}

func TestPublicKeyFromSeedIsThePublishedKey(t *testing.T) {
	v := loadVectors(t)
	public, err := PublicKeyFromSeed(mustHex(t, v.Signature.SigningKeySeedHex))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(public, mustHex(t, v.Signature.PublicKeyHex)) {
		t.Errorf("public key\n got %x\nwant %s", []byte(public), v.Signature.PublicKeyHex)
	}
}

func TestSignFrameAttestationSignsThePublishedFrame(t *testing.T) {
	v := loadVectors(t)
	providerID, frame := publishedFrame(v)
	want := v.Signature.Attestation
	got, err := SignFrameAttestation(providerID, frame, publishedKey(t, v), want.KeyID, want.AttesterID, want.IssuedAt)
	if err != nil {
		t.Fatalf("signing the published frame: %v", err)
	}
	if got != want {
		t.Errorf("attestation\n got %+v\nwant %+v", got, want)
	}
	if result := VerifyFrameAttestation(providerID, frame, got, mustHex(t, v.Signature.PublicKeyHex)); !result.IsValid() {
		t.Errorf("a fresh signature must verify under the published key, got %+v", result)
	}
}

// TestSigningAFrameWithoutAContentDigestIsRefused holds the SDK to §6.5.2 and
// ADR 0018: a digest-less commitment binds identity and provenance but not
// content, so its signature would survive the frame's content being replaced.
func TestSigningAFrameWithoutAContentDigestIsRefused(t *testing.T) {
	v := loadVectors(t)
	providerID, frame := publishedFrame(v)
	frame.ContentDigest = nil
	_, err := SignFrameAttestation(providerID, frame, publishedKey(t, v), "key-1", "oxagen", "2026-08-27T00:00:00Z")
	if !errors.Is(err, ErrFrameHasNoContentDigest) {
		t.Errorf("got %v, want ErrFrameHasNoContentDigest", err)
	}
}

func TestASeedIsExactlyThirtyTwoBytes(t *testing.T) {
	for _, n := range []int{0, 31, 33, 64} {
		if _, err := PrivateKeyFromSeed(make([]byte, n)); !errors.Is(err, ErrSeedLength) {
			t.Errorf("%d-byte seed: got %v, want ErrSeedLength", n, err)
		}
		if _, err := PublicKeyFromSeed(make([]byte, n)); !errors.Is(err, ErrSeedLength) {
			t.Errorf("%d-byte seed: got %v, want ErrSeedLength", n, err)
		}
	}
}

// hsmSigner stands in for a key held outside the process: it exposes only
// crypto.Signer, and a caller never sees its key bytes.
type hsmSigner struct {
	key   ed25519.PrivateKey
	calls int
}

func (s *hsmSigner) Public() crypto.PublicKey { return s.key.Public() }

func (s *hsmSigner) Sign(_ io.Reader, message []byte, opts crypto.SignerOpts) ([]byte, error) {
	s.calls++
	if opts.HashFunc() != 0 {
		return nil, errors.New("this key signs pure Ed25519 only")
	}
	return ed25519.Sign(s.key, message), nil
}

// TestAnHSMBackedSignerProducesThePublishedAttestation is the custody path:
// the same call, a signer that never hands over its key, the same bytes.
func TestAnHSMBackedSignerProducesThePublishedAttestation(t *testing.T) {
	v := loadVectors(t)
	providerID, frame := publishedFrame(v)
	want := v.Signature.Attestation
	hsm := &hsmSigner{key: publishedKey(t, v)}
	got, err := SignFrameAttestation(providerID, frame, hsm, want.KeyID, want.AttesterID, want.IssuedAt)
	if err != nil {
		t.Fatalf("signing through crypto.Signer: %v", err)
	}
	if got != want {
		t.Errorf("attestation\n got %+v\nwant %+v", got, want)
	}
	if hsm.calls != 1 {
		t.Errorf("the backend must be asked to sign exactly once, was asked %d times", hsm.calls)
	}
}

// fixedSigner is a crypto.Signer assembled from parts, for the refusals.
type fixedSigner struct {
	public crypto.PublicKey
	sign   func(message []byte) ([]byte, error)
}

func (s fixedSigner) Public() crypto.PublicKey { return s.public }

func (s fixedSigner) Sign(_ io.Reader, message []byte, _ crypto.SignerOpts) ([]byte, error) {
	return s.sign(message)
}

func TestSigningRefusalsAreNamed(t *testing.T) {
	v := loadVectors(t)
	key := publishedKey(t, v)
	commitment := mustDigest(t, v.Signature.Attestation.SignedCommitment)
	sign := func(message []byte) ([]byte, error) { return ed25519.Sign(key, message), nil }

	cases := []struct {
		name   string
		signer crypto.Signer
		want   error
	}{
		{"nil signer", nil, ErrNotEd25519Signer},
		{
			"a signer that is not Ed25519",
			fixedSigner{public: "not a key", sign: sign},
			ErrNotEd25519Signer,
		},
		{
			// The first small-order encoding the fixture publishes: a strict
			// verifier declines it, so signing under it publishes nothing.
			"a small-order public key",
			fixedSigner{public: ed25519.PublicKey(mustHex(t, v.VerifierStrictness.SmallOrderPublicKeysHex[0])), sign: sign},
			ErrUnusableSigningKey,
		},
		{
			"a backend whose signature does not verify",
			fixedSigner{public: key.Public(), sign: func(message []byte) ([]byte, error) {
				signature := ed25519.Sign(key, message)
				signature[0] ^= 0x01
				return signature, nil
			}},
			ErrSignatureDoesNotVerify,
		},
		{
			"a backend that returns the wrong length",
			fixedSigner{public: key.Public(), sign: func([]byte) ([]byte, error) { return make([]byte, 10), nil }},
			ErrSignatureDoesNotVerify,
		},
	}
	for _, c := range cases {
		if _, err := SignCommitment(commitment, c.signer, "key-1", "oxagen", "2026-08-27T00:00:00Z"); !errors.Is(err, c.want) {
			t.Errorf("%s: got %v, want %v", c.name, err, c.want)
		}
	}

	backendDown := errors.New("the HSM is unreachable")
	failing := fixedSigner{public: key.Public(), sign: func([]byte) ([]byte, error) { return nil, backendDown }}
	if _, err := SignCommitment(commitment, failing, "key-1", "oxagen", "2026-08-27T00:00:00Z"); !errors.Is(err, backendDown) {
		t.Errorf("a backend's own error must reach the caller: got %v", err)
	}
}

func TestASignedMerkleRootVerifies(t *testing.T) {
	v := loadVectors(t)
	root := MerkleRoot(merkleLeaves(v, 7))
	attestation, err := SignCommitment(root, publishedKey(t, v), "key-1", "oxagen", "2026-08-27T00:00:00Z")
	if err != nil {
		t.Fatal(err)
	}
	if attestation.SignedCommitment != v.Merkle.RootsByLeafCount["7"] {
		t.Errorf("the attestation must name the root it signs: %s", attestation.SignedCommitment)
	}
	if got := VerifyCommitment(root, attestation, mustHex(t, v.Signature.PublicKeyHex)); !got.IsValid() {
		t.Errorf("a signed root must verify, got %+v", got)
	}
}
