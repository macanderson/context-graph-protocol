# Reference providers

Two reference Context Graph Protocol providers ship in this repository as
`publish = false` binary crates. They exist to exercise the pillars the bundled
conformance fixture cannot fake — **real files with real digests**, and a **real
graph** — against actual bytes on disk, and to give a new host or SDK author a
second and third conformant provider to point at.

| Binary | Serves | Backed by |
| --- | --- | --- |
| `contextgraph-ripgrep` | `Snippet` frames | a content search over a target directory (`rg` if present, else a built-in walk) |
| `contextgraph-treesitter` | `Symbol` + `Graph` frames | a symbol-graph extraction over Rust source |

Both speak the same newline-delimited [`Envelope`](./protocol-surface.md) stdio
protocol as `contextgraph-example-docs`, and both are green on all thirteen
provider-side conformance checks — including the ones that only bite a provider
touching real files: `provenance-fixture-consistency` (every `file` provenance
digest is re-read and re-hashed off disk, `SPEC.md` §6.2) and `anchor-relevance`
(§G4). The shared protocol skeleton lives in the internal `contextgraph-refprov`
crate; each binary only supplies where its frames come from.

## Build

```console
$ cargo build --workspace --bins
```

This produces `target/debug/contextgraph-ripgrep` and
`target/debug/contextgraph-treesitter` alongside `contextgraph-inspect`.

## Verify they are conformant

Point the conformance suite at either binary. No argument is needed — each
defaults to searching its own bundled `fixtures/` directory, which is what CI
probes:

```console
$ ./.github/scripts/conformance-external.sh -- ./target/debug/contextgraph-ripgrep
  OK handshake: provider 'contextgraph-ripgrep' v0.1.0 — ... query kinds=["snippet"], graph=true
  OK frame-validity: 4 frame(s) — scores in [0,1], titles, citation labels, ... well-formed digests, labelled and targeted relations
  OK verify-honesty: provider verified 4 unchanged frame(s) `valid` and all 4 mutated digest(s) `stale`, carrying no frame bodies
  OK budget-honesty: 4 frame(s), 69 tokens within the 4096 budget; every declared cost matches its canonical count
  OK provenance-fixture-consistency: re-read and re-hashed 4 file-provenance digest(s) against the bytes on disk — all match (§6.2)
  ...
All 13 checks passed — external provider is conformant.
```

Or drive one directly with `contextgraph-inspect stdio -- <binary>`.

## Run a fan-out query over this repo

A host registers each provider by the command that spawns it, then fans one
query out to both and composes the accepted frames into a single, byte-stable,
cited context block:

```rust
use contextgraph_host::Host;
use contextgraph_types::ContextQuery;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut host = Host::new();

// Each provider takes an optional target directory; here, this repo.
let root = vec![".".to_string()];
host.add_stdio("ripgrep", "target/debug/contextgraph-ripgrep", &root).await?;
host.add_stdio("treesitter", "target/debug/contextgraph-treesitter", &root).await?;

let query = ContextQuery {
    goal: "how is provenance verified".into(),
    query_text: Some("provenance".into()),
    embedding: None,
    kinds: vec![],           // no kind filter: ask both for their best frames
    anchors: vec![],
    max_frames: 8,
    max_tokens: 4096,
    as_of: None,
    representation_preferences: vec![],
};

let fanout = host.query_all(&query).await;

// Every accepted frame, paired with the provider that served it.
for (provider_id, frame) in fanout.accepted_with_provider() {
    println!(
        "{provider_id}: {} [{}] {}",
        frame.title,
        frame.citation_label.as_deref().unwrap_or(""),
        frame.content_digest.as_deref().unwrap_or(""),
    );
}

// A byte-stable, deterministically-ordered context block for the prompt.
let context_block = fanout.compose();
host.shutdown().await;
# Ok(()) }
```

## The composed frames carry real citations

Each frame names the exact bytes it came from — a `file://` URI, an `L<line>`
range, and a `sha256` digest a host can re-read and re-verify — so the composed
block is auditable, not just plausible:

```text
ripgrep:    reference.md L11          [reference.md L11]   sha256:4860fb52…   (file provenance, range L11)
treesitter: struct Config             [sample.rs L6]       sha256:…           (edges: code.defines)
treesitter: fn parse_config           [sample.rs L11]      sha256:…           (edges: code.defines)
treesitter: sample.rs symbol graph    [sample.rs graph]    sha256:…           (edges: code.defines ×3, code.imports, code.calls)
```

The `contextgraph-treesitter` `Graph` frame's edges are real
`code.defines` / `code.imports` / `code.calls` [`Relation`](./protocol-surface.md)s
between the file's symbols; a graph-aware host can anchor a follow-up query on any
`symbol://…` target and both providers boost the anchored frame to the front
(§G4). Costs are the canonical [`budget_tokens`](./protocol-surface.md) count of
each frame's content, and the response reports `truncated` /`dropped_estimate`
honestly when `max_frames` or `max_tokens` bites.

> **On the tree-sitter name.** `contextgraph-treesitter` ships a self-contained,
> line-based symbol extractor rather than a `tree-sitter` grammar dependency. The
> frames — and their provenance — are just as real; the trade avoids pulling a C
> toolchain build into every CI job for output that need not be byte-exact. See
> the crate docs for what the extractor recognizes.
