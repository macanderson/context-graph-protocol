# Implementing a CGP provider

There are two ways to implement a Context Graph Protocol (CGP) provider, depending on whether you're
writing Rust that runs inside the same process as the host, or a standalone
program (in any language) that the host talks to as a child process or a
remote HTTP endpoint.

## Option A: in-process, via the `ContextProvider` trait (Rust only)

If your provider runs in the same process as an `contextgraph-host`-based host,
implement the one trait every source implements
(`contextgraph-host::provider::ContextProvider`):

```rust
use async_trait::async_trait;
use contextgraph_host::HostError;
use contextgraph_types::{Capabilities, ContextQuery, ContextQueryResult, ProviderInfo};

#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// The provider's host-facing id — its routing key and its consent key.
    fn id(&self) -> &str;

    /// Identity + declared data-flow direction, surfaced at consent time.
    fn info(&self) -> &ProviderInfo;

    /// Capabilities: which frame kinds and filters this provider serves,
    /// whether it upserts, does graph, is an embedder, or supports
    /// subscriptions.
    fn capabilities(&self) -> &Capabilities;

    /// Answer a context query with budgeted, provenance-carrying frames.
    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError>;

    /// Shut the provider down cleanly. Defaults to a no-op.
    async fn shutdown(&self) -> Result<(), HostError> { Ok(()) }
}
```

`info()` and `capabilities()` are cheap synchronous getters — cache them at
construction time rather than recomputing per call. Register your provider
with `host.register(Box::new(my_provider))` and it participates in
`Host::query_all`'s fan-out like any other provider.

## Option B: out-of-process, via the wire protocol (any language)

A provider written in any language — the common case for a third-party
integration — implements the CGP wire protocol directly. `contextgraph-host` speaks
this protocol over two transports; you only need to implement one:

- **stdio** — the host spawns your program as a child process and exchanges
  newline-delimited JSON (NDJSON) over its stdin/stdout: exactly one
  `serde_json`-shaped value per line.
- **streamable HTTP** — the host POSTs one JSON envelope per exchange to your
  URL and expects one JSON envelope back as the response body.

> **Writing TypeScript, Python, or Go?** You don't have to hand-roll any of the
> wire below — the official provider SDKs implement the whole state machine over
> both transports, and you implement one small interface. See **Provider SDKs:
> TypeScript, Python, and Go** at the end of this page. The raw protocol here is
> what those SDKs are built on, and all you need for any other language.

Both transports carry the same message vocabulary, `contextgraph-host::wire::Envelope`
(a `serde` externally-tagged enum, `#[serde(tag = "type", rename_all =
"snake_case")]`):

| `type` | direction | payload |
|---|---|---|
| `handshake` | host → provider | `{ protocol_version }` |
| `handshake_ack` | provider → host | `{ protocol_version, provider: ProviderInfo, capabilities: Capabilities }` |
| `query` | host → provider | `{ query: ContextQuery }` |
| `frames` | provider → host | `{ result: ContextQueryResult }` |
| `verify` | host → provider | `{ request: VerifyRequest }` — *only if you advertise `verify`* |
| `verified` | provider → host | `{ response: VerifyResponse }` |
| `shutdown` | host → provider | *(no payload)* |
| `error` | provider → host | `{ message: String }` |

`ProviderInfo`, `Capabilities`, `ContextQuery`, `ContextQueryResult`, and
`ContextFrame` are the `contextgraph-types` shapes documented in
[protocol-surface.md](./protocol-surface.md) — the wire payload is exactly
their `serde_json` serialization, field names and all.

### The exchange

