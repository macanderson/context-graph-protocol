//! Cross-provider ranking policy (`SPEC.md` §6.6, F10).
//!
//! `score` is **provider-local and ordinal**: it orders one provider's frames
//! against one query, and nothing in the protocol makes two providers' numbers
//! commensurable. A host still has to put the frames in *some* order — a prompt
//! is a sequence, and a budget is spent front-first — so every host is making a
//! cross-provider ranking decision whether or not it admits to one. F10's rule
//! is that the decision is the host's policy, named as such.
//!
//! [`RankingStrategy`] is where that policy lives. [`super::compose_for_prompt`]
//! keeps ranking by raw `score` ([`ScoreDescending`]) so an existing host's
//! output does not move; [`super::compose_for_prompt_with`] takes any strategy,
//! and two that need no configuration ship here:
//!
//! - [`RoundRobinByRank`] — every provider's best frame, then every provider's
//!   second, and so on. Uses only within-provider rank, where the ordering is
//!   meaningful.
//! - [`PerProviderQuota`] — the same idea in blocks of `k`: each provider's top
//!   `k`, then each provider's next `k`.
//!
//! # Why raw score starves a provider
//!
//! Consider a semantic-search provider that reports cosine similarity in the
//! `0.8`–`0.95` band and a lexical provider that reports a normalized BM25 rank
//! topping out near `0.4`. Both are honest about their own frames. Rank the
//! union by raw `score` under a budget that fits four frames and the lexical
//! provider contributes nothing — not because its evidence is worse, but
//! because its retriever's number is smaller. The prompt's evidence set was
//! decided by an implementation detail of someone else's scorer.
//! `starvation_*` in this module's tests is that scenario, run.
//!
//! # Determinism
//!
//! Ranking must be a pure function of the input *set*: two runs over the same
//! frames must produce the same order, or a host's prompt stops being
//! reproducible and its provider prompt cache stops hitting (`super`'s module
//! docs). Every strategy here derives its provider ordering from a
//! [`BTreeMap`] and breaks every remaining tie on the canonical
//! [`FrameId`], so no ordering ever depends on
//! hash iteration or arrival order.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use contextgraph_types::{ContextFrame, FrameId};

/// A host's policy for ordering frames drawn from more than one provider —
/// the choice `SPEC.md` §6.6 (F10) says a host owns and must name.
///
/// A strategy reports a **permutation of the input indices, best first**. It
/// ranks; it never filters. Dropping evidence is the budget packer's job in
/// [`super::compose_for_prompt_with`], which excludes a frame with a recorded
/// [`ExclusionReason`](super::ExclusionReason) so the audit still explains
/// every drop. A strategy that quietly returned fewer indices would put frames
/// into neither the prompt nor the audit.
///
/// # Contract
///
/// - `order(frames)` returns each index in `0..frames.len()` exactly once.
/// - The result depends only on `frames` as a set — not on their arrival
///   order, and not on anything that varies between two runs over the same
///   input. A `HashMap` iteration is the usual way to get this wrong.
///
/// [`rank_with`] holds the first half rather than trusting it: an index out of
/// range or repeated is skipped, and any frame a strategy failed to place is
/// appended in canonical order, so a third-party strategy cannot make the
/// reference host lose evidence. It repairs rather than panicking — a library
/// that aborted a host's turn over a ranking bug would be a worse failure than
/// the one it caught. [`is_ranking_permutation`] is the assertion to put in a
/// strategy's own tests.
pub trait RankingStrategy {
    /// The policy's name, for a host that has to state which cross-provider
    /// ordering it applied. F10 requires a host to document the choice; a
    /// strategy that cannot say what it is makes that impossible, which is why
    /// this has no default implementation.
    fn policy_name(&self) -> &str;

    /// Order the input indices best-first. See the trait contract.
    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize>;
}

impl<T: RankingStrategy + ?Sized> RankingStrategy for &T {
    fn policy_name(&self) -> &str {
        (**self).policy_name()
    }

    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize> {
        (**self).order(frames)
    }
}

