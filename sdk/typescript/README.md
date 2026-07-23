# @contextgraph/sdk — TypeScript

A zero-dependency TypeScript SDK for building **conformant** Context Graph
Protocol providers. Implement one interface, hand it to the runtime, and you
have a provider that speaks the line-oriented JSON wire over stdio and passes
the same conformance suite that judges the Rust reference provider.

> This is the second independent implementation (after the Rust reference), and
> it passes the full conformance suite — the concrete evidence behind the
> protocol's "enforced by contract, not convention" claim. See
> [`sdk/README.md`](../README.md) for the multi-language picture.

## Install

```sh
npm install @contextgraph/sdk
```

## Write a provider

```ts
import { runStdioProvider, budgetTokens, type Provider } from "@contextgraph/sdk";

const provider: Provider = {
  info: () => ({
    name: "my-docs-provider",
    version: "0.1.0",
    // Nothing leaves the machine ⇒ declare the honest local-only egress scope.
    data_flow: { reads: true, writes: false, egress: false, egress_scopes: ["local-only"] },
  }),
  capabilities: () => ({ query: { kinds: ["doc"] }, correlation: true, verify: true }),
  query: () => ({
    frames: [
      {
        id: "doc:1",
        kind: "doc",
        title: "Getting started",
        content: "Install the binding, then implement the required methods.",
        content_digest: `sha256:${"11".repeat(32)}`,
        score: 0.9,
        // token_cost MUST equal ceil(utf8_len(content)/4) — the SDK computes it.
        token_cost: budgetTokens("Install the binding, then implement the required methods."),
        valid_from: "2026-01-01T00:00:00Z",
        provenance: [{ type: "file", uri: "file:///docs/start.md", range: "L1-10", digest: `sha256:${"11".repeat(32)}` }],
        citation_label: "start.md L1-10",
        relations: [],
      },
    ],
    truncated: false,
  }),
};

runStdioProvider(provider);
```

The runtime handles the whole lifecycle a host drives — handshake, query
(echoing the correlation `id`), verify, shutdown — and stays alive with a typed
error on a malformed line rather than crashing.

## What it gives you

- **Wire types** (`ContextFrame`, `ContextQuery`, `Capabilities`, `Envelope`, …)
  mirrored from the JSON Schema — the language-neutral source of truth.
- **`runStdioProvider(provider)`** — the stdio lifecycle loop.
- **`budgetTokens(content)`** — the canonical B3 cost, `ceil(utf8_len/4)`.
- A runnable **example provider** (`examples/example-docs.ts`) that passes all
  seven conformance checks.

## Prove it conformant

Build, then run the reference suite against your provider:

```sh
npm run build
# from the repository root, with the Rust bins built (cargo build --workspace --bins):
./.github/scripts/conformance-external.sh -- node sdk/typescript/dist/examples/example-docs.js
```

A green run is the machine-checkable claim that your provider honors the
protocol — the same bar every implementation is held to.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.
