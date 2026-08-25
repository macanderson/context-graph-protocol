# contextgraph-trace

[![crates.io](https://img.shields.io/crates/v/contextgraph-trace.svg)](https://crates.io/crates/contextgraph-trace)
[![docs.rs](https://img.shields.io/docsrs/contextgraph-trace)](https://docs.rs/contextgraph-trace)

The host execution trace (journal) and its replay oracles for the **Context
Graph Protocol**.

> **Sketch stage — not `contextgraph/1.0`.** This crate implements
> a design sketch (removed as stale after `contextgraph/1.0` shipped). It
> is published so downstream hosts can depend on the trace vocabulary by
> version rather than by git rev, but the journal wire format may change in any
> `0.x` release. Gate on the `TRACE_FORMAT` constant, not on the crate version.

The conformance suite (`contextgraph-conformance`) holds a *provider* honest;
nothing holds the *host-side agent loop* honest. This crate is that missing
half, split the same way the rest of the protocol is.

## The journal

`TraceEvent` / `Journal` — an append-only NDJSON record a harness (or a thin
adapter observing one) emits while it works: turns, prompt assemblies,
tool-call pairing, verify observations, side effects, crashes and resumes.

It reuses the protocol's identity spine — frames are named by
`contextgraph_types::FrameId`, verify observations carry the wire
`contextgraph_types::Verdict` — and **no frame body ever travels in it**.

## The oracles

`run_oracles` — pure replay checks over a parsed journal, in the conformance
suite's vocabulary: named checks, pass/fail/skip, evidence naming the exact
`seq` numbers. They catch defects an outcome-graded benchmark structurally
cannot see:

- evidence cited after it was verified stale
- budget arithmetic drifting from the itemization
- phantom tool executions (a result with no matching call)
- side effects replayed across a crash-resume
- resumes blind to their own durable record

The oracles never talk to the harness — they read the journal. That split is
what makes an eventual benchmark runner agent-agnostic: one adapter per harness
maps its native logs onto this vocabulary, and every check downstream is
shared.

```rust,no_run
use contextgraph_trace::{Journal, run_oracles};

let journal = Journal::from_ndjson(&std::fs::read_to_string("trace.ndjson")?)?;
for check in &run_oracles(&journal).checks {
    println!("{}: {:?} — {}", check.name, check.status, check.evidence);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Depends on `contextgraph-types` and serde only, so the oracles stay runnable
anywhere the journal can be read.

## License

MIT OR Apache-2.0
