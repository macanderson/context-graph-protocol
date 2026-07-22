# ADR 0003 — Canonical token accounting

- **Status:** accepted
- **Date:** 2026-07-21
- **Issue:** [#8](https://github.com/macanderson/context-graph-protocol/issues/8)
- **Normative:** yes — tightens a conformance requirement (B1) and adds a new
  one (B3).

## Context

Budget honesty is CGP's flagship guarantee, enforced as
`sum(token_cost) <= max_tokens`. Nothing in the protocol defined **what a token
is**. Three consequences, in ascending order of seriousness:

1. A provider counting `cl100k` tokens, one counting words, and one counting
   bytes÷4 return mutually incomparable numbers, and all three pass conformance.
2. A `BudgetLie` verdict — the protocol's loudest accusation — may in fact be a
   tokenizer disagreement rather than dishonesty.
3. **The check verified arithmetic, not truth.** A provider reporting
   `token_cost: 1` on a ten-thousand-token frame satisfied
   `sum(token_cost) <= max_tokens` perfectly while destroying the host's actual
   budget. The suite could not catch the one lie that matters.

## Options considered

**A. A normative canonical counting rule** — define a budget token as a
model-independent function of content bytes. Deterministic, recomputable in any
language with no dependencies, and therefore checkable by conformance.

**B. Tokenizer negotiation at handshake** — the host names a tokenizer id, the
provider echoes or refuses. Exact, but drags a tokenizer implementation into
every provider SDK in every language, and into the conformance suite too.

**C. Status quo plus host-side recount** — treat `token_cost` as advisory. Honest
bookkeeping, but it demotes the flagship guarantee from a contract to a hint.

## Decision

**Option A**, with **exact equality** and zero tolerance.

```text
budget_tokens(content) = ceil(utf8_byte_length(content) / 4)
```

`ContextFrame.token_cost` **MUST** equal `budget_tokens(frame.content)`.
Conformance recomputes the value from the received bytes and fails any
provider whose declaration differs.

### Why exact equality rather than a tolerance band

A tolerance band re-opens the hole it was meant to close: any band wide enough
to accommodate genuine tokenizer disagreement is also wide enough to hide
meaningful under-reporting, and the suite is back to guessing. Exact equality
makes the check binary, language-independent, and impossible to argue with. A
provider cannot "disagree" with a byte count.

### Why a byte-based unit is the right unit, given that it is not a real tokenizer

It is not trying to be one. `budget_tokens` is an **accounting unit**, not a
prediction of any model's tokenizer. Its job is to make every provider's cost
claims *comparable and verifiable*, which no real tokenizer can do without
being mandated everywhere.

The approximation is honest about its direction: at roughly four bytes per
token it tracks English prose closely, and it **under-estimates** dense source
code (≈3–3.5 bytes/token) and CJK text (≈3 bytes/token). A host therefore
**MUST** map its real model budget into budget tokens with a safety factor
rather than treating one budget token as one model token. `SPEC.md` states this
explicitly so no host mistakes the unit for a model token; the reference host
applies a documented default factor.

### Scope of the count

`budget_tokens` covers `frame.content` **only**. Not `title`, not
`citation_label`, not provenance, not the host's fences and labels.

This is deliberate. `content` is the one field the provider fully controls and
the one whose byte string both sides observe identically, so it is the only
input on which a byte-exact check can be built. The host's own rendering chrome
— labels, citation lines, delimiters — is *the host's* cost to budget, and is
accounted for by the composition layer (issue #15), not charged to the
provider. Splitting it this way keeps each party responsible for exactly the
bytes it chooses.

### Relationship to option B

Exact tokenizer agreement remains available as a **1.x additive refinement**: an
optional handshake field naming a tokenizer, plus an optional per-frame exact
count. That path is deliberately left open, and it does not disturb the floor
established here — the canonical count stays the unit conformance checks.

## Consequences

- `contextgraph-types` gains a `token` module exposing `budget_tokens`, so every
  Rust provider gets the rule for free and the SDKs (issue #17) have one
  function to port.
- The `budget-honesty` conformance check recomputes counts from content instead
  of summing the provider's claims. **This will fail providers that were
  previously green**, which is the point; it is called out in `CHANGELOG.md` and
  in `MIGRATION.md`.
- A new `--misbehave` mode under-reports `token_cost` and must be caught.
- `docs/implementing-a-provider.md` shows the rule in its worked example.

## New conformance requirement

| # | Requirement |
| - | ----------- |
| B3 | Every frame's `token_cost` **MUST** equal `ceil(utf8_byte_length(content) / 4)`. |

B1 (the sum stays within `max_tokens`) is unchanged but becomes meaningful,
because B3 now anchors the summands to observable bytes.
