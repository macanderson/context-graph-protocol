# contextgraph-conformance

The public conformance suite for the **Context Graph Protocol**, plus
`contextgraph-inspect` — an interactive Context Graph Protocol prober analogous to MCP's inspector.

"Context Graph Protocol conformant" means *green on this suite for your declared capability
set* — a checkable claim, which is what makes third-party adoption safe.
[`run_conformance`] drives a provider through the protocol (handshake, a
sample query, shutdown, and a malformed-input probe) and returns a typed
[`ConformanceReport`] with a pass/fail/skip verdict per check and an evidence
string for each, so a failure says exactly what was wrong — never just "not
conformant."

## Checks

| check | what it proves |
|---|---|
| `handshake` | the provider completes the handshake and reports a non-empty identity + capabilities |
| `frame-validity` | every returned frame has a score in `[0, 1]`, a non-empty title, and a non-empty `citation_label` (never a bare id) |
| `budget-honesty` | returned frames' summed `token_cost` never exceeds the query's `max_tokens` |
| `shutdown-clean` | the provider tears down without error |
| `malformed-input-tolerance` | a garbage line on the wire is ignored-or-errored, never crashes the provider (stdio providers only) |

## Run it against your provider

```rust,no_run
use contextgraph_conformance::{ProviderTarget, run_conformance};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let report = run_conformance(ProviderTarget::Stdio {
    program: "my-contextgraph-provider".into(),
    args: vec![],
}).await;

for check in &report.checks {
    println!("{}: {:?} — {}", check.name, check.status, check.evidence);
}
assert!(report.passed(), "not Context Graph Protocol conformant");
# Ok(())
# }
```

Or from the command line with the bundled binary:

```bash
cargo install contextgraph-conformance
contextgraph-inspect stdio -- my-contextgraph-provider
contextgraph-inspect http https://my-provider.example.com/contextgraph
```

`contextgraph-inspect` prints the negotiated capabilities, optionally fires a test
query (`--query "goal text"`), then runs the full conformance suite and
prints a colored (or `--json`) verdict — exiting non-zero when the provider
isn't conformant, so it's CI-friendly.

See [Running conformance][conformance] for the full guide.

## Depends on

[`contextgraph-types`](https://crates.io/crates/contextgraph-types) and
[`contextgraph-host`](https://crates.io/crates/contextgraph-host) — no dependency on
[Stella](https://github.com/macanderson/stella) or any of its other crates.

## Docs

- [Protocol surface][protocol-surface] — the wire types the suite validates
  against.
- [Implementing a provider][implementing] — build something to point this at.
- [Running conformance][conformance] — this crate's full guide.
- [Stability][stability] — the crate-semver vs. protocol-version relationship.

[protocol-surface]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/protocol-surface.md
[implementing]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/implementing-a-provider.md
[conformance]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/running-conformance.md
[stability]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/stability.md
[`run_conformance`]: https://docs.rs/contextgraph-conformance/latest/contextgraph_conformance/fn.run_conformance.html
[`ConformanceReport`]: https://docs.rs/contextgraph-conformance/latest/contextgraph_conformance/struct.ConformanceReport.html

## License

MIT OR Apache-2.0 — see [`LICENSE-MIT`](https://github.com/macanderson/stella/blob/main/LICENSE-MIT)
/ [`LICENSE-APACHE`](https://github.com/macanderson/stella/blob/main/LICENSE-APACHE)
in the workspace root.
