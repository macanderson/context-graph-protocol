# 0007 — The protocol/product boundary: atomic `ContextFrame` vs host-owned `CompiledContextFrame`

**Status:** Accepted (draft; `contextgraph/1.0-draft`)

## Context

In July 2026 an "adaptive-context" spec bundle was authored and merged into two
downstream repos:

- **stella** — `docs/design/context-frame-spec.md`, `docs/design/directive-schema.md`,
  `docs/design/adaptive-context-lifecycle.md`, and a build prompt addressed *at
  this repo* (`docs/design/context-graph-protocol-build-prompt.md`).
- **oxagen-platform** — `docs/specs/adaptive-context/` (PR #1074), which vendored
  editable copies of stella's lifecycle spec and this repo's build prompt, plus
  its own "Context Exchange Provider" plan.

Those documents define frame semantics — bounded, typed, provenance-native
frames; "the six questions a frame answers"; directive taxonomies; bi-temporal
memory — that overlap the surface this repo is meant to own. By the time the
reconciliation began (issue [#27](https://github.com/macanderson/context-graph-protocol/issues/27))
there were three sources of truth drifting apart. This ADR records the boundary
that resolves the drift. It is the boundary the build prompt itself asked this
repo to write down ("an ADR stating the protocol/product boundary and why
`ContextFrame` remains atomic").

### The drift is, at its root, a name collision

Two different things were both being called `ContextFrame`:

1. **The atomic retrieval envelope.** One frame = one snippet, symbol, fact,
   doc, memory, episode, or graph node, carried with its provenance, score,
   `token_cost`, citation label, temporal profile, and (post-[ADR 0005](./0005-frame-representations.md))
   representation. This is what this repo has *implemented and conformance-tests*
   (`contextgraph-types::ContextFrame`, `SPEC.md` §6). A provider returns a
   `Vec<ContextFrame>` from `context/query`.

2. **The task-wide compiled aggregate.** The whole context package assembled for
   one agent turn: the task and its success criteria, entity bindings, six typed
   directive buckets, bi-temporal memories, observations, artifact contracts, a
   code-map slice, evidence, an *excluded* list, citations, and a
   `FrameMetadata` carrying selection/authorization/promotion **policy
   versions**. This is what the stella bundle calls `ContextFrame`.

These are not the same object at different zoom levels; they are a mechanism and
a product. The second one is a *host's* compilation decision — which frames to
admit, how to budget them, which policy versions applied, what got excluded and
why. None of that is portable wire semantics. The build prompt's own ownership
table already names it correctly: it is a **`CompiledContextFrame`**, host-owned,
**not a protocol type**. Once the two names are separated the boundary is
obvious, and most of the "drift" turns out to be the aggregate's fields leaking
into the envelope's spec.

## Decision

### 1. Two names, one owner each

| Name | Definition | Owner |
|---|---|---|
| `ContextFrame` | One atomic retrieval envelope — the canonical result unit of `context/query`. | **Protocol (this repo).** |
| `CompiledContextFrame` | The task-wide aggregate a host assembles for one agent turn (task, directive buckets, code-map, excluded set, citations, policy-version metadata). | **Host (stella).** Never a protocol type. |
| `PromptContext` | The rendered prompt string/segments a host emits from a `CompiledContextFrame`. | **Host (stella).** Never a protocol type. |

The protocol `ContextFrame` remains the canonical atomic result of a provider
query. It is **not** replaced by, renamed to, or widened into the task-wide
aggregate.

### 2. What the protocol owns

Wire types and canonical serialization; typed record identity, relationships,
provenance, scope, sharing, and time; frame representation negotiation
(full / compact / reference) and opaque reference resolution; the atomic
`context/query` retrieval operation and — on the exchange-provider profile track
(issue [#28](https://github.com/macanderson/context-graph-protocol/issues/28)) —
immutable record append/get; idempotency, batching, receipts, typed errors,
payload limits, and timeout behavior; capabilities, JSON Schemas, examples,
compatibility rules, and conformance tests.

### 3. What the protocol does *not* own (host / product concerns)

Observation extraction from traces, logs, git, or user behavior; confidence
formulas and recurrence thresholds; solo/team/regulated governance policy;
Keep/Edit/Ignore review UI; automatic activation, confirmation, publication, or
pruning decisions; blocking authorization and security permissions; artifact-
contract *execution* and semantic judging; prompt compilation, token
*allocation/budgeting*, snapshots, or aggregate deltas; the SQLite schema, rule
files, code-owner routing, or Context-PR workflows; product packaging or
telemetry policy. **The rule of thumb: mechanism in the protocol, policy in the
host.** Protocol data never grants enforcement authority — the protocol carries
a value; it does not authorize acting on it.

### 4. Frames carry evidence, not directives — but the exchange profile may carry directive *records*

These are two different layers and must not be conflated:

- **In the retrieval/frame layer**, frame `content` is *untrusted evidence,
  quoted and cited, never a directive the model executes* (`SPEC.md` §6/R3,
  `docs/protocol-advantages.md`). This is unchanged. The atomic `ContextFrame`
  has no `directive` field and gains none.
- **In the exchange/record layer** (the Context Exchange Provider profile,
  issue #28), a `directive` may exist as one immutable, provenance-bearing
  **record kind** that a provider stores and serves. Carrying a directive record
  is not the same as a frame instructing a model: the host still decides whether
  any directive is admitted, enforced, or authorized. This ADR deliberately does
  **not** foreclose the #28 profile from defining a `directive` record.

When that profile defines directive records, the portable directive taxonomy is
`preference | rule | constraint | procedure` (four kinds). `memory` and `fact`
are **not** portable directive kinds — they are separate record kinds
(`memory`; `knowledge`, with `fact` a knowledge kind). The six-kind directive
taxonomy in the superseded downstream drafts is a host-runtime convenience, not
a wire contract.

### 5. Naming: Context Graph Protocol (CGP), not "Context Graph Exchange Protocol" (CGEP)

The stella build prompt and the oxagen lifecycle spec proposed renaming the
protocol to "Context Graph **Exchange** Protocol / CGEP", a `cgep/1.0-draft`
capability namespace, and a `context-graph-exchange-protocol` repo. **This is
rejected.** The canonical name is **Context Graph Protocol (CGP)**; the wire
version is **`contextgraph/1.0-draft`**; the capability/namespace stem is
**`contextgraph`**. Any item adopted from the build prompt is normalized to this
naming. (Public docs spell out "Context Graph Protocol (CGP)" on first use, then
use "CGP" freely.)

### 6. Providers are optional and unnamed in the wire

stella is a complete local/BYOK host. oxagen is *one optional commercial
provider and control plane*. No provider name, endpoint, product tier, policy
default, or database semantic is hard-coded into the portable protocol; a
non-oxagen provider must be implementable from the spec, schema, examples, and
the #28 profile alone.

## Consequences

- **Downstream docs stop restating frame semantics.** stella's
  `context-frame-spec.md` / `directive-schema.md` and oxagen's vendored copies
  become pointers to this repo for the atomic frame + wire semantics, keeping
  only genuinely host-side material (the aggregate, compilation, budgeting,
  learning, governance) — which they describe against their *own code's* type
  names, not this ADR's.
- **The single source of truth for the atomic frame is `SPEC.md` + the schema.**
  Completing `SPEC.md` so it documents the already-shipped representation/verify/
  identity surface is issue [#49](https://github.com/macanderson/context-graph-protocol/issues/49).
- **The lifecycle/records layer lands as a profile, not as core creep.** The
  build prompt's record taxonomy, append/get/resolve operations, capability
  negotiation, and typed errors are folded into the exchange-provider profile
  (#28) and the write/resolve issues (#5, #50) rather than widening the frozen
  1.0 core.
- **Re-drift is structurally prevented, not linted.** With the normative frame
  text living in exactly one place and the downstream docs holding only a pinned
  pointer, there is nothing left to drift. The downstream canary
  (issue [#29](https://github.com/macanderson/context-graph-protocol/issues/29))
  guards the *code* side by building stella and the oxagen copy against this
  repo's HEAD.

See [`docs/adaptive-context-reconciliation.md`](../adaptive-context-reconciliation.md)
for the item-by-item delta table this ADR is the anchor for.
