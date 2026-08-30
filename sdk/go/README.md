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
go get github.com/macanderson/context-graph-protocol/sdk/go/contextgraph@v0.1.0
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
			Content:       content,
			ContentDigest: "sha256:1111111111111111111111111111111111111111111111111111111111111111",
			Score:         0.9,
			// TokenCost MUST equal ceil(utf8_len(content)/4).
			TokenCost:     cg.BudgetTokens(content),
			ValidFrom:     "2026-01-01T00:00:00Z",
			Provenance:    []cg.Provenance{{Type: "file", URI: "file:///docs/start.md", Range: "L1-10", Digest: "sha256:1111111111111111111111111111111111111111111111111111111111111111"}},
			CitationLabel: "start.md L1-10",
		}},
	}, nil
}

func main() { cg.RunStdioProvider(myProvider{}) }
```

To answer `context/verify`, also implement `cg.Verifier`. The runtime handles the
whole lifecycle — handshake, query (echoing the correlation `id`), verify,
shutdown — and stays alive with a typed error on a malformed line.

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

Signing is not here. The protocol specifies the preimage, never the custody of
the key: a provider computes `attest.FrameCommitment(...)`, signs those 32
bytes with whatever backend holds its key, and assembles the attestation
itself.

Two things worth knowing:

- **`attest.Link` is not `contextgraph.Provenance`.** The wire struct carries
  its optional fields as `string` with `omitempty` and so cannot tell an absent
  URI from a present empty one — a distinction the §6.5.1 presence byte makes
  normative. `attest.Link` uses pointers, and `LinkFromProvenance` states the
  collapse it performs rather than hiding it.
- **Go's `crypto/ed25519` accepts a small-order public key.** §6.5.4 asks for a
  strict verifier, so `VerifyCommitment` declines those keys — and any key
  whose `y` is not reduced — before the standard library sees them.

The vectors are shared across every language:

```sh
cd sdk/go && go test ./contextgraph/attest/
```

They come from `tests/vectors/attestation-vectors.json`, which the Rust
reference publishes and pins.

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
