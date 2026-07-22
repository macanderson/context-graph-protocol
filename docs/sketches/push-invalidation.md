# Design sketch — push invalidation (`context/subscribe`)

> **Status: sketch, not specification.** `Capabilities.subscribe` was removed
> from the `contextgraph/1.0` surface by
> [ADR 0004](../adr/0004-dead-capability-surface.md). Freshness in 1.0 is
> answered by the **pull**-based `context/verify` (issue #26). This file records
> the push design so adding it in 1.x is additive. Nothing here is normative.

## Why push is worth keeping open even though pull shipped first

`context/verify` answers "are the frames I hold still valid?" at a moment the
*host* chooses — a turn boundary, typically. That is the right default, and it
is implementable by every provider including stateless HTTP ones.

It is not sufficient for a long-running agent that reasons for minutes between
turns. Between two verify calls, a file changes and the agent keeps citing dead
evidence. Push closes that window: the provider tells the host the moment frame
`X` is superseded.

The two are complements, not competitors — *push where you can, pull where you
must*.

## The prerequisite, already satisfied

Unsolicited provider→host messages were impossible under the original lock-step
wire: no request ids, one in-flight exchange behind a connection-wide mutex.
[ADR 0002](../adr/0002-request-correlation-and-the-json-rpc-question.md) fixed
exactly this — an envelope carrying **no** `id` is defined as a notification,
and the stdio transport demultiplexes on `id` rather than serializing. Push
invalidation is now *expressible*; it is simply not yet *specified*.

## Sketch

```jsonc
// host → provider, after handshake
{ "type": "subscribe", "id": "sub-1", "kinds": ["doc"], "uris": ["file:///repo/src/**"] }

// provider → host, unsolicited, at any later time (no "id" ⇒ notification)
{ "type": "invalidated", "frame_ids": ["frm_7"], "reason": "source_changed" }
```

Deliberately minimal: **invalidation only**. No re-delivery of new frames, no
live query results, no server-push retrieval. The host reacts by evicting the
frame and, if it still wants that context, issuing an ordinary `context/query`.
Keeping push to a single one-way signal is what stops it from growing into a
second, subtly divergent retrieval path.

## Open questions

- **Lifecycle**: does a subscription survive a provider restart? Almost
  certainly not — the host re-subscribes, and the spec should say so rather than
  leave it to implementations.
- **Backpressure**: a provider watching a busy repository can emit invalidations
  faster than a host consumes them. Coalescing (one notification per frame per
  interval) belongs in the spec, not in each implementation's judgement.
- **Interaction with deterministic composition**: an eviction breaks the cached
  prompt prefix. Batching evictions to turn boundaries preserves the property
  that issue #23 establishes — which is an argument for the host, not the
  provider, deciding when an invalidation takes effect.
- **Conformance**: a declared-subscribe provider must emit an invalidation when
  its backing data changes — testable against the example-docs fixture plus a
  file touch.
