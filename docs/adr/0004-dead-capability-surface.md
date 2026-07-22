# ADR 0004 — Dead capability surface: `upsert`, `subscribe`, `filters`

- **Status:** accepted
- **Date:** 2026-07-21
- **Issues:** [#5](https://github.com/macanderson/context-graph-protocol/issues/5),
  [#6](https://github.com/macanderson/context-graph-protocol/issues/6),
  [#11](https://github.com/macanderson/context-graph-protocol/issues/11)
- **Normative:** yes — removes fields from the wire types. Permitted pre-freeze
  under `docs/stability.md` ("any `0.x → 0.y` bump may contain breaking changes
  to either the Rust API or the wire shape"); it would **not** be permitted
  after the `contextgraph/1.0` freeze.

## Context

Three corners of the handshake were negotiable but unusable — a provider could
declare a capability that no host on earth could exercise.

| Field | Declared at handshake | Reachable? |
| --- | --- | --- |
| `Capabilities.upsert` | yes | No `Upsert` envelope variant, no host API, no schema entry, no conformance check |
| `Capabilities.subscribe` | yes | No subscription method, no notification envelope, no semantics anywhere |
| `QueryCapability.filters` | yes | `ContextQuery` has no field with which to *send* a filter |

The pre-freeze asymmetry is the whole reason to act now, and it is worth
stating precisely because it is the load-bearing argument: **after the freeze,
adding `context/upsert` is a cheap additive minor, while removing a dead
`upsert` field requires a whole new `contextgraph/2` family.** Deciding today
costs a downstream recompile. Deferring makes the wrong default permanent.

## Decision

### 1. `Capabilities.upsert` — removed

There is no write-path consumer. A speculative write API frozen without an
implementation is exactly how protocols accrete regret: the payload shape
(full frames? a reduced write-frame with no `score`?), the idempotency key, the
consent interaction, and partial-failure reporting would all be guessed rather
than learned.

The episodic-memory use case that motivates a write path is real, and it
deserves a design driven by a working provider. `docs/sketches/write-path.md`
keeps the door visibly open.

### 2. `DataFlow.writes` — **kept**, and redefined

Issue #5 proposed removing `DataFlow.writes` alongside `upsert`. This ADR
**declines that half of the proposal**, and the divergence is deliberate.

`Capabilities` and `DataFlow` answer different questions. `Capabilities` says
*what methods you may call on me* — and with `context/upsert` gone, `upsert`
answers a question nobody can ask. `DataFlow` says *what I do with data you
give me*, which is the input to the consent gate. Those come apart: a provider
with no write method at all still persists data if it indexes the queries it
receives, and a user consenting to egress deserves to know that.

`DataFlow.writes` is therefore retained with a corrected definition — no longer
"persists `context/upsert` writes" (a dangling reference to a removed method)
but:

> **`writes`** — the provider durably persists data derived from what it
> receives, such as indexing query payloads or retaining request logs. This is
> a consent-surface declaration; it does not imply any host-callable write
> method.

Removing it would have quietly narrowed the consent surface as a side effect of
tidying an unrelated capability flag, which is the wrong trade in a protocol
whose security story *is* the consent gate.

### 3. `Capabilities.subscribe` — removed; freshness is answered by pull

Same reasoning as `upsert`: no method, no notification envelope, no semantics.

The freshness guarantee ("is it still true?") is answered instead by
**`context/verify`** (issue #26) — a pull-based revalidation in which the host
sends frame identities and the provider answers `valid` / `stale` / `gone` /
`unknown`, with no frame body travelling in either direction. Pull is the right
default: many providers cannot push (stateless HTTP, batch indexes), and many
hosts do not want a subscription's lifecycle merely to ask a question at a turn
boundary.

Issue #26 asked that the freeze be able to keep "both, either, or one" of push
and pull. **This ADR consciously chooses one — pull — for 1.0**, rather than
letting push lapse by omission. The push path is not foreclosed:
[ADR 0002](./0002-request-correlation-and-the-json-rpc-question.md) makes
unsolicited provider→host messages *expressible* by defining an envelope with
no `id` as a notification, which is the exact prerequisite push invalidation
needs. `docs/sketches/push-invalidation.md` records the design so the 1.x
additive path is genuinely open and not accidentally closed.

### 4. `QueryCapability.filters` — removed

The capability is unreachable by construction: there is nothing in
`ContextQuery` to carry a filter. Specifying a grammar now means designing
`language:rust` / `path:src/**` semantics — provider-declared keys, unknown-key
behaviour, escaping, conjunction rules — with no implementation forced to live
with the result.

`kinds`, `anchors`, and `query_text` cover the reference providers planned in
issue #18. Those providers are the intended forcing function: if writing them
proves filters necessary, the design lands additively in 1.x with a real
consumer behind it, which is a strictly better position than the one being
abandoned here. `docs/sketches/query-filters.md` records the open design
questions.

## Downstream impact

`stella` and the vendored copy in `oxagen-platform` pin this code (issues #29,
#30). Removing three fields is a source-breaking change for them. Mitigations:

- `MIGRATION.md` documents each removal and the one-line fix (the fields were
  never readable in any meaningful way, so every call site is a struct literal
  or a `..Default::default()`).
- The downstream canary CI job (issue #29) exists precisely so this class of
  break is visible here before a downstream discovers it.
- Because the fields carried `#[serde(default)]`, **existing wire messages
  continue to deserialize**: an old provider that still emits `"upsert": true`
  is not rejected, the field is simply ignored. The break is at the Rust API,
  not on the wire.

## Consequences

- Three fields removed from `contextgraph-types`, the JSON Schema, the
  examples, and `docs/`.
- Three design sketches committed under `docs/sketches/` so each door is
  visibly open rather than silently shut.
- `CHANGELOG.md` records all three removals under `[Unreleased]` as breaking.
- The handshake surface now contains nothing a host cannot exercise, which is
  the property GOVERNANCE.md's freeze criterion ("the conformance suite's
  checks are agreed to fully enforce the documented conformance requirements")
  actually requires.
