# Composing MCP and Context Graph Protocol

The README says Context Graph Protocol (CGP) is "complementary to MCP — compose them."
This is that composition, made concrete: one agent session that uses **MCP tools
for actions** and **CGP frames for context**, with a budget audit and citations
that MCP alone does not carry.

The division of labour is the whole point:

- **MCP** is how an agent *acts* — it calls tools that run commands, open pull
  requests, read resources. Its output is a blob of text and a URI.
- **CGP** is how an agent *retrieves context to reason with* — it asks a host for
  frames that carry provenance, an honest token cost, a relevance score, and a
  citation label, all inside a budget the host enforces.

An agent wants both, and it should not have to choose its retrieval stack based
on which protocol its tools happen to speak. Two bridges — one in each direction
— remove the choice. Both ship as crates in this repository, each wrapping a
**hermetic in-repo fixture** so you can run every command below with no network
and no `npx`.

Build the binaries once:

```sh
cargo build --workspace --bins
```

## Direction 1 — an MCP resource server becomes a CGP provider

`contextgraph-mcp-bridge` is an MCP **client** wrapped as a CGP **provider**. It
speaks `initialize` + `resources/list` + `resources/read` to a wrapped MCP
server, then maps each MCP resource to a `ContextFrame`:

| CGP field | Where it comes from |
|---|---|
| `content` | the resource's text (`resources/read`) |
| `token_cost` | the canonical byte count of that text ([budget tokens](./context-reuse.md)) |
| `content_digest` | `sha256` of the served bytes |
| `provenance[0]` | `{ type: "mcp-resource", uri: <mcp uri>, by: <server name> }` |
| `provenance[1]` | a `file` digest a host can re-read, when the resource is a local `file://` |
| `score` | a lexical overlap between the query and the resource |
| `citation_label` | the resource name + its originating MCP server |

The result: **every existing MCP resource server becomes a budgeted, cited,
consent-gated context source with zero changes to it.**

### Probe it with the CGP inspector

