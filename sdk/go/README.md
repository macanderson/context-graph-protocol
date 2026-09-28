# contextgraph go SDK

A zero-dependency (stdlib-only) Go SDK for building **conformant** Context Graph
Protocol providers. Implement one small interface, hand it to the runtime, and
you have a provider that speaks the line-oriented JSON wire over stdio and passes
the same conformance suite that judges the Rust reference provider.

> Fourth independent implementation (after Rust, TypeScript, and Python); passes
> the full conformance suite. See [`sdk/README.md`](../README.md) for the whole
> picture.

## Install

```sh
go get github.com/macanderson/context-graph-protocol/sdk/go/contextgraph
```

## Write a provider

```go
package main

import cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"

type myProvider struct{}

func (myProvider) Info() cg.ProviderInfo {
	// Nothing leaves the machine -> declare the honest local-only egress scope.
	return cg.ProviderInfo{
		Name: "my-docs-provider", Version: "0.1.0",
		DataFlow: cg.DataFlow{Reads: true, EgressScopes: []string{"local-only"}},
	}
}

func (myProvider) Capabilities() cg.Capabilities {
	return cg.Capabilities{Query: cg.QueryCapability{Kinds: []string{"doc"}}, Correlation: true}
}

func (myProvider) Query(_ cg.ContextQuery) (cg.ContextQueryResult, error) {
	content := "Install the binding, then implement the required methods."
	return cg.ContextQueryResult{
		Frames: []cg.ContextFrame{{
			ID: "doc:1", Kind: "doc", Title: "Getting started",
			Content:       cg.Ptr(content),
			ContentDigest: cg.Ptr("sha256:1111111111111111111111111111111111111111111111111111111111111111"),
			Score:         0.9,
			// TokenCost MUST equal ceil(utf8_len(content)/4).
			TokenCost:     cg.BudgetTokens(content),
			ValidFrom:     "2026-01-01T00:00:00Z",
			Provenance:    []cg.Provenance{{Type: "file", URI: cg.Ptr("file:///docs/start.md"), Range: cg.Ptr("L1-10"), Digest: cg.Ptr("sha256:1111111111111111111111111111111111111111111111111111111111111111")}},
			CitationLabel: "start.md L1-10",
		}},
	}, nil
}

func main() { cg.RunStdioProvider(myProvider{}) }
```

`cg.Ptr` is there because the optional fields that feed a provenance
attestation are `*string`, not `string`: SPEC.md §6.5.1 makes `"uri": null` and
`"uri": ""` hash differently, and a plain `string` with `omitempty` cannot hold
that difference in either direction. A `nil` is omitted; a pointer to `""` is
emitted as `""`.

To answer `context/verify`, also implement `cg.Verifier`. The runtime handles the
whole lifecycle — handshake, query (echoing the correlation `id`), verify,
shutdown — and stays alive with a typed error on a malformed line.

### Four things the runtime does so you do not have to

- **An empty answer is a real answer.** "I have nothing relevant" is explicitly
  permitted, so `return cg.ContextQueryResult{}, nil` is valid — the runtime
  normalizes a nil `Frames` (and a nil `Verdicts` from your `Verifier`) to `[]`
  before writing. Go marshals a nil slice to `null`, and `"frames": null` is a
  *deserialization error* at a conforming host, not an empty result.
- **A refusal keeps its code in either spelling.** `cg.ProviderError{…}` and
  `&cg.ProviderError{…}` both reach the host as
  `{"type":"error","code":"bad_request",…}`, and so does either one wrapped with
  `%w`. Use it to refuse a request you cannot honestly serve — SPEC.md §E1's
  rejection of a query embedding whose length contradicts your declared
  `EmbeddingsFingerprint` is the canonical case.
- **`kinds` is a filter, not a hint (§Q1).** When `query.Kinds` is non-empty,
  return only frames whose `Kind` is in it — returning others spends the host's
  budget on content it explicitly excluded. Zero frames is the right answer for
  a kind you declare but cannot currently serve.
- **`as_of` pins retrieval to an instant (§F4).** Drop any frame whose
  `ValidFrom` is strictly after `query.AsOf`. The timestamp profile admits one
  spelling per instant, so a string compare is a chronological one.

Neither filter is truncation: leave `Truncated` false and `DroppedEstimate` nil,
because the host excluded that content itself.

