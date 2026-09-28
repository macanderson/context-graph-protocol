# 0015 — Cross-provider ranking is a seam, not a function

**Status:** Accepted (`contextgraph/1.0`; additive in Rust, no wire change).
Amended 2026-09-28 — trust weighting, the reranker hook, and the audited
ranking policy; see [the amendment](#amendment-trust-weights-a-reranker-and-an-audited-policy).

## Context

`SPEC.md` §6.6 (F10) says `score` is **provider-local and ordinal**. Nothing in
the protocol defines a shared scale, and §6.6 explains at length why nothing
could: a calibration rule this specification cannot verify is worse than an
absent one. So one provider's `0.8` and another's are not the same claim.

The reference host ranked mixed frame sets by raw `score` anyway.
`order_by_value` sorted the union score-descending, and `compose_for_prompt`
packed the budget in that same order. F10 permits this — a prompt is a
sequence, so *some* total order is required, and §6.6 names "ranking by raw
`score` and saying so" as a legitimate host policy. It was documented as a
policy choice in the function's own doc comment and in §6.6.

Documented is not the same as answered. The cost is concrete and it lands on
the *evidence set*, not just the ordering: a semantic-search provider reporting
cosine similarity in the `0.8`–`0.95` band and a lexical provider reporting a
normalized BM25 rank near `0.4` are both honest, and under a tight budget the
lexical provider's frames are ranked below every one of the semantic
provider's and never reach the prompt at all. Which sources a host cites was
decided by an implementation detail of somebody else's retriever. That is the
unaccountable behaviour this protocol exists to remove, one layer up from where
§6.6 removes it.

`fold_to_edges` was made public so a host could rank frames itself and reuse
the Lost-in-the-Middle placement. Nothing in the repository demonstrated a
ranking to put in front of it, so the public function was a hook with no
worked example, and the *default* stayed the only thing anyone would actually
run (issue #95).

## Decision

Cross-provider ranking becomes a **seam** — `RankingStrategy` in
`contextgraph-host/src/compose/ranking.rs` — with three implementations, and
the ranking is applied where it decides which frames survive the budget.

### A seam rather than a better `order_by_value`

Three reasons, in order of weight.

1. **There is no ranking that is right for every host.** F10 lists four
   plausible policies precisely because the choice depends on facts the
   protocol cannot see: how many providers a host runs, whether it trusts them
   equally, whether it has a reranker of its own. Replacing one default with a
   different default would move the arbitrary choice, not remove it.
2. **Replacing the default is a behaviour change for every existing host.** A
   host that upgrades this crate and finds its prompts reordered has been
   handed a silent regression in the one thing composition promises — a
   byte-stable prefix and a reproducible evidence set.
3. **The four candidates are not one function.** A trust weighting needs
   configuration, a reranker needs I/O, quotas and interleaving need neither.
   Only a trait accommodates all of them, and only a trait lets a host that is
   not in this repository supply the fourth.

### `ScoreDescending` stays the default

The case for changing it is real: it is the strategy with the known
pathology, and a reference implementation that ships a default it can describe
the failure mode of is inviting every host to inherit that failure mode.

It stays anyway, for two reasons.

- **With one provider it is simply correct**, and there is no cross-provider
  question to get wrong. Within a provider the score ordering *is* meaningful —
  that is the premise F10 rests on — so for a single-provider host, which is
  the common case today, score-descending is not a compromise. Every other
  strategy here degenerates to it in that case, which is the test
  `with_one_provider_every_strategy_agrees_with_raw_score`.
- **A default that changes under a host is worse than a default that is
  wrong in a documented way.** The pathology is now demonstrated by a test
  anyone can read, and switching costs one argument. A host that never chose a
  ranking policy gets exactly the bytes it got before; a host that thinks about
  it has two policies to pick from and a trait to write a third.

The honest summary: this is a defaults-are-sticky argument, not a
this-is-the-best-ranking argument. If the reference host ever fans out to
several providers by default, that changes and the default should be revisited
with it.

### Two concrete strategies, and why these two

- **`RoundRobinByRank`** — every provider's best frame, then every provider's
  second. Uses only within-provider rank, the one comparison F10 endorses.
- **`PerProviderQuota { k }`** — the same in blocks of `k`, so a provider's
  evidence stays contiguous in the prompt.

Neither needs configuration, which is what makes them testable without
inventing a trust model. Trust weighting and a reranker hook are both
expressible as `RankingStrategy` implementations and are deliberately left to
the host that has the trust model or the reranker — this repository has
neither, and a made-up weighting shipped as a reference would be a worse
artifact than none.

Two rather than one is the point: a seam with a single implementation is an
interface bolted to a function, and nothing proves it can carry another.

### The strategy ranks *before* the budget pack

`compose_for_prompt_with` hands the strategy's order to the budget packer, so
the policy decides which frames are included, not only where they sit. Ranking
only the already-included set would have left the starvation exactly where it
was and produced a feature that reorders a prompt without changing what is in
it.

Placement stays separate: the packer walks the ranking, and `fold_to_edges`
deals the survivors to the attention-favoured edges. `order_by_value` is now
`order_by(&ScoreDescending, …)`, so there is one copy of the score ranking
rather than two.

### A strategy ranks; it never filters

`RankingStrategy::order` returns a permutation of the input indices. It cannot
drop a frame, because dropping is the packer's job and the packer records an
`ExclusionReason` for the audit — a strategy that discarded evidence would put
frames into neither the prompt nor the record of why they are absent, breaking
the total-partition guarantee `CompositionAudit` exists for.

`rank_with` enforces this against third-party strategies rather than trusting
the doc comment: an out-of-range or repeated index is skipped, and anything
left unplaced is appended in canonical order. It repairs rather than panicking,
because a library that aborted a host's turn over a ranking bug would be a
worse outcome than the one it caught. `is_ranking_permutation` is exported so a
strategy's own tests can assert the contract, which is where a panic belongs.

### Ties are broken so the order is total and reproducible

Two runs over the same frames must produce the same order, or a host's prompt
stops being reproducible and its provider prompt cache stops hitting. Every
strategy here resolves to a total order through the same ladder:

1. the strategy's own key — score for `ScoreDescending`, within-provider rank
   (and tier) for the other two;
2. **provider ordinal**, taken from the set's provider ids sorted
   lexicographically;
3. the canonical `FrameId` (`provider id`, `frame id`, `content digest`).

Providers are ordinalized through a `BTreeMap`. A `HashMap` would have given a
different answer per run and is the specific way this goes wrong; the
determinism tests run the same input twice and compare, including a
twelve-provider case.

Step 2 is arbitrary, and admitting that is the point. Once two frames sit in
the same tier the protocol offers nothing that ranks one provider above
another, and reaching for `score` there would be exactly the cross-provider
comparison F10 says is not a measurement. Alphabetical order is visibly a coin
toss; a score comparison would dress one up as a judgement.

## Consequences

- Additive in Rust and invisible on the wire: `compose_for_prompt`,
  `order_by_value` and `fold_to_edges` keep their signatures and their output.
  The existing composition tests pass unchanged, which is the evidence that the
  default path did not move.
- A host outside this crate can supply its own ranking through the public API.
  The tests in `contextgraph-host/tests/cross_provider_ranking.rs` are written
  through that API for exactly that reason.
- The budget bound is unchanged and re-proved per strategy: ranking decides
  order, the packer decides what fits, and a strategy cannot select a set that
  exceeds the budget.
- `PerProviderQuota` tiers rather than truncates. A provider's `k + 1`th frame
  is ranked lower, never dropped — truncation is the packer's decision because
  only the packer records a reason.
- Trust weighting and a reranker hook remain unbuilt, by choice. Their shape is
  now fixed by the trait, so adding one is a new type, not a redesign.
  *(Superseded by the 2026-09-28 amendment below: both are now built, as new
  types, with no change to the trait.)*

## Amendment: trust weights, a reranker, and an audited policy

*2026-09-28, [#115](https://github.com/macanderson/context-graph-protocol/issues/115)
and [#116](https://github.com/macanderson/context-graph-protocol/issues/116).*

The decision above left two of issue #95's four candidate policies unbuilt and
left the composition audit silent on which policy ran. Both gaps meant a host
had nothing in the repository to copy — exactly the state #95 described for
ranking as a whole. This amendment closes them, and records three decisions.

### 1. A trust weight scales allocation, never score

The original objection to shipping trust weights was that this repository has
no trust model, and inventing one would be a worse artifact than none. That
objection holds against shipping *weights*; it does not hold against shipping
the *mechanism* a host's weights plug into. `TrustWeighted` ships the
mechanism and no numbers: every provider is weight `1` until the host
configures otherwise, which makes an unconfigured `TrustWeighted` exactly
`RoundRobinByRank`.

What a weight multiplies was the real decision. Two readings were on the
table:

- **Weight × score.** Rejected. A weighted score is still a score: it
  inherits every scale mismatch between two providers' retrievers and
  multiplies it by a number the host chose, then ranks the union by the
  product. That is F10's forbidden cross-provider comparison with an extra
  factor, and it would reintroduce the starvation this ADR exists to remove —
  a generous scorer with weight `1` still buries a conservative one with
  weight `2` when its numbers are three times larger.
- **Weight × allocation.** Chosen. A provider of weight `w` is dealt `w`
  frames per round: a frame at within-provider rank `r` sits in round
  `r / w`, rounds go in order, and inside a round providers take the same
  arbitrary-but-stable provider-id order every strategy here uses. Uniform
  weight `k` is exactly `PerProviderQuota::new(k)`. The only score
  comparisons are within one provider; a test rescales one provider's scores
  by a factor of 18 and asserts the ranking does not move.

Weight `0` means **last resort**, not excluded: a strategy ranks and never
filters, so a zero-weight provider's frames rank after every positively
weighted provider's, and a budget that runs out first records them as
`OverBudget`. Distrust stays visible in the audit rather than becoming a
silent drop.

`policy_name` is the stable `"trust-weighted"`, not a rendering of the weight
table: the audit names the rule, and a host that must reproduce a composition
records its weight table (`TrustWeighted::weights`, in deterministic order)
beside the audit.

### 2. A reranker runs before composition (option 1)

`RankingStrategy::order` is synchronous and pure. A reranker does I/O. Issue
#115 laid out three options:

1. the host reranks before calling `compose_for_prompt_with` and passes a
   strategy that reads a precomputed order;
2. a second, async `AsyncRankingStrategy` with an async composition entry
   point;
3. option 1 as documentation only, with a worked example and no type.

**Option 1, with a type.** The async trait is rejected because composition's
value is that it is a pure function of its inputs: the same frame set composes
to the same bytes, which is what keeps a host's prompt reproducible and its
provider prompt cache hitting. An async strategy makes the composition path
depend on whatever the strategy awaits, puts an executor and `async-trait` on
a path that has neither, and still cannot make a remote model deterministic.
The host already awaits I/O in its fan-out; the reranker belongs beside it.
This crate's job is placement and accounting, not inference.

Option 3 was close. It is rejected because the precomputed-order strategy is
not trivial to write correctly, and the incorrect versions are the obvious
ones:

- **Keyed by index, it breaks.** Composition de-duplicates *before* it ranks,
  so the slice a strategy sees is not the slice the host reranked, and every
  index after a dropped duplicate points at the wrong frame. `PrecomputedOrder`
  is keyed by `FrameId`, which names the same evidence on both sides of dedup;
  a test reranks a set containing a cross-provider duplicate and asserts the
  order lands exactly.
- **A partial verdict must not lose evidence.** A reranker that timed out
  partway names some frames and not others. Unnamed frames rank after every
  named one, in `RoundRobinByRank` order — so the fallback itself makes no
  cross-provider score comparison — and a repeated or unknown identity is
  tolerated rather than trusted.

A host copying a thirty-line type out of an example would get one of those
wrong. Shipping it costs one small type and no trait change.

`contextgraph-host/examples/rerank_before_compose.rs` runs the whole flow
offline: a fan-out's frames, raw score starving the provider that holds the
answer, an awaited stand-in reranker, `PrecomputedOrder`, and the audit
naming the reranker. CI runs it, so it stays runnable rather than illustrative.

### 3. The audit records the ranking policy (#116)

`CompositionAudit` gains `ranking_policy: String`, filled from the strategy's
`policy_name()`. F10 requires a host that orders frames across providers to
document the choice; once the choice is made per call, the record of the call
is the only place that documentation is reliably true. The strategy decides
which frames survive the budget, so an audit without it explained every drop
but not the rule that caused it.

A `String`, not `&'static str`, because a host's strategy may name itself at
runtime — `PrecomputedOrder` carries its reranker's model name.

Adding a public field to a public struct is a Rust-semver break for any
downstream struct literal or exhaustive destructure. The crate's `[Unreleased]`
section already carries breaking changes, so the cost is taken now rather than
at a later major. `CompositionAudit` is marked `#[non_exhaustive]` in the same
change: the audit is built by this crate's composition entry points and only
read by hosts, so a downstream literal was never a use worth protecting, and
the next field the record needs lands without another break. Nothing in this
workspace constructs one outside the crate — the host-conformance check reads
the audit and builds none.

### Consequences of the amendment

- Five strategies ship. The trait is unchanged; both additions are new types,
  as the original consequences predicted.
- `CompositionAudit` gains a field and becomes `#[non_exhaustive]` — a
  Rust-semver break, named as such in `CHANGELOG.md`. No wire change: the audit
  is a host-side record, not a protocol message.
- A reranker remains outside this crate by design. A host that wants one
  awaits it and passes its verdict; a future proposal for an async strategy
  has to answer why composition should stop being a pure function.
