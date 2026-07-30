# Context Graph Protocol reference docs

Reference documentation for the **Context Graph Protocol** crates:
[`contextgraph-types`](https://crates.io/crates/contextgraph-types),
[`contextgraph-host`](https://crates.io/crates/contextgraph-host), and
[`contextgraph-conformance`](https://crates.io/crates/contextgraph-conformance).

- [**The Context Graph Protocol: A Technical Overview**](./overview.md) — the
  one-read marketing overview for engineers: the problem Context Graph Protocol solves, the seven
  guarantees, the wire surface, how it relates to MCP, and why you would build
  against it. Start here if you are new to Context Graph Protocol.
- [**The Context Graph Protocol: Advantages and Uniqueness**](./protocol-advantages.md)
  — standalone research analysis of the seven advantages that make Context Graph Protocol a
  qualitatively different approach to context retrieval (provenance, budget
  honesty, consent enforcement, conformance verification, citation guarantees,
  version stability, temporal validity), and why the combination is
  irreducible.
- [**Protocol surface**](./protocol-surface.md) — the wire types: context
  frames, queries, capabilities, provenance. Start here to understand *what*
  Context Graph Protocol is.
- [**Context reuse**](./context-reuse.md) — the four interlocking guarantees
  that make reusing context across turns cache-friendly, auditable, and safe:
  deterministic composition (stable frame identity + canonical ordering), usage
  reports, consent scopes + receipts, and pull-based `context/verify`.
- [**Composing frames into a prompt**](./composing-frames-into-a-prompt.md) —
  the reference host-side composer that turns accepted frames into a prompt: a
  global-budget split across providers, cross-provider dedup, value-aware
  (Lost-in-the-Middle) placement, injection-resistant fenced rendering, plus a
  citation map and an audit record explaining every included and excluded frame.
- [**Implementing a provider**](./implementing-a-provider.md) — how a third
  party builds a CGP provider, in Rust (via `ContextProvider`) or any other
  language (via the wire protocol directly). Start here to *build* something.
- [**Prompt ingestion**](./prompt-ingestion.md) — the paste treated as a local
  provider: intent and anchors extracted, the rest turned into
  content-addressed evidence frames that are compact by default and pulled at
  `[full]` on demand. Provider *policy*, not protocol — a worked example of
  building one.
- [**Running conformance**](./running-conformance.md) — how to prove your
  provider (or host) is Context Graph Protocol conformant, via the `contextgraph-inspect` CLI or the
  `contextgraph-conformance` library. Start here to *verify* what you built.
- [**Conformance registry**](./registry.md) — providers that are Context Graph
  Protocol conformant today, with a reproducible report backing each claim,
  and how to get your own provider listed.
- [**Stability**](./stability.md) — the crate-semver vs. protocol-version
  relationship, and what changes (and doesn't) as the protocol moves from
  `contextgraph/1.0-draft` to `contextgraph/1.0`.

Also at the repo root: [`GOVERNANCE.md`](../GOVERNANCE.md) (how the protocol is
maintained, what counts as a normative change, and the path to `contextgraph/1.0`),
[`SECURITY.md`](../SECURITY.md) (vulnerability reporting),
[`CODE_OF_CONDUCT.md`](../CODE_OF_CONDUCT.md),
[`schema/`](../schema/) (machine-readable JSON Schema for the wire types), and
[`examples/`](../examples/) (diffable wire transcripts).
