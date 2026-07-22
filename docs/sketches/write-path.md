# Design sketch — the write path (`context/upsert`)

> **Status: sketch, not specification.** `Capabilities.upsert` was removed from
> the `contextgraph/1.0` surface by [ADR 0004](../adr/0004-dead-capability-surface.md)
> because it had no method, no host API, and no implementation. This file
> records the design so re-adding it in 1.x is an additive minor rather than a
> rediscovery. Nothing here is normative.

## The motivating consumer

An episodic-memory provider: an agent finishes a task, and the lesson learned
("the analytics deploy needs `AWS_REGION` set before the migration step") should
outlive the session. Today that provider can serve `Memory`/`Episode` frames but
has no protocol-level way to *receive* them, so every host invents a side
channel — which is the blob-pipe problem re-emerging at the write side.

## Questions the design must answer before it is specified

**Payload shape.** A full `ContextFrame` is wrong: `score` is relevance to a
*query* and is meaningless on a write, and `token_cost` is derivable
(ADR 0003). A reduced `WriteFrame` — `kind`, `title`, `content`, `uri`,
provenance, validity window — is the likely shape, with the provider assigning
`id` and returning it.

**Idempotency.** A retried write must not duplicate. Keying on a
client-supplied idempotency token is the conventional answer; keying on content
digest is cheaper but conflates "same bytes" with "same event", which is wrong
for episodes (the same lesson learned twice on different days is two episodes).

**Consent.** `DataFlow.writes` (retained by ADR 0004) declares that a provider
persists. Whether a *write* needs its own consent gate distinct from `egress` is
genuinely open: writing to a local provider is arguably ungated, while writing
to a remote one is already covered by the egress gate. The safe default is a
separate gate, because "you may read my workspace" and "you may durably record
things about me" are different grants.

**Partial failure.** A batch write needs per-item outcomes, matching the
per-provider outcome discipline the fan-out already uses.

## Minimum bar for re-adding it

Per ADR 0004's reasoning, the bar is a **working provider that needs it**, plus:
spec text, envelope variant, schema entry, and at least two conformance checks
(a declared-upsert provider persists and re-serves a frame; a non-upsert
provider rejects with a clean typed error — `unsupported_kind` or a new
`unsupported_method`, see the error-code table in `SPEC.md`).
