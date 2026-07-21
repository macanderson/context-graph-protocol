# Context reuse: identity, accounting, consent, and verification

This is a companion to the [protocol surface](./protocol-surface.md). Where
that page defines the wire shapes for a *single* retrieval, this one defines
the four interlocking guarantees that make **reusing** retrieved context
across turns safe, cheap, and auditable:

1. **[Deterministic composition](#1-deterministic-composition)** — a stable
   frame identity and a canonical ordering, so an unchanged context set renders
   byte-identically and stays friendly to provider prompt caches.
2. **[Usage reports](#2-usage-reports)** — a per-request roll-up of frame costs
   an auditor can walk from an invoice line back to the exact frames.
3. **[Consent scopes and receipts](#3-consent-scopes-and-receipts)** — a closed
   vocabulary of egress classes and a durable receipt, so "did it leave the
   machine, to whom, and who agreed?" is answerable years later.
4. **[Context verification](#4-context-verification)** — a pull-based
   `context/verify` request that revalidates held frames cheaply, without
   re-sending any frame body.

They share one spine: the **frame identity** defined in §1. A usage report
references served frames by it; a verify request carries it. Build order and
dependency run top to bottom.

> **Conformance language.** **MUST**, **MUST NOT**, **SHOULD**, **MAY** in
> **bold** follow [BCP 14 / RFC 2119](https://www.rfc-editor.org/rfc/rfc2119),
> exactly as in [protocol-surface.md](./protocol-surface.md). Each section ends
> with its own conformance requirements; they are also consolidated into the
> [protocol-surface conformance table](./protocol-surface.md#conformance-requirements).

> **Spec-version implications.** Everything here is **additive** within the
> `contextgraph/1` family: new optional fields (`content_digest`,
> `egress_scopes`), new capability-gated methods (`verify`), and new host-side
> artifacts (`UsageReport`, `ConsentReceipt`) that ride no new required wire
> field. A `contextgraph/1.0-draft` provider that implements none of them still
> handshakes and answers queries; a host that wants them degrades to its
> existing behavior (re-query, boolean consent) when a provider opts out. No
> flag day — see [stability.md](./stability.md).

---

## 1. Deterministic composition

### The problem

Provider prompt caches reward a **byte-stable prompt prefix**: Anthropic bills
cache reads at 0.1× input, OpenAI and Gemini cache long prefixes automatically.
Context retrieval is the part of a prompt most likely to destroy that
stability. A host that re-queries its providers each turn and concatenates
frames in arrival order produces a *different* prefix every turn — silently
forfeiting the cache and multiplying the very token costs this protocol exists
to make honest. And nothing today stops two conformant hosts, given the
identical frames, from rendering arbitrarily different prompts.

This is distinct from the reference composition module (budget packing, dedup,
injection-resistance — issue #15). This section is the narrower **contract**
that any composition, reference or not, can satisfy to be cache-friendly.

### Frame identity

A frame's exact bytes are identified by the triple **`(provider id, frame id,
content digest)`**, bound to Rust as
[`FrameId`](https://docs.rs/contextgraph-types/latest/contextgraph_types/identity/struct.FrameId.html):

```rust
pub struct FrameId {
    pub provider_id: String,           // the host's routing/consent key for the serving provider
    pub frame_id: String,              // ContextFrame::id — provider-scoped, stable for dedup
    pub content_digest: Option<String>,// provider-declared, opaque (e.g. "sha256:<hex>")
}
```

- **The `content_digest` is provider-declared and opaque.** A provider chooses
  the algorithm; the reference frames use `sha256:<hex>`, matching the
  `provenance` digests. The protocol never re-derives it. This is deliberate:
  a host that computed the digest from its *own* canonical serialization would
  force every non-Rust provider to byte-exactly reproduce that serialization
  just to answer a [verify](#4-context-verification) request — precisely the
  lock-in the protocol avoids. The cost is that identity is a provider
  *promise* rather than a host-checkable fact; §4's conformance case is what
  holds a provider to that promise.
- A frame **MAY** omit its digest (`content_digest: None` /
  [`ContextFrame::content_digest`](./protocol-surface.md#context-frame) absent).
  Such a frame is **not verifiable**: a host **MUST** treat it as un-revalidatable
  and re-query rather than reuse it unchecked (§4).

Two frames with the same identity **MUST** have the same content bytes;
changing a frame's content **MUST** change its `content_digest` (else §4's
staleness detection cannot work).

### Canonical ordering

The **canonical composition order** is the total order over `FrameId` given by
comparing `provider_id`, then `frame_id`, then `content_digest`
(`Option<String>`, `None` before `Some`). A host that composes a set of frames
into a single context block:

- **MUST** emit them in canonical order, so an unchanged frame set renders
  byte-identically across turns *and* across hosts;
- **MUST NOT** let a frame's `score` or `token_cost` affect the rendered bytes
  — relevance is query-dependent and cost is derived, so a re-query that only
  re-ranks the same frames would otherwise bust the prefix for no content
  change;
- **SHOULD** de-duplicate identical identities to a single rendered block.

The reference host implements this in
[`compose_context`](https://docs.rs/contextgraph-host/latest/contextgraph_host/compose/fn.compose_context.html)
(and [`FanOut::compose`](https://docs.rs/contextgraph-host/latest/contextgraph_host/host/struct.FanOut.html#method.compose)).
Frame `content` is emitted inside an explicit `<frame>…</frame>` fence as
quoted material, never as instructions (protocol-surface R3).

### Prefix stability (informative)

Canonical ordering guarantees cross-turn and cross-host determinism. To
*maximize* cache hits, a host **SHOULD** additionally prefer **append-only**
composition: place newly-retrieved frames after the frames retained from the
previous turn, so a turn that only *adds* context extends the cached prefix
instead of reordering it. This is guidance, not a conformance requirement — a
host that re-sorts the whole set each turn is still conformant, just less
cache-efficient on turns that add high-sorting frames.

### Rationale: the cache economics (appendix)

Why bake this into the protocol rather than leave it to each host? Because the
failure is silent and expensive, and it compounds exactly where this protocol
claims to help. Consider a 20-turn agent session with an 8-frame, ~4,000-token
context block:

- **Arrival-order composition.** Each turn's retrieval returns the same frames
  in a slightly different order (vector search is not order-stable), so the
  prompt prefix changes every turn. Every turn pays full input price on the
  whole context block: `20 × 4,000 = 80,000` context tokens billed at 1×.
- **Canonical-order composition.** The context block is byte-identical on turns
  that don't change the underlying frames. Turn 1 pays 1× to write the cache;
  turns 2–20 read it at 0.1×: `4,000 + 19 × 4,000 × 0.1 = 11,600` tokens. A
  **~7× reduction** on the context portion of the prompt — with no change to
  what the model sees.

The saving is not hypothetical head-room; it is the difference between a
protocol that *measures* context cost honestly (budget honesty, §protocol-surface
B1) and one that also *lets a host control* it. Determinism is the small,
mechanical precondition that turns "we can see the cost" into "we can cut the
cost," and pairing it with [verification](#4-context-verification) is what makes
reuse both cache-friendly *and* safe from serving stale evidence.

### Conformance (§1)

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| D1 | Frames with the same `FrameId` **MUST** have identical content bytes; changing content **MUST** change `content_digest`. | provider contract; `verify` conformance case (§4) |
| D2 | A host composing a frame set **MUST** emit frames in canonical `FrameId` order, independent of arrival order. | `compose_context` round-trip test |
| D3 | Composition **MUST NOT** depend on `score` or `token_cost`. | `compose_context` relevance-invariance test |
| D4 | A frame with no `content_digest` **MUST** be treated as unverifiable and re-queried, never reused unchecked. | host verify fallback (§4) |

---

## 2. Usage reports

### The problem

Every frame carries an honest `token_cost` ([budget honesty](./protocol-surface.md#budget-honesty), B1), but the protocol otherwise stops at the frame. A host that meters context into a billing system — the usage-events → ClickHouse → Stripe loop that platforms reselling agents run — has to invent its own aggregate: which providers served how many frames, at what token cost, against which budget. Every host inventing that shape independently means context cost stays unauditable one level up from the wire — the blob-pipe problem reborn at the accounting layer, where "what did this turn cost, and which sources drove it?" has no standard answer.

### The report

A **usage report** is the per-request roll-up, bound to Rust as
[`UsageReport`](https://docs.rs/contextgraph-types/latest/contextgraph_types/usage/struct.UsageReport.html):

```rust
pub struct UsageReport {
    pub budget_requested: u32,          // the query's max_tokens
    pub budget_consumed: u64,           // summed token_cost of every served frame
    pub as_of: String,                  // the report's accounting snapshot (RFC 3339)
    pub providers: Vec<ProviderUsage>,  // one entry per provider the query reached
}

pub struct ProviderUsage {
    pub provider_id: String,
    pub frames_served: u32,             // accepted (passed consent, timeout, budget audit)
    pub frames_rejected: u32,           // dropped — e.g. a budget lie, or (§4) a stale verify verdict
    pub token_cost: u64,                // this provider's contribution to budget_consumed
    pub served_frames: Vec<ServedFrame>,// each served frame, by identity + declared cost
}

pub struct ServedFrame {
    pub frame: FrameId,                 // the stable identity from §1
    pub token_cost: u32,                // the cost this frame contributed
}
```

Three properties make it usable as an accounting record rather than a debug dump:

- **It is a host-side artifact, not a wire message.** No new envelope variant, no new required field: a provider implements nothing to make one possible. The report rides the frames a query already returned.
- **It references served frames by their §1 identity.** `budget_requested` / `budget_consumed` are the roll-up an invoice line quotes; `served_frames` is the drill-down an auditor walks — from a billed total to the exact `(provider id, frame id, content digest)` triples behind it.
- **Its `as_of` is the report's snapshot time**, stamped by the producing host — *not* a query's bi-temporal [`as_of`](./protocol-surface.md#context-query) retrieval pin. The two are different clocks: one dates the accounting event, the other dates the facts retrieved.

The reference host produces one from a fan-out with
[`FanOut::usage_report(&query, as_of)`](https://docs.rs/contextgraph-host/latest/contextgraph_host/host/struct.FanOut.html#method.usage_report).
Accepted frames are itemized; a budget-lying provider's dropped frames count as
`frames_rejected` and contribute zero cost; a consent-gated or failed provider
served nothing. Because the host sums the totals from the very frames it
itemizes, the result always satisfies the arithmetic identity below.

### The arithmetic identity

The report is **self-verifying**: `budget_consumed` re-sums from
`providers[].token_cost`, which re-sums from `served_frames[].token_cost`. A
metering pipeline checks this before trusting a total —
[`UsageReport::is_consistent()`](https://docs.rs/contextgraph-types/latest/contextgraph_types/usage/struct.UsageReport.html#method.is_consistent)
is the one call — so a corrupted or tampered total is a checkable arithmetic
failure, never a silent misbill. The conformance case (below) proves the
identity holds against a *real* provider: it re-sums the accepted frames
independently and asserts equality with the report the host built.

### Worked example: mapping a report into a metering pipeline

A reseller host bills its customers for context by the token. After each turn it
takes the fan-out's report and fans it out into per-provider usage events:

```rust
// `as_of` is caller-supplied: the host stamps the report with its own RFC 3339
// clock (the accounting-event time), keeping the type free of a time dependency.
let report = fanout.usage_report(&query, now_rfc3339());
assert!(report.is_consistent()); // refuse to bill an inconsistent total

for provider in &report.providers {
    // One append-only usage event per (turn, provider) — the ClickHouse row.
    meter.emit(UsageEvent {
        request_id,
        turn_id,
        provider_id: &provider.provider_id,
        frames_served: provider.frames_served,
        frames_rejected: provider.frames_rejected,
        tokens: provider.token_cost,         // the metered quantity
        as_of: &report.as_of,
        // The drill-down: the exact frames behind this line, for dispute
        // resolution and audit. Stored alongside the event, not billed twice.
        frames: &provider.served_frames,
    });
}
```

- **ClickHouse (append-only runtime events):** each `UsageEvent` is one row.
  `tokens` is the metered measure; `provider_id` and `as_of` are the grouping
  keys; `frames` (the `ServedFrame` list) is the citation trail an auditor
  follows from a monthly total back to individual turns and frames.
- **Stripe (billing):** a periodic job sums `tokens` per customer over the
  billing window and reports it to a Stripe metered price. Because
  `budget_consumed` equals the summed `token_cost` of served frames by
  construction, the number a customer is billed is the number the frames
  actually cost — the honest-cost guarantee carried all the way to the invoice.
- **Dispute path:** a customer questioning a charge is answered by the same
  `served_frames` identities — every billed token maps to a cited frame with a
  provider, a frame id, and a content digest, not an opaque aggregate.

This is the payoff of pairing the report with §1's identity: the identity makes
the frames *nameable*, and the report makes them *countable*, so context cost is
auditable from the wire all the way up to the invoice line.

### Conformance (§2)

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| U1 | A host **MUST** be able to produce a usage report for any query it executed, whose `budget_consumed` equals the summed `token_cost` of the served frames it reports. | `FanOut::usage_report`; `usage_report` conformance case (drives the real fixture, re-sums independently) |
