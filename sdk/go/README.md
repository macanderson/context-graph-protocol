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
