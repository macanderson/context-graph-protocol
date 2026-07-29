# Sketch: `context/neighbors` (a post-1.0 additive minor)

**Status:** not in `contextgraph/1.0`. This sketch keeps the door open so the
graph shapes can freeze now — `relations`, the `rel` vocabulary, and the G4
*anchored* predicate all travel on the wire today — while the *operation* that
walks those edges beyond one hop lands later without a breaking change. See
[SPEC.md §8](../../SPEC.md) and the G3/G4 rows there.

## Why it is deferred

`contextgraph/1.0` freezes what a graph frame **is** (a node with labelled
edges, §8) and pins the one traversal semantics a suite can witness: G4's
*anchored* predicate — a frame is reachable from an anchor URI at zero hops (its
own `uri`) or one hop (any `relations[].target_uri`). That floor is deliberate.
It is decidable by string equality, so `anchor-relevance` can actually catch a
provider that ignores `anchors`, and it is a floor on what must be *found*, not
a ceiling on how far a provider may look internally.

Multi-hop traversal *as a wire operation* is a different promise. There is no
cross-wire consumer of it in 1.0: the host fans a `query` out, receives frames
with their edges, and composes — it never asks a provider "give me the
neighborhood of this node to depth 3." Freezing a `neighbors` operation now,
with no host emitting it, would reintroduce exactly the dead-capability-surface
anti-pattern [ADR 0004](../adr/0004-dead-capability-surface.md) removed. Better
to ship the honest one-hop floor and add the operation when a concrete traversal
consumer (an agent walking a call graph, a "why is this here" impact query)
forces its design.

## Shape it would take

Two envelopes, correlated by `id` like `query`/`frames`:

```jsonc
// host → provider
{ "type": "neighbors", "id": "n1",
  "request": {
    "uri": "symbol:///repo/src/host.rs#FanOut::compose",
    "rels": ["code.calls", "code.references"],  // optional filter; absent ⇒ all
    "depth": 2,                                    // hops from the seed node
    "budget": 4000                                 // token budget, as on query
  } }

// provider → host
{ "type": "neighbored", "id": "n1",
  "response": {
    "seed": "symbol:///repo/src/host.rs#FanOut::compose",
    "frames": [ /* ContextFrame[], same shape as `frames` */ ],
    "truncated": false,
    "dropped_estimate": 0
  } }
```

Design constraints it must honor:

- **Built on G4, not beside it.** `depth: 1` with no `rels` filter **MUST**
  return exactly the anchored set G4 already defines for that URI, so the
  operation is a strict generalization of the predicate the suite pins in 1.0 —
  not a second, subtly different notion of adjacency.
- **Bounded and honest.** `depth` and `budget` are hard caps. A provider that
  can't return the full neighborhood within them **MUST** set `truncated: true`
  and a `dropped_estimate`, reusing the B4 frame-flood discipline rather than
  silently pruning — a traversal that hides what it dropped is a budget liar.
- **Cycle-safe.** Graphs have cycles; a node **MUST NOT** appear twice in
  `frames`, and revisiting a node does not spend depth twice. Identity is the
  `FrameId` triple (§6.3), so dedup is the same operation the host already does
  when composing a fan-out.
- **Verifiable frames.** Returned frames carry `token_cost`, `content_digest`,
  and provenance under the same rules as any `query` result (§7, §6.3) — a
  neighborhood is not a privileged shape, just a differently-selected one.
- **Capability.** A new `capabilities.neighbors` gates it, and it **MUST**
  co-require `capabilities.graph` (a provider with no edges has no neighbors to
  walk). Advertising `neighbors` obligates answering it; a 1.0 provider that
  declares only `graph` is unaffected because a 1.0 host never sends one.
- **Consent.** A `neighbors` call selects among content the provider already
  indexes; like `query` it moves nothing new *about the workspace*, but if the
  provider is an egress provider it may move source off-machine, so it rides the
  same C-series consent gate as `query`.
- **Errors.** A seed `uri` the provider doesn't know answers `error` with a
  `bad_request`-class code; exceeding a provider-internal traversal limit answers
  with an `unavailable`-class code (open vocabulary, §10 X1).

## Migration note

Because 1.0 hosts never emit `neighbors`, adding these two envelopes is a clean
minor bump: a 1.0 provider that does not implement them is unaffected (it never
receives one), and a 1.x host discovers support through `capabilities.neighbors`
exactly as it discovers `verify` today. The `depth: 1` ≡ G4 identity above means
the freeze's one witnessed traversal rule survives verbatim into the richer
operation, so nothing a 1.0 suite asserted about anchoring is invalidated.
