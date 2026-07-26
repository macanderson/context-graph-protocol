# Sketch: `context/feedback` — shipping attribution back to a provider

**Status:** deferred to a 1.x additive minor. Not part of `contextgraph/1.0`.

Companion to the shipped half: `contextgraph-types::attribution`
(`AttributionReport`, `ContextUse`) and SPEC.md §14.1. This sketch records the
wire hop that is *not* being built for the freeze, and why.

## What shipped, and what did not

Issue #31 asked for two things:

1. a stable per-item attribution id, and
2. an optional, capability-gated host→provider feedback surface.

The first needed no new surface at all. `FrameId` — `(provider id, frame id,
content digest)` — is already the identity composition, dedup, usage reports,
and `verify` all key on. Minting a second id for attribution would let the two
disagree, and a disagreement between *the frame that was billed* and *the frame
that was cited* is exactly what attribution exists to prevent. §14.1 states this
normatively.

The second is deferred. The attribution **vocabulary** is specified now, because
that is the half that must be shared for scores to be comparable across
implementations. The transport is not.

## Why the wire hop is deferred

Shipping `Capabilities.feedback` plus a `context/feedback` envelope in 1.0 would
recreate the defect this repo has spent the pre-freeze work removing:

- ADR 0004 cut `upsert`, `subscribe`, and `filters` because each was a capability
  no host could exercise — declared at the handshake, unreachable in practice.
- §Q1 had to be *written* because `kinds` shipped as a request field with
  documented syntax that every implementation ignored, silently.

A feedback method with no provider consuming it and no conformance check able to
witness it would be a third instance, added days before a freeze. The
family-compatibility asymmetry makes deferring cheap and shipping expensive:
**adding** `context/feedback` after 1.0 is a family-safe additive minor;
**removing** a dead `feedback` capability after 1.0 is family-breaking.

Meanwhile nothing is blocked. `AttributionReport` lets a host score retrieval
locally today — which is what the concrete consumer (a host that A/B-suppresses
recall to measure whether retrieval earns its budget) actually needs. The wire
hop matters only once a provider wants to *act* on the signal, e.g. by
re-ranking, and no such provider exists yet.

## The shape, when it lands

```jsonc
// host -> provider, fire-and-forget; no reply envelope.
{
  "type": "feedback",
  "report": {
    "as_of": "2026-07-25T12:00:00Z",
    "uses": [
      { "frame": { "provider_id": "docs", "frame_id": "frm_retry",
                   "content_digest": "sha256:…" },
        "selected": true, "rendered": true, "cited": true }
    ]
  }
}
```

Negotiated by `Capabilities.feedback: bool`. A host MUST NOT send it to a
provider that does not advertise it, and MUST NOT expect a reply — a provider
that ignores feedback entirely is conformant, so the exchange stays one-way and
cannot become a latency dependency on the query path.

### Open questions to settle before it ships

- **Privacy.** `cited` leaks something about the model's output back to the
  provider, which for an `egress: true` provider is a new outflow the user never
  consented to at install time. Does feedback need its own egress scope (§4.1),
  or its own consent gate?
- **Batching.** Per-request, or accumulated? Per-request is simpler and matches
  `UsageReport`; accumulated is cheaper but needs a flush contract.
- **Conformance.** What does a `feedback` check even assert, given a conformant
  provider may do nothing observable with it? Probably: it accepts the envelope
  and stays alive (an R1-shaped check), which is weak — and that weakness is
  itself an argument for waiting until a provider consumes it.
- **Honesty.** Nothing stops a host from reporting `cited: true` for everything.
  Attribution is a host self-report; unlike `token_cost` (§B3) there is no
  canonical rule to check it against, so it is trust-scoped to the host.

## References

- SPEC.md §14.1 (attribution identity and vocabulary)
- ADR 0004 — dead capability surface
- ADR 0007 — the protocol/product boundary, where the `context_use` vocabulary
  (`selected` / `rendered` / `cited`) comes from
- Issue #31, issue #8 (token cost — the other half of value-per-token)
