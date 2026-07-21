# ADR 0002 — Request correlation, and the JSON-RPC question

- **Status:** accepted
- **Date:** 2026-07-21
- **Issue:** [#4](https://github.com/macanderson/context-graph-protocol/issues/4)
- **Normative:** yes — adds an optional envelope field (GOVERNANCE.md § "What
  counts as a normative change"). Additive, so it stays inside the
  `contextgraph/1` family.

## Context

Two problems, one decision.

**The documentation claimed JSON-RPC; the wire was never JSON-RPC.**
`contextgraph-host/src/wire.rs` described CGP as riding "MCP's transport and
lifecycle conventions (JSON-RPC 2.0, stdio + streamable HTTP,
initialize/capabilities handshake)". The envelope it actually defines is a
bespoke `type`-tagged object: no `jsonrpc` member, no `method`/`params` split,
no `id`. Anyone arriving from MCP notices the mismatch in the first five
minutes, and a spec that misdescribes its own wire has no business asking for
trust on subtler claims.

**No correlation identifier means a lock-step transport.** Because envelopes
carry nothing to match a response to its request, the stdio transport
serializes all traffic behind a mutex over the whole connection
(`RawStdioConnection`). The consequences are structural, not cosmetic:

- one in-flight query per provider, so a slow provider head-of-line blocks;
- no way to express an unsolicited provider→host message, which permanently
  forecloses push invalidation (issue #6) and any future streaming result.

## Options considered

**A. Adopt JSON-RPC 2.0 framing.** `{jsonrpc, id, method: "context/query",
params}`, with the current payloads becoming `params`/`result`. Makes the "rides
MCP conventions" claim true, maximizes familiarity for the MCP-adjacent
audience, and lets non-Rust SDKs reuse mature JSON-RPC libraries.

**B. Add an optional `id` to the existing envelope** and correct the
documentation to describe the wire honestly — an NDJSON envelope informed by
MCP's lifecycle, not JSON-RPC.

**C. Keep lock-step, fix only the documentation.** Rejected outright: it
permanently forecloses notifications and per-provider pipelining, which is too
high a price for saving an optional field.

## Decision

**Option B.** `Envelope` grows an optional `id: Option<String>`. The
JSON-RPC claim is removed from the documentation and replaced with an accurate
description of the framing.

The reasoning, in the order it actually weighed:

1. **Two live downstreams pin this code today** — `stella` and a vendored copy
   in `oxagen-platform` (issues #29, #30). Option A rewrites every example,
   every schema fixture, and both consumers' transport layers. `docs/stability.md`
   permits a breaking `0.x → 0.y`, so Option A is *allowed*; it is not
   thereby *free*. Pre-freeze latitude is a budget, and it should be spent on
   semantics that cannot be added later, not on re-encoding a wire that already
   works.
2. **The repository's stated bias is additive over breaking** (GOVERNANCE.md).
   Option B is a new optional field: a minor change under the rule the project
   already published.
3. **The framing is not where CGP's value lives.** CGP's differentiation is
   typed, budgeted, provenance-carrying, consent-gated frames. JSON-RPC
   conveys none of that; it is an envelope convention. Adopting it would buy
   familiarity, not capability.
4. **The MCP-alignment goal is preserved by other means.** See below.

### What `id` means

- `id` is an opaque, host-generated string, unique per connection and
  per in-flight exchange. Providers **MUST** echo the `id` of the request they
  are answering onto the corresponding `frames` or `error` envelope.
- An envelope with **no** `id` is a **notification**: it expects no reply, and
  correspondingly a reply carrying no `id` cannot be correlated. `handshake` /
  `handshake_ack` / `shutdown` remain valid without one, which preserves every
  byte of the existing wire.
- A provider that does not implement correlation simply omits `id`. A host
  **MUST** fall back to lock-step for such a provider — this is precisely why
  the field is optional, and it is what keeps already-deployed providers
  conformant.
- Concurrency is therefore **negotiated by observation**, not by a capability
  flag: a host may pipeline as soon as it has seen a provider echo an `id`.

### Keeping the MCP door open

A JSON-RPC *binding* — an alternate encoding of the same semantic layer, in
which each envelope maps onto a JSON-RPC request/response — may be specified
later as an additional transport without touching frame or query semantics and
without a new protocol family. Separating the **semantic layer** (frames,
queries, capabilities) from the **transport binding** (NDJSON today, possibly
JSON-RPC and gRPC later) is the structure that lets CGP align with MCP *if and
when* that is worth doing, instead of paying for it speculatively now. This
separation is stated in `SPEC.md` § Transport bindings.

## Consequences

- `stdio.rs` grows a demultiplexer keyed on `id`; the connection mutex shrinks
  to protecting the write half only, so concurrent queries interleave.
- Providers that never send `id` continue to work unchanged, at lock-step.
- The documentation no longer claims JSON-RPC. This is a correction of a false
  statement, and is called out in `CHANGELOG.md`.
- Push notifications (issue #6) become *expressible*. Whether they are
  *specified* is decided separately in [ADR 0004](./0004-dead-capability-surface.md).

## Witness

Per GOVERNANCE.md § "How a normative change lands", the witness for this change
is a test asserting two concurrent queries over a single stdio connection
round-trip to the correct callers, plus the `id` field appearing in
`schema/contextgraph-envelope.schema.json` and in the `examples/` transcripts.
