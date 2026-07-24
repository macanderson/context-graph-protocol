# Sketch: `context/resolve` (a post-1.0 additive minor)

**Status:** not in `contextgraph/1.0`. This sketch keeps the door open so the
representation fields can freeze now (they already travel on the wire) while the
*operation* that dereferences a `content_ref` lands later without a breaking
change. See [SPEC.md §6.4.1](../../SPEC.md) and
[ADR 0005](../adr/0005-frame-representations.md).

## Why it is deferred

`contextgraph/1.0` freezes the `full`/`compact`/`reference` **frame shapes** and
the `content_ref` handle, but defines **no wire operation** that turns a handle
into bytes. Today the only producer of `compact`/`reference` frames is the
in-process prompt-ingestion provider, which rehydrates from its own artifact
store inside the host's address space — a host-internal concern, not a protocol
message. A *remote* (stdio/HTTP) provider therefore cannot have a `reference`
frame rehydrated over the wire in 1.0, which is exactly why SPEC.md §6.4.1 tells
remote providers to prefer `compact` or `full`.

Freezing an operation with one in-process caller and no wire consumers would
reintroduce the dead-capability-surface anti-pattern ADR 0004 removed. Better to
ship the honest subset and add the operation when a concrete cross-wire consumer
forces its design.

## Shape it would take

Two envelopes, correlated by `id` like `query`/`frames`:

```jsonc
// host → provider
{ "type": "resolve", "id": "r1",
  "request": { "refs": [
    { "provider_id": "acme-index", "uri": "cref://acme/9f2a…" }
  ] } }

// provider → host
{ "type": "resolved", "id": "r1",
  "response": { "contents": [
    { "ref": { "provider_id": "acme-index", "uri": "cref://acme/9f2a…" },
      "content": "…full source bytes…",
      "canonical_content_hash": "sha256:<64 hex>",
      "token_cost": 812 }
  ] } }
```

Design constraints it must honor:

- **Verifiable rehydration.** The returned `content` MUST hash to the
  `canonical_content_hash` the original `reference`/`compact` frame carried, so a
  host proves it got the real source and not a substitute — the same
  digest-honesty discipline as F5/D-series.
- **Routing.** `content_ref.provider_id` already names the provider that must
  answer, so a fan-out host routes a resolve back to the exact source.
- **Consent.** A resolve transmits nothing new *about the workspace*, but it may
  move source content off-machine if the provider is an egress provider; it rides
  the same C-series consent gate as `query`.
- **Capability.** `capabilities.resolve` becomes a real, exercisable promise:
  advertising it obligates answering `resolve`. Its 1.0 meaning (a
  forward-declaration / shape check) tightens into a callable contract — an
  additive move, since 1.0 hosts never sent a `resolve` to rely on.
- **Errors.** A handle that no longer resolves answers `error` with an
  `expired`/`gone`-class code (open vocabulary, §10 X1).

## Migration note

Because 1.0 hosts never emit `resolve`, adding these two envelopes is a clean
minor bump: a 1.0 provider that does not implement them is unaffected (it never
receives one), and a 1.x host discovers support through `capabilities.resolve`
exactly as it discovers `verify` today.