1. **Handshake.** The host sends `handshake` with the protocol version it
   speaks. Your provider replies `handshake_ack` with:
   - `protocol_version` — see [Version compatibility](#version-compatibility) below.
   - `provider` — your `ProviderInfo` (name, version, and **honest**
     `data_flow`).
   - `capabilities` — what you can do (`Capabilities`).
2. **Zero or more queries.** The host sends `query`; you reply `frames` with
   a `ContextQueryResult`, or `error` if the request itself was bad (an
   `error` reply lets you report a problem without dying — a provider that
   exits on a bad request fails the `malformed-input-tolerance` conformance
   check).
3. **Zero or more verifications** *(optional).* If you set
   `capabilities.verify`, the host may send `verify` with a batch of frame
   identities and expect `verified` back with one verdict each — `valid`,
   `stale` (optionally naming your current digest), `gone`, or `unknown`.
   **Never send frame bodies in a `verified` reply**: the whole point is that
   revalidation costs bytes rather than tokens, so the host re-queries when it
   actually wants new content. Answer by comparing the digest the host presents
   against the one you currently serve — a mismatch is `stale`, and answering
   `valid` for bytes you are not serving fails the `verify-honesty` conformance
   check. Leave `verify` unset (the default) and the host simply re-queries your
   frames instead; nothing breaks. See
   [context-reuse §4](./context-reuse.md#4-context-verification).
4. **Shutdown.** The host sends `shutdown`; a well-behaved provider exits
   cleanly (stdio: exit the process; HTTP: nothing further to do — the host
   doesn't expect a reply).

A malformed line (bad JSON, wrong envelope shape) should be **ignored or
answered with `error` — never crash the process.** The host bounds every
exchange with a timeout on its side, so a slow reply is a timeout, not a
hang; but only your provider can guarantee it survives garbage input.

### Version compatibility

Two protocol version strings interoperate when they share a **major
family** — the substring up to the first `.`. So `contextgraph/1.0` and
`contextgraph/1.0` interoperate (both family `contextgraph/1`), while `contextgraph/2.0` does not
(`contextgraph-host::wire::versions_compatible`). This is what lets the eventual
public `contextgraph/1.0` freeze drop the `-draft` suffix without a flag day — ack
whatever `1.x` family you actually implement; don't hardcode the exact
string. A version-family mismatch is reported to the host as a named error,
never left to hang.

### The data-flow / consent contract

`ProviderInfo.data_flow` is not decorative — it changes what the host will
do:

- `reads: true` — you can see workspace content via query payloads.
- `writes: true` — you persist `context/upsert`-style writes (not yet part
  of the query/frames exchange in this crate; reserved for a future CGP
  method).
- `egress: true` — **anything you do sends data off the local machine.**

**Declare `egress: true` honestly if your provider calls out to a remote
service, even indirectly.** A conforming host (`contextgraph-host::consent`) refuses
to query an `egress` provider until the user has recorded explicit, one-time
consent naming what leaves — the query payload is never transmitted before
that. This is enforced host-side and cannot be opted out of by a provider
that under-declares its own egress; note that `contextgraph-host`'s own HTTP transport
goes further and treats *every* remote provider as egress regardless of what
it claims in the handshake, precisely so a remote can't lie its way out of
the consent gate.

### The budget-honesty contract

Every `ContextQuery` carries `max_tokens`. **The frames you return must sum
`token_cost` to at most that budget.** A host built on `contextgraph-host::Host`
checks `ContextQueryResult::respects_budget` on every response and drops
(with a loud report, not a silent discard) the frames of any provider that
exceeds it — the `budget-honesty` conformance check enforces the same rule.
If you have more relevant material than fits, return your best frames within
budget, set `truncated: true`, and optionally `dropped_estimate`.

### The citation contract

Every frame needs a non-empty `title` and a non-empty `citation_label` — a
host must be able to cite what it used without falling back to a bare id.
This is checked by the `frame-validity` conformance check and is a
platform-wide convention, not a CGP-specific quirk.

### A complete minimal example

The `contextgraph-example-docs` binary bundled with `contextgraph-conformance`
(`contextgraph-conformance/src/bin/contextgraph-example-docs.rs`) is a real, runnable ~150-line
stdio provider that implements this whole exchange: it reads NDJSON lines
from stdin, replies to `handshake` with a `handshake_ack`, replies to `query`
with two canned `doc` frames, and exits cleanly on `shutdown`. Read it end to
end as the reference implementation; it deliberately reuses `contextgraph-host`'s
`Envelope` type for convenience (both crates live in the same workspace), but
an out-of-tree provider in any language only needs a JSON codec and the wire
table above — no dependency on `contextgraph-host` itself.

### Probing your provider interactively

Once you have something that speaks the handshake, point `contextgraph-inspect` (from
`contextgraph-conformance`) at it before running the full suite:

```bash
cargo install contextgraph-conformance
contextgraph-inspect stdio --query "how do I configure it" -- ./my-provider
# or:
contextgraph-inspect http --query "how do I configure it" https://my-provider.example.com/contextgraph
```

It prints your negotiated identity, capabilities, and data-flow, fires the
optional test query, and shows you the frames it got back with their scores
and token costs — a fast human-readable feedback loop before you run the
scripted conformance suite. See
[running-conformance.md](./running-conformance.md) for that next step.

### Getting listed once you're green

Once `contextgraph-inspect ... --json` reports every check `pass` (or `skip`,
never `fail`), your provider is eligible for the
[**conformance registry**](./registry.md) — a table of conformant providers
with a reproducible report backing each claim, plus the
`![CGP conformant](https://raw.githubusercontent.com/macanderson/context-graph-protocol/main/assets/badges/conformant.svg)`
badge you can put in your own README once listed. Listing is a pull request, not a
self-attested form: see [registry.md](./registry.md#how-to-get-listed) for
exactly what to include.

## Provider SDKs: TypeScript, Python, and Go

The wire protocol above is small on purpose — small enough to hand-roll in any
language. But if you're writing **TypeScript, Python, or Go**, you don't have
to: the official zero-dependency SDKs implement the whole lifecycle (handshake,
correlation-id echo, verify, shutdown, malformed-input tolerance) over both
transports. You implement one small interface; the SDK is the conformant
machinery around it.

The SDKs live under `sdk/typescript`, `sdk/python`, and `sdk/go`, each with a
runnable example provider that passes the same conformance suite that judges the
Rust reference. (Publishing to npm / PyPI is tracked in #59; until then, install
from a checkout as shown.)

### TypeScript quick-start

```sh
npm install @contextgraphprotocol/typescript-sdk   # or, from a checkout: npm install ./sdk/typescript
```

```ts
import { runStdioProvider, budgetTokens, type Provider } from "@contextgraphprotocol/typescript-sdk";

const provider: Provider = {
  info: () => ({
    name: "my-docs-provider",
    version: "0.1.0",
    // Nothing leaves the machine ⇒ declare the honest local-only egress scope.
    data_flow: { reads: true, writes: false, egress: false, egress_scopes: ["local-only"] },
  }),
  capabilities: () => ({ query: { kinds: ["doc"] }, correlation: true, verify: true }),
  query: () => {
    const content = "Install the binding, then implement the required methods.";
    return {
      frames: [{
        id: "doc:1", kind: "doc", title: "Getting started", content,
        content_digest: `sha256:${"11".repeat(32)}`, score: 0.9,
        // token_cost MUST equal ceil(utf8_len(content)/4) — let the SDK compute it.
        token_cost: budgetTokens(content),
        valid_from: "2026-01-01T00:00:00Z",
        provenance: [{ type: "file", uri: "file:///docs/start.md", range: "L1-10", digest: `sha256:${"11".repeat(32)}` }],
        citation_label: "start.md L1-10", relations: [],
      }],
      truncated: false,
    };
  },
};

runStdioProvider(provider);
```

Build it, then prove it conformant with the same suite that judges the reference
provider:

```sh
npm run build
contextgraph-inspect stdio --json -- node dist/provider.js
```

### Python quick-start

```sh
pip install contextgraph-sdk   # or, from a checkout: pip install -e ./sdk/python
```

```python
from contextgraph_sdk import run_stdio_provider, budget_tokens


class MyDocsProvider:
    def info(self):
        # Nothing leaves the machine -> declare the honest local-only egress scope.
        return {"name": "my-docs-provider", "version": "0.1.0",
                "data_flow": {"reads": True, "writes": False, "egress": False,
                              "egress_scopes": ["local-only"]}}

    def capabilities(self):
        return {"query": {"kinds": ["doc"]}, "correlation": True, "verify": True}

    def query(self, query):
        content = "Install the binding, then implement the required methods."
        return {"frames": [{
            "id": "doc:1", "kind": "doc", "title": "Getting started", "content": content,
            "content_digest": "sha256:" + ("11" * 32), "score": 0.9,
            "token_cost": budget_tokens(content),   # ceil(utf8_len(content)/4)
            "valid_from": "2026-01-01T00:00:00Z",
            "provenance": [{"type": "file", "uri": "file:///docs/start.md",
                            "range": "L1-10", "digest": "sha256:" + ("11" * 32)}],
            "citation_label": "start.md L1-10", "relations": [],
        }], "truncated": False}


run_stdio_provider(MyDocsProvider())
```

```sh
contextgraph-inspect stdio --json -- python3 my_provider.py
```

`verify` is optional in every SDK — omit it and the host falls back to
re-querying your frames. The runtime handles the whole lifecycle and stays alive
with a typed error on a malformed line rather than crashing. (Go's SDK is the
same shape: implement the `Provider` interface and hand it to
`contextgraph.RunStdioProvider`; see `sdk/go`.)

### Hosting a provider over HTTP

Each SDK ships an HTTP adapter that runs the *same* provider behind one POST
endpoint (the streamable-HTTP transport). You write the provider once; the
transport is a one-line change.

TypeScript — a framework-agnostic handler, here on a plain `node:http` server:

```ts
import { createServer } from "node:http";
import { createHttpHandler } from "@contextgraphprotocol/typescript-sdk";

createServer(createHttpHandler(provider)).listen(8787);
// Under Express: app.post("/contextgraph", createHttpHandler(provider))  — no JSON body-parser on that route.
// Under Fastify:  reply with respondToEnvelopeBody(provider, request.body).
```

Python — a WSGI app, runnable on the stdlib server or any WSGI host (gunicorn,
Flask):

```python
from wsgiref.simple_server import make_server
from contextgraph_sdk import make_wsgi_app

make_server("127.0.0.1", 8788, make_wsgi_app(provider)).serve_forever()
# Under Flask:            app.wsgi_app = make_wsgi_app(provider)
# Under FastAPI (ASGI):   reply with respond_to_body(provider, await request.body()) in your route.
```

Go — a `net/http` handler:

```go
http.ListenAndServe("127.0.0.1:8789", contextgraph.Handler(provider))
```

Point the prober at the running server to confirm it's green:

```sh
contextgraph-inspect http http://127.0.0.1:8787
```

The three wire-level probes (`malformed-input-tolerance`, `embedding-fingerprint`,
`correlation`) report as **skipped** over HTTP — they inspect raw framing the
request/response transport doesn't expose — so a fully conformant HTTP provider
shows those three skipped and every other check green. Runnable examples:
`example-docs-http.ts`, `example_docs_http.py`, and
`examples/example-docs-http/main.go` in each SDK.

### Scaffolding a new provider

To start from a green project rather than a blank file, use the scaffold
generator in `sdk/create-contextgraph-provider`:

```sh
npm create contextgraph-provider@latest my-provider              # TypeScript
npm create contextgraph-provider@latest my-provider -- --lang python
```

It generates a provider wired to both transports **plus a GitHub Actions
workflow that runs `contextgraph-inspect` against it on every push** — so the
generated project is conformant from its first commit, and stays honest as you
replace the example frames with your real retrieval. `npm run conformance` (or
`python scripts/check_conformance.py`) runs the same check locally.
