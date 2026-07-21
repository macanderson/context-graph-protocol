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

---

## 3. Consent scopes and receipts

### The problem

Consent-gating is one of the seven guarantees: retrieval that transmits workspace content off-machine must be agreed to. `DataFlow.egress` already gates it — but a boolean answers the question only *in the moment it is asked*. When an auditor asks, months later, "**what** left the machine, **to whom**, and **who** agreed?", a `true` in a since-exited process is no answer. And without a shared vocabulary of egress classes, "consent" means something different to every provider.

This section makes consent an **artifact** rather than an event, in two parts: a closed **scope vocabulary** that classes *where* content goes, and durable **consent receipts** that record each grant.

### The egress-scope vocabulary

A provider declares — alongside the boolean `egress` — the [egress scopes](./protocol-surface.md#handshake--capability) its served content falls under, bound to Rust as
[`EgressScope`](https://docs.rs/contextgraph-types/latest/contextgraph_types/scope/enum.EgressScope.html)
in the new [`DataFlow.egress_scopes`](./protocol-surface.md#handshake--capability) field. Four **normative base classes** form the closed core:

| Scope | Wire string | Off-machine? | Meaning |
| ----- | ----------- | ------------ | ------- |
| `LocalOnly` | `local-only` | no | Nothing leaves the machine. |
| `OrgTenant` | `org-tenant` | yes | Leaves the machine, stays in the org's own infrastructure. |
| `ThirdPartyIndex` | `third-party-index` | yes | Content sent to an external index / embedding service. |
| `ThirdPartyModel` | `third-party-model` | yes | Content sent to an external model API. |

The vocabulary is **extensible** by namespaced custom scopes — `EgressScope::Custom("vendor:scope-name")` — which **MUST** contain a `:` separator with non-empty sides, so a custom scope can never collide with or be mistaken for a base class. Everything other than `local-only` is treated as off-machine, including an unrecognized custom scope: the conservative default is that an unknown destination *leaves*, so a host never under-gates.

**A scope is declared at the provider (data-flow) level, and it governs every frame that provider serves.** There is no per-frame scope: the serving provider's declaration *is* the egress class of each frame it returns. This keeps the consent gate — which fires once, before a query is transmitted — the single place egress is decided, rather than scattering a scope across every frame.

The declaration must be **truthful**: an off-machine scope alongside `egress: false` is a contradiction (a provider claiming local posture while naming a destination that leaves), and a host holds a provider to this at the handshake
([`DataFlow::scopes_consistent`](https://docs.rs/contextgraph-types/latest/contextgraph_types/struct.DataFlow.html#method.scopes_consistent),
requirement C5).

### Consent receipts

When a host grants consent, it records a
[`ConsentReceipt`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentReceipt.html):

```rust
pub struct ConsentReceipt {
    pub provider_id: String,        // the provider this authorizes
    pub scope: EgressScope,         // the exact egress class consented to
    pub provider_name: String,      // provider identity, pinned at grant time
    pub provider_version: String,
    pub grantor: Grantor,           // Human(id) | Policy(id) — who agreed
    pub granted_at: String,         // RFC 3339
    pub expires_at: Option<String>, // RFC 3339, if consent lapses
}
```

A receipt turns "is this allowed?" (a boolean, now) into "what left, to whom, who agreed, and when?" (a durable record). It pins the provider's identity **at grant time**, so a later rename can't retroactively rewrite what was agreed. Receipts live in an **append-only** ledger
([`ConsentStore::record_receipt`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentStore.html#method.record_receipt)):
a new grant never edits or erases an old one, so the history of consent *is* the audit trail, and it is serde-able for durable persistence across host runs.

### Host behavior: reject unconsented egress

A host's pre-query consent gate
([`ConsentStore::evaluate`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentStore.html#method.evaluate))
is scope-aware:

- A provider declaring **off-machine egress scopes** is permitted only when *every* such scope has a recorded receipt. A scope with no matching receipt **MUST** cause the query to be refused with a **typed error** —
  [`HostError::ConsentScopeRequired`](https://docs.rs/contextgraph-host/latest/contextgraph_host/error/enum.HostError.html)
  naming exactly the scopes that would leave unconsented — and the payload **MUST NOT** be transmitted (requirement C6). A budget-style boolean `ConsentRecord` does **not** satisfy a scope gate; only a receipt for that scope does.
- A provider declaring only the boolean `egress` flag (no scopes) keeps the pre-scope legacy gate — a `ConsentRecord` unlocks it. This is what keeps the change additive: an existing provider that never declares scopes behaves exactly as before.

The runtime gate is **presence-based** (does a receipt for the scope exist?) and carries no clock, so it never depends on wall-time to make a decision. **Expiry** is a first-class receipt property
([`ConsentReceipt::is_live`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentReceipt.html#method.is_live)):
a host that enforces expiry consults
[`ConsentStore::live_receipt`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentStore.html#method.live_receipt)
against its own `now`, and treats an expired receipt as absent — re-shutting the gate — while the receipt itself is never pruned from the audit ledger.

### Worked audit scenario: "what left, where, who agreed, when?"

Six months after the fact, an auditor asks whether a customer's repository snippets were ever sent to an external model. The host answers from its persisted consent ledger — no live process required:

```rust
for receipt in store.receipts_for("acme-cloud-model") {
    println!(
        "{scope} — granted by {grantor:?} at {at}{expiry}",
        scope = receipt.scope,                 // third-party-model
        grantor = receipt.grantor,             // Human("ops@oxagen.sh")
        at = receipt.granted_at,               // 2026-01-14T09:02:00Z
        expiry = receipt.expires_at            // Some("2026-07-14T00:00:00Z")
            .map(|e| format!(", expiring {e}"))
            .unwrap_or_default(),
    );
}
```

Every question is answered by a field:

- **What left?** The `scope` (`third-party-model`) — the class of destination — combined with the [usage report](#2-usage-reports)'s `served_frames`, which name the exact frames that provider served.
- **To whom?** The pinned `provider_name` / `provider_version` at grant time.
- **Who agreed?** The `grantor` — a named human or a named policy, not an anonymous "yes".
- **When?** `granted_at`, and `expires_at` if the grant was time-boxed. A query after `expires_at` would have been refused by the gate, so the window of authorized egress is itself on the record.

Because the ledger is append-only, a *revocation* or a lapsed expiry adds to the record rather than erasing it — the audit shows not just the current state but the full history of what was permitted and when.

### Conformance (§3)

| # | Requirement | Enforced / verified by |
| - | ----------- | ---------------------- |
| C5 | A provider **MUST** declare its egress scopes (`egress_scopes`) truthfully and consistently with `data_flow.egress`; an off-machine scope alongside `egress: false`, or a non-namespaced custom scope, is a conformance failure. | `DataFlow::scopes_consistent`; `consent-scope` conformance check |
| C6 | A host **MUST** reject a query to a provider any of whose off-machine egress scopes lacks a recorded consent receipt, with a typed error, before transmitting the payload. | `ConsentStore::evaluate` scope gate; `HostError::ConsentScopeRequired`; host scope-gate test |