### Attaching a vector to a frame

`ContextFrame.Embedding` is a `*cg.FrameEmbedding` — the fingerprint naming the
space, and an elidable vector:

```go
frame.Embedding = &cg.FrameEmbedding{
	Fingerprint: "bge-small-en-v1.5/384/l2",
	Vector:      vector, // optional: name the space without shipping the numbers
}
```

On the query side, `query.HasEmbedding()` asks whether the host sent an
`embedding` **field**, which is not the same question as whether it sent any
numbers: `"embedding": []` is present and empty, and its length of 0 contradicts
a declared 384 exactly as 385 does. Guard a §E1 dimension check with
`HasEmbedding()`, never with `len(query.Embedding) > 0`.

## Host it over HTTP

The same provider runs behind a single POST endpoint (the streamable-HTTP
transport, SPEC.md §3) via a `net/http` handler:

```go
http.ListenAndServe("127.0.0.1:8789", cg.Handler(myProvider{}))
```

`cg.RespondToBody(provider, body)` is the transport-free state machine if you
want to wire it into a router yourself. A runnable HTTP example lives at
`examples/example-docs-http`; confirm it green with
`contextgraph-inspect http http://127.0.0.1:8789` (the `malformed-input-tolerance`,
`embedding-fingerprint`, and `correlation` probes report *skipped* over HTTP —
they inspect raw framing this transport doesn't expose).

## Verify a provenance attestation

`SPEC.md` §6.5 makes a frame's provenance *evidence* rather than merely
tamper-evident: a detached Ed25519 signature over a commitment to the frame's
identity and its provenance chain. Package `contextgraph/attest` implements the
whole construction — the length-prefixed link encoding, the source-first chain
fold, the frame commitment, an RFC 6962 Merkle root over a result set with
inclusion proofs, and verification.

```go
result := attest.VerifyFrameAttestation("repo-graph", frame, attestation, publicKey)
if !result.IsValid() {
    // Never a bool: "the frame changed after signing" and "the key is wrong"
    // call for opposite responses, and F9 says an unverifiable attestation
    // degrades a frame to unattested rather than disqualifying it.
    log.Printf("attestation: %s", result.Verdict)
}
```

Two things worth knowing:

- **`attest.Link` is not `contextgraph.Provenance`.** `Link` is the encoding's
  view — the six fields that enter the preimage, nothing else — and `Frame` is
  the three fields §6.5.2 commits to, so verifying a frame you received does not
  mean fabricating a `Score` and a `TokenCost`. Both types carry their optional
  fields as pointers, and so does the wire struct, so `LinkFromProvenance` and
  `FrameFromContextFrame` copy presence straight through:

  ```go
  var frame cg.ContextFrame
  json.Unmarshal(body, &frame)   // somebody else's frame, "uri": "" and all
  result := attest.VerifyFrameAttestation(providerID,
      attest.FrameFromContextFrame(frame), attestation, publicKey)
  ```

  Before `v0.2.0` the wire struct's optional fields were `string`, which folded
  an absent URI and a present empty one together; a Go verifier then computed a
  chain head the signer never did and answered `commitment_mismatch` on an
  honest frame. See [`MIGRATION.md`](../../MIGRATION.md) §7.
- **Go's `crypto/ed25519` accepts a small-order public key.** §6.5.4 asks for a
  strict verifier, so `VerifyCommitment` declines those keys — and any key
  whose `y` is not reduced — before the standard library sees them.

## Sign a provenance attestation

The protocol specifies the preimage and never the custody of the key, so every
signing function takes a `crypto.Signer` — the interface `ed25519.PrivateKey`
implements, and the one KMS clients, PKCS#11 wrappers and `ssh-agent` bindings
implement too. One call serves both kinds of provider:

```go
// In-process: a key derived from a 32-byte seed.
key, err := attest.PrivateKeyFromSeed(seed)
attestation, err := attest.SignFrameAttestation("repo-graph", frame, key,
    "key-2026-09", "acme", "2026-09-28T00:00:00Z")

// Out of process: anything that implements crypto.Signer with an Ed25519 key.
attestation, err := attest.SignCommitment(attest.MerkleRoot(commitments), kmsSigner,
    "kms-key-2026-09", "acme", "2026-09-28T00:00:00Z")
```

`attest.PublicKeyFromSeed(seed)` gives the matching public key in the raw form
the verifiers take. Before an attestation leaves the process, the signer's
public key is checked against the same strictness rules a verifier applies, and
the signature is verified under it — so a faulty backend, or one that hashed
the message first, is an error here rather than a `bad_signature` at a
consumer. `SignFrameAttestation` refuses a frame with no `ContentDigest`: its
commitment binds identity and provenance but not content, so the signature
would outlive a change to the frame's content (§6.5.2, ADR 0018).

Signing is pinned to the published vector byte for byte. Ed25519 is
deterministic, and `tests/vectors/attestation-vectors.json` publishes the seed,
so the test signs the published commitment and compares the result with the
published signature.

### Key custody

`PrivateKeyFromSeed` puts a long-lived signing key in your application's
memory. Know what that costs before you choose it:

- **Anything that can read the process can sign as you.** A core dump, a heap
  profile, a debugger attached in production, or a memory-disclosure bug hands
  over the key, and every attestation it signs afterwards is indistinguishable
  from yours until the key is rotated and consumers stop trusting its `key_id`.
- **Zeroing it afterwards is not a reliable mitigation.** You can overwrite
  the slice you hold, but copies made along the way — the seed you read, a
  buffer a library allocated, a stack the runtime moved — are not yours to
  reach, and Go does not promise to clear them.
- **The seed is the key.** Wherever the seed is stored — an environment
  variable, a config file, a secret mount — has the same exposure as the key.

That is an acceptable trade for tests, for a local provider signing its own
index on a developer machine, and for a short-lived key whose rotation you
automate. For an attestation anyone else will rely on, keep the key in a KMS or
an HSM and pass its `crypto.Signer`: the process then holds only a handle, a
compromise can sign only while it lasts, and the backend's audit log records
every signature. Name each key with a fresh `key_id` and never reuse one after
rotation (§6.5).

## Content-address and attest a lifecycle record

A Context Exchange Provider identifies each record by its `record_hash`:
SHA-256 over the RFC 8785 (JCS) canonical form of the record with its own
top-level `record_hash` member removed (profile `LH1`, ADR 0017). Package
`contextgraph/jcs` is the canonicalizer — ECMAScript number formatting, UTF-16
member ordering, and a strict parser that refuses the lone surrogates,
duplicated members and invalid UTF-8 that `encoding/json` would silently
repair — and `attest` builds the record layer on it:

```go
hash, err := attest.RecordHash(recordJSON)            // "sha256:…"
preimage, err := attest.RecordHashPreimage(recordJSON) // the exact bytes hashed

attestation, err := attest.SignRecord(recordJSON, key, "key-1", "acme", issuedAt)
result, err := attest.VerifyRecordAttestation(recordJSON, attestation, publicKey)
```

The record functions take the record's JSON **text**. Hashing from the bytes
you received is the faithful path; decoding into `map[string]any` first turns
every number into a `float64` and resolves duplicated members silently.
`attest.RecordHashOf(v)` is there for a record you are building in Go.

`VerifyRecordAttestation` recomputes the hash from the record's content rather
than trusting its stored `record_hash` (profile `LC5`), so a record edited
after signing and then given a matching `record_hash` is still a
`commitment_mismatch`. The signed message is `"contextgraph/attest/1/record"`
followed by the digest's 32 raw bytes (`LC4`); `attest.RecordAttestationMessage`
builds it for a backend that is not a `crypto.Signer`.

## Reproduce the published vectors

The vectors are shared across every language:

```sh
cd sdk/go && go test ./contextgraph/attest/ ./contextgraph/jcs/
```

The provenance vectors come from `tests/vectors/attestation-vectors.json`, and
the record vectors from `tests/fixtures/record-hash-vectors.json`,
`record-attestation.json` and `record-attestation-key.json`; the Rust reference
publishes and pins all of them. The `jcs` suite pins RFC 8785's own examples,
including the Appendix B number table, where `strconv.FormatFloat` and
ECMAScript part ways.

## Prove it conformant

From the repository root, with the Rust bins built:

```sh
cargo build --workspace --bins
( cd sdk/go && go build -o /tmp/cg-go-example ./examples/example-docs )
./.github/scripts/conformance-external.sh -- /tmp/cg-go-example
```

A green run is the machine-checkable claim that your provider honors the protocol.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.