Point `contextgraph-inspect` at the bridge, and point the bridge at the fixture
MCP server (everything after the bridge's own `--` is the MCP command it wraps):

```sh
contextgraph-inspect stdio --query "how do we roll out and roll back a deploy" \
  -- ./target/debug/contextgraph-mcp-bridge \
     -- ./target/debug/contextgraph-mcp-fixture
```

The bridge passes the full conformance suite — all fourteen checks, no skips —
exactly as the reference provider does, because it negotiates and honors the
whole surface (`verify`, `graph`, `correlation`, an embedding fingerprint, and
byte-verifiable `file` provenance):

```sh
./.github/scripts/conformance-external.sh \
  -- ./target/debug/contextgraph-mcp-bridge -- ./target/debug/contextgraph-mcp-fixture
# All 14 checks passed — external provider is conformant.
```

### Query it through a host, with a budget audit

Registering the bridge as a stdio provider and calling `Host::query_all` is the
demo the issue asks for — per-provider outcome, budget audit, and citations:

```rust
use contextgraph_host::Host;
use contextgraph_types::ContextQuery;

let mut host = Host::new();
host.add_stdio(
    "mcp",
    "./target/debug/contextgraph-mcp-bridge",
    &["--".into(), "./target/debug/contextgraph-mcp-fixture".into()],
)
.await?;

let query = ContextQuery {
    goal: "how do we roll out and roll back a deploy".into(),
    query_text: Some("roll out and roll back a deploy".into()),
    embedding: None,
    kinds: vec![],           // no filter: ask for every relevant kind
    anchors: vec![],
    max_frames: 8,
    max_tokens: 4096,
    as_of: None,
    representation_preferences: vec![],
};

let fanout = host.query_all(&query).await;

// Frames, each carrying provenance and a citation label:
for (provider, frame) in fanout.accepted_with_provider() {
    println!("[{provider}] {}  ({}tok)", frame.citation_label.as_deref().unwrap(), frame.token_cost);
}

// The budget audit: a self-consistent usage report the host can meter on.
let report = fanout.usage_report(&query, "2026-07-29T00:00:00Z");
assert!(report.is_consistent() && report.within_budget());
println!("budget: {}/{} tokens", report.budget_consumed, report.budget_requested);
```

Every frame's `citation_label` reads like `Deploy runbook (mcp:contextgraph-mcp-fixture)`
— an agent can cite the resource it used, which a raw MCP `resources/read` gives
it no honest way to do.

### Consent posture is transitive

The transport-honesty rule follows the data. A bridge wrapping a **remote** MCP
server declares `egress: true` with an off-machine scope, so a host gates it
behind consent exactly as it would any egress provider:

```sh
# declares egress: true (third-party-index) — the host will not query it until
# consent is recorded, and the query payload never leaves before then.
contextgraph-mcp-bridge --remote --egress-scope third-party-index \
  -- some-remote-mcp-server
```

A local/filesystem MCP server stays `egress: false` (`local-only`). Nothing about
the bridge's frames changes; only the consent gate does.

## Direction 2 — a CGP host becomes an MCP tool

`contextgraph-mcp-server` is the mirror image: an MCP **server** exposing one
tool, `query_context(goal, budget, kinds)`, backed by a CGP host. An agent that
only speaks MCP — Claude Code, say — gets CGP retrieval today, and the
frame-vs-blob difference becomes directly visible in the tool output.

A `tools/call` runs `Host::query_all` and returns the result as MCP **structured
content**: frames with provenance and citations intact, plus a budget audit.

```jsonc
// tools/call → query_context(goal="how do retries and timeouts work", budget=2000)
{
  "content": [{ "type": "text", "text": "Retrieved 2 context frame(s), 61/2000 budget tokens." }],
  "structuredContent": {
    "goal": "how do retries and timeouts work",
    "frames": [
      {
        "provider": "example-docs",
        "id": "frm_retry",
        "kind": "doc",
        "title": "Retry policy",
        "citation": "docs/retry.md L1-12",
        "token_cost": 40,
        "provenance": [{ "type": "derivation", "by": "contextgraph-example-docs", "method": "curated" }]
      },
      { "provider": "example-docs", "id": "frm_timeout", "kind": "snippet", "citation": "src/client.rs L44", "token_cost": 21 }
    ],
    "citations": ["docs/retry.md L1-12", "src/client.rs L44"],
    "budget_audit": { "budget_requested": 2000, "budget_consumed": 61, "within_budget": true, "frames_served": 2 }
  },
  "isError": false
}
```

The `citations` array and per-frame `provenance` are the payload an MCP agent
could not otherwise get from a retrieval tool: it can now say *which* source each
claim rests on, and the host has proven the returned frames fit the budget it
asked for.

Drive it over stdio as any MCP host would:

```sh
./target/debug/contextgraph-mcp-server
# then speak MCP (JSON-RPC 2.0, one message per line): initialize, then
# tools/call query_context
```

## One session, both halves

Putting them together is the composition the README promises. Within a single
agent turn:

1. The agent **acts** through MCP tools — runs the deploy command, opens the
   rollback PR — because that is what MCP tools are for.
2. The agent **retrieves the context to reason with** through CGP — the deploy
   runbook and rollback policy come back as frames with citations and an honest
   cost, whether they were sourced by wrapping an MCP resource server
   (direction 1) or by an MCP-only agent calling `query_context` (direction 2).
3. The host **audits** the turn: a `UsageReport` says exactly how much budget
   each provider spent and itemizes every served frame by stable identity, so the
   agent's citations are backed by a ledger rather than a promise.

MCP moved the world; it is the largest install base of agent tooling there is.
CGP does not replace it — it gives every one of those tools a retrieval layer
that budgets, cites, and gates the context they run on.

See also [Implementing a provider](./implementing-a-provider.md) for the
`ContextProvider` trait and the raw wire, and [Context reuse](./context-reuse.md)
for the budget audit and `context/verify` guarantees this walkthrough leans on.