/// Apply a [`RankingStrategy`], returning the frames best-first.
///
/// The strategy's permutation is checked, not trusted: an out-of-range or
/// repeated index is skipped and anything the strategy left unplaced is
/// appended in canonical order (`score` descending, `FrameId` ascending). The
/// output is therefore always a permutation of the input, whoever wrote the
/// strategy — which is what keeps the composition audit a total partition of
/// the evidence the host offered.
pub fn rank_with<S: RankingStrategy + ?Sized>(
    strategy: &S,
    frames: Vec<(String, ContextFrame)>,
) -> Vec<(String, ContextFrame)> {
    let n = frames.len();
    let proposed = strategy.order(&frames);
    let mut placed = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    for index in proposed {
        if index < n && !placed[index] {
            placed[index] = true;
            order.push(index);
        }
    }
    if order.len() < n {
        // Repair, deterministically: whatever the strategy failed to place
        // follows in canonical order rather than vanishing.
        let mut missing: Vec<usize> = (0..n).filter(|index| !placed[*index]).collect();
        missing.sort_by(|&a, &b| by_score_desc(&frames[a], &frames[b]));
        order.extend(missing);
    }

    let mut slots: Vec<Option<(String, ContextFrame)>> = frames.into_iter().map(Some).collect();
    order
        .into_iter()
        .map(|index| slots[index].take().expect("each index placed once"))
        .collect()
}

/// Whether `order` is exactly the indices `0..n`, each once — the
/// [`RankingStrategy`] contract, as an assertion a strategy's own tests can
/// make.
///
/// [`rank_with`] repairs a violation rather than calling this, because a host
/// mid-turn needs its evidence more than it needs a panic. That leaves a
/// strategy author with nothing to fail on, which is what this is for.
pub fn is_ranking_permutation(order: &[usize], n: usize) -> bool {
    if order.len() != n {
        return false;
    }
    let mut seen = vec![false; n];
    for &index in order {
        match seen.get_mut(index) {
            Some(slot) if !*slot => *slot = true,
            _ => return false,
        }
    }
    true
}

/// The canonical within-provider ordering: `score` descending, canonical
/// [`FrameId`] ascending as the tiebreak.
///
/// Comparing `score` here is comparing two frames **from one provider**, which
/// is the only comparison F10 says means anything. Every strategy in this
/// module uses it for exactly that, and never to rank one provider's frame
/// against another's.
fn by_score_desc(a: &(String, ContextFrame), b: &(String, ContextFrame)) -> Ordering {
    b.1.score
        .total_cmp(&a.1.score)
        .then_with(|| a.1.identity(&a.0).cmp(&b.1.identity(&b.0)))
}

/// One frame's position in the lane structure every interleaving strategy
/// ranks over: which provider it came from, and how good it is *within that
/// provider*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lane {
    /// The provider's ordinal in the set's providers sorted by id — an
    /// arbitrary but stable order. See [`provider_lanes`].
    provider: usize,
    /// `0` for this provider's best frame, `1` for its second, and so on.
    rank: usize,
}

/// Each frame's [`Lane`].
///
/// Providers are ordinalized through a [`BTreeMap`], so the traversal is by
/// provider id and never by hash order — the determinism this module's docs
/// promise. Ranks come from [`by_score_desc`] applied inside one provider,
/// which is the only place F10 says a score comparison means anything.
///
/// The provider ordinal is **arbitrary but stable**, and deliberately so: once
/// two frames sit in the same tier, the protocol offers nothing that ranks one
/// provider above another, and reaching for `score` there would be exactly the
/// cross-provider comparison F10 says is not a measurement. An alphabetical
/// order admits it is a coin toss; a score comparison would dress one up as a
/// judgement.
fn provider_lanes(frames: &[(String, ContextFrame)]) -> Vec<Lane> {
    let mut by_provider: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, (provider_id, _)) in frames.iter().enumerate() {
        by_provider
            .entry(provider_id.as_str())
            .or_default()
            .push(index);
    }
    let mut lanes = vec![
        Lane {
            provider: 0,
            rank: 0
        };
        frames.len()
    ];
    for (provider, indices) in by_provider.into_values().enumerate() {
        let mut indices = indices;
        indices.sort_by(|&a, &b| by_score_desc(&frames[a], &frames[b]));
        for (rank, index) in indices.into_iter().enumerate() {
            lanes[index] = Lane { provider, rank };
        }
    }
    lanes
}

/// Sort `0..frames.len()` by a per-frame key, canonical `FrameId` ascending as
/// the standing final tiebreak so the order is total and reproducible.
///
/// The keys are computed once rather than inside the comparator, so a strategy
/// pays for one `identity()` per frame instead of one per comparison.
fn order_by_key<K: Ord>(frames: &[(String, ContextFrame)], key: impl Fn(usize) -> K) -> Vec<usize> {
    let mut keyed: Vec<(K, FrameId, usize)> = (0..frames.len())
        .map(|index| {
            (
                key(index),
                frames[index].1.identity(&frames[index].0),
                index,
            )
        })
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, _, index)| index).collect()
}

/// Rank the whole mixed set by raw `score`, descending — the reference host's
/// documented default (`SPEC.md` §6.6), and what
/// [`super::order_by_value`] has always done.
///
/// It is a real policy with a real cost: the provider that scores most
/// generously wins the top of the prompt and the front of the budget, whatever
/// its evidence is worth. It stays the default because it is the only strategy
/// here that changes nothing for a host that never asked for a ranking policy,
/// and because with a single provider it is simply the right answer — see
/// [ADR 0015](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0015-cross-provider-ranking-strategies.md).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScoreDescending;

impl RankingStrategy for ScoreDescending {
    fn policy_name(&self) -> &str {
        "score-descending"
    }

    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..frames.len()).collect();
        order.sort_by(|&a, &b| by_score_desc(&frames[a], &frames[b]));
        order
    }
}

/// Interleave providers by **within-provider rank**: every provider's best
/// frame first, then every provider's second-best, and so on.
///
/// The only score comparisons are within one provider, where F10 says the
/// ordering is meaningful. Across providers the order is by provider id, which
/// is arbitrary and stable rather than a claim that one source outranks
/// another.
///
/// With one provider the ranks are `0, 1, 2, …` in score order, so this is
/// identical to [`ScoreDescending`] — the cross-provider question does not
/// arise, and no strategy here invents one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoundRobinByRank;

impl RankingStrategy for RoundRobinByRank {
    fn policy_name(&self) -> &str {
        "round-robin-by-rank"
    }

    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize> {
        let lanes = provider_lanes(frames);
        // Rank first, provider second: every provider's best frame, then every
        // provider's second.
        order_by_key(frames, |index| (lanes[index].rank, lanes[index].provider))
    }
}

/// Give every provider its top `k` before any provider gets its `k + 1`th:
/// each provider's best `k` frames, then each provider's next `k`, and so on.
///
/// The difference from [`RoundRobinByRank`] is contiguity. A round robin deals
/// one frame per provider per turn; a quota deals `k` at a time, so a
/// provider's block of evidence stays together in the prompt. `k = 1` is a
/// round robin.
///
/// The quota **tiers, it does not truncate.** A provider's `k + 1`th frame is
/// ranked lower, never dropped: dropping is the budget packer's decision, and
/// it records a reason for the audit. A strategy that discarded frames would
/// leave them out of both the prompt and the record of why.
///
/// With one provider every frame sits in its own tier position in score order,
/// so this too degenerates to [`ScoreDescending`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerProviderQuota {
    per_provider: NonZeroUsize,
}

impl PerProviderQuota {
    /// A quota of `per_provider` frames per provider per tier. `0` is read as
    /// `1`: a quota of nothing would rank every frame into one tier and mean
    /// nothing, and returning an error for a number a caller can only have
    /// meant as "one at a time" buys the caller nothing.
    pub fn new(per_provider: usize) -> Self {
        Self {
            per_provider: NonZeroUsize::new(per_provider).unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// The frames each provider contributes per tier.
    pub fn per_provider(&self) -> usize {
        self.per_provider.get()
    }
}

impl Default for PerProviderQuota {
    /// Three frames per provider per tier — enough that a provider's evidence
    /// arrives as a block rather than a single frame, small enough that a
    /// second provider is reached inside any realistic budget. A host with a
    /// measured number should pass it to [`PerProviderQuota::new`].
    fn default() -> Self {
        Self::new(3)
    }
}

impl RankingStrategy for PerProviderQuota {
    fn policy_name(&self) -> &str {
        "per-provider-quota"
    }

    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize> {
        let lanes = provider_lanes(frames);
        let k = self.per_provider.get();
        // Tier first, then provider, then rank within the provider — so one
        // provider's `k` frames stay contiguous inside the tier, which is the
        // whole difference from a round robin.
        order_by_key(frames, |index| {
            (
                lanes[index].rank / k,
                lanes[index].provider,
                lanes[index].rank,
            )
        })
    }
}
