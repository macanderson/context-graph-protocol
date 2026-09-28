//! Cross-provider ranking policy, driven through the public host API only
//! (`SPEC.md` §6.6, F10; issues #95, #115, #116).
//!
//! An integration test rather than a unit one, because the thing under test is
//! precisely that a host *outside* this crate can bring its own cross-provider
//! ranking. Everything here is reachable from `contextgraph_host`'s root.

use contextgraph_host::{
    ComposedPrompt, ExclusionReason, FrameDisposition, PerProviderQuota, PrecomputedOrder,
    RankingStrategy, RoundRobinByRank, ScoreDescending, TrustWeighted, compose_for_prompt,
    compose_for_prompt_with, is_ranking_permutation, rank_with, rendered_token_cost,
};
use contextgraph_types::{ContextFrame, FrameId, FrameKind, budget_tokens};

/// A frame whose `token_cost` is the canonical cost of its content, with a
/// digest unique to `(provider, id)` so no two test frames are ever taken
/// for the same evidence by cross-provider dedup.
fn mk(provider: &str, id: &str, score: f32, content: &str) -> (String, ContextFrame) {
    let mut frame = ContextFrame::full(
        id,
        FrameKind::Doc,
        format!("{id} title"),
        content,
        score,
        budget_tokens(content),
    );
    frame.content_digest = Some(format!("sha256:{provider}-{id}"));
    frame.citation_label = Some(format!("{id} cite"));
    (provider.to_string(), frame)
}

fn ids<S: RankingStrategy + ?Sized>(
    strategy: &S,
    frames: Vec<(String, ContextFrame)>,
) -> Vec<String> {
    rank_with(strategy, frames)
        .into_iter()
        .map(|(_, frame)| frame.id)
        .collect()
}

/// A generous scorer and a conservative scorer over the same evidence: the
/// exact shape F10 describes. `lex` is not worse, it just reports smaller
/// numbers.
///
/// Every frame's content is four bytes, so each costs exactly one budget
/// token (`contextgraph_types::budget_tokens`) and a budget of four fits
/// four frames — whichever four the ranking policy puts first.
fn generous_and_conservative() -> Vec<(String, ContextFrame)> {
    vec![
        mk("sem", "sem-1", 0.95, "sem1"),
        mk("sem", "sem-2", 0.92, "sem2"),
        mk("sem", "sem-3", 0.89, "sem3"),
        mk("sem", "sem-4", 0.86, "sem4"),
        mk("lex", "lex-1", 0.40, "lex1"),
        mk("lex", "lex-2", 0.31, "lex2"),
        mk("lex", "lex-3", 0.22, "lex3"),
    ]
}

// ---- the problem F10 describes, and the strategies that answer it ----

#[test]
fn starvation_raw_score_gives_the_conservative_provider_nothing() {
    // The control. Ranking the union by raw score puts all four generous
    // frames ahead of every conservative one, so a budget that fits four
    // frames buys four frames from one provider.
    let ranked = ids(&ScoreDescending, generous_and_conservative());
    assert_eq!(
        &ranked[..4],
        &["sem-1", "sem-2", "sem-3", "sem-4"],
        "raw score should rank the generous provider's whole set first"
    );
}

#[test]
fn starvation_round_robin_seats_the_conservative_provider_immediately() {
    let ranked = ids(&RoundRobinByRank, generous_and_conservative());
    // Tier 0 is each provider's best frame; `lex` sorts before `sem`, and
    // that ordering is the arbitrary-but-stable provider tiebreak.
    assert_eq!(ranked[0], "lex-1");
    assert_eq!(ranked[1], "sem-1");
    assert_eq!(ranked[2], "lex-2");
    assert_eq!(ranked[3], "sem-2");
    // Within a provider the score order is preserved — the premise of the
    // whole change is that within-provider rank *is* meaningful.
    let sem: Vec<&String> = ranked.iter().filter(|id| id.starts_with("sem")).collect();
    assert_eq!(sem, ["sem-1", "sem-2", "sem-3", "sem-4"]);
    let lex: Vec<&String> = ranked.iter().filter(|id| id.starts_with("lex")).collect();
    assert_eq!(lex, ["lex-1", "lex-2", "lex-3"]);
}

#[test]
fn starvation_a_quota_seats_both_providers_as_contiguous_blocks() {
    let ranked = ids(&PerProviderQuota::new(2), generous_and_conservative());
    // Tier 0: `lex`'s top two, then `sem`'s top two.
    assert_eq!(&ranked[..4], &["lex-1", "lex-2", "sem-1", "sem-2"]);
    // Tier 1: what is left of each, same provider order.
    assert_eq!(&ranked[4..], &["lex-3", "sem-3", "sem-4"]);
}

#[test]
fn starvation_shows_up_in_the_composed_prompt_under_a_real_budget() {
    // The argument for the whole change, at the level a host sees it: one
    // budget, one frame set, two policies, and the conservative provider
    // is either cited or it is not.
    //
    // Four frames fit, so `score` ordering spends the entire budget on `sem`.
    // The budget is derived from what the packer actually charges — the whole
    // rendered block, chrome included — rather than from the frames' content
    // cost, which is only part of it. Every frame here renders to the same size,
    // so four of anything is four of everything.
    let budget: u32 = 4 * rendered_token_cost("sem", &generous_and_conservative()[0].1);
    let providers_cited = |composed: &ComposedPrompt| -> Vec<String> {
        let mut seen: Vec<String> = composed
            .citations
            .iter()
            .map(|c| c.frame.provider_id.clone())
            .collect();
        seen.sort();
        seen.dedup();
        seen
    };

    let frames = generous_and_conservative();
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();

    let by_score = compose_for_prompt_with(borrowed.iter().copied(), budget, &ScoreDescending);
    assert_eq!(
        providers_cited(&by_score),
        vec!["sem".to_string()],
        "raw score starves the conservative provider out of the prompt entirely"
    );

    let interleaved = compose_for_prompt_with(borrowed.iter().copied(), budget, &RoundRobinByRank);
    assert_eq!(
        providers_cited(&interleaved),
        vec!["lex".to_string(), "sem".to_string()],
        "the interleave seats both providers under the same budget"
    );

    let quota =
        compose_for_prompt_with(borrowed.iter().copied(), budget, &PerProviderQuota::new(2));
    assert_eq!(
        providers_cited(&quota),
        vec!["lex".to_string(), "sem".to_string()],
        "so does the quota"
    );
}

// ---- determinism ----

#[test]
fn every_strategy_is_a_pure_function_of_the_set() {
    // Two runs over the same frames must produce the same order, or a
    // host's prompt stops being reproducible. Arrival order must not
    // matter either: the same set shuffled ranks identically.
    let trust = lex_trusted();
    let reranked = reranked_lex_first();
    let strategies: [&dyn RankingStrategy; 5] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(2),
        &trust,
        &reranked,
    ];
    for strategy in strategies {
        let first = ids(strategy, generous_and_conservative());
        let second = ids(strategy, generous_and_conservative());
        assert_eq!(
            first,
            second,
            "{}: two runs over the same set must agree",
            strategy.policy_name()
        );

        let mut shuffled = generous_and_conservative();
        shuffled.reverse();
        shuffled.swap(0, 3);
        assert_eq!(
            ids(strategy, shuffled),
            first,
            "{}: arrival order must not change the ranking",
            strategy.policy_name()
        );
    }
}

#[test]
fn a_composed_prompt_is_byte_identical_across_two_runs_of_the_same_strategy() {
    // The determinism that actually matters to a host: identical prompt
    // bytes, so the provider prompt cache still hits.
    let frames = generous_and_conservative();
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let trust = lex_trusted();
    let reranked = reranked_lex_first();
    let strategies: [&dyn RankingStrategy; 5] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(3),
        &trust,
        &reranked,
    ];
    for strategy in strategies {
        let first = compose_for_prompt_with(borrowed.iter().copied(), 1000, strategy);
        let second = compose_for_prompt_with(borrowed.iter().copied(), 1000, strategy);
        assert_eq!(
            first.prompt,
            second.prompt,
            "{}: the same set must compose to the same bytes",
            strategy.policy_name()
        );
        assert_eq!(first.audit, second.audit);
    }
}

#[test]
fn many_providers_do_not_perturb_the_ordering_between_two_runs() {
    // Enough providers that a hash-ordered grouping would differ between
    // runs within a single process (a `HashMap`'s per-process random seed
    // is fixed, so this catches the ordering being unstable *at all*
    // rather than only across processes; `within_provider_ranks` uses a
    // BTreeMap so neither can happen).
    let build = || -> Vec<(String, ContextFrame)> {
        (0..12)
            .flat_map(|p| {
                (0..4).map(move |f| {
                    mk(
                        &format!("prov-{p:02}"),
                        &format!("p{p:02}-f{f}"),
                        1.0 - (f as f32) / 10.0,
                        "content",
                    )
                })
            })
            .collect()
    };
    let first = ids(&RoundRobinByRank, build());
    let second = ids(&RoundRobinByRank, build());
    assert_eq!(first, second);
    // Tier 0 is one frame from each of the twelve providers, in provider order.
    let tier0: Vec<&String> = first.iter().take(12).collect();
    let mut expected: Vec<String> = (0..12).map(|p| format!("p{p:02}-f0")).collect();
    expected.sort();
    assert_eq!(
        tier0.into_iter().cloned().collect::<Vec<_>>(),
        expected,
        "every provider is seated before any provider's second frame"
    );
}

// ---- degeneration and contract ----

#[test]
fn with_one_provider_every_strategy_agrees_with_raw_score() {
    // The issue's own observation: with a single provider the
    // cross-provider question does not arise, and no strategy here may
    // invent one.
    let single = || {
        vec![
            mk("only", "a", 0.9, "a"),
            mk("only", "b", 0.5, "b"),
            mk("only", "c", 0.7, "c"),
            mk("only", "d", 0.7, "d"),
        ]
    };
    let baseline = ids(&ScoreDescending, single());
    assert_eq!(
        baseline,
        ["a", "c", "d", "b"],
        "score desc, FrameId tiebreak"
    );
    assert_eq!(ids(&RoundRobinByRank, single()), baseline);
    assert_eq!(ids(&PerProviderQuota::new(1), single()), baseline);
    assert_eq!(ids(&PerProviderQuota::new(7), single()), baseline);
    // Trust weights scale allocation between providers; with one provider
    // there is nothing to allocate between, whatever the weight — including
    // a last-resort zero.
    assert_eq!(ids(&TrustWeighted::default(), single()), baseline);
    assert_eq!(
        ids(&TrustWeighted::default().with_weight("only", 3), single()),
        baseline
    );
    assert_eq!(ids(&TrustWeighted::new(0), single()), baseline);
    // A precomputed order that names nothing falls back to round robin,
    // which with one provider is score order.
    assert_eq!(
        ids(&PrecomputedOrder::new("empty-rerank", Vec::new()), single()),
        baseline
    );
}

#[test]
fn an_empty_set_ranks_to_an_empty_set() {
    let trust = TrustWeighted::default();
    let reranked = reranked_lex_first();
    for strategy in [
        &ScoreDescending as &dyn RankingStrategy,
        &RoundRobinByRank,
        &PerProviderQuota::default(),
        &trust,
        &reranked,
    ] {
        assert!(rank_with(strategy, Vec::new()).is_empty());
    }
}

#[test]
fn a_quota_of_zero_is_read_as_one() {
    // Not a panic and not an error: a divide-by-zero in a ranking policy
    // would take down a host over a number that can only have meant "one".
    assert_eq!(PerProviderQuota::new(0).per_provider(), 1);
    assert_eq!(
        ids(&PerProviderQuota::new(0), generous_and_conservative()),
        ids(&RoundRobinByRank, generous_and_conservative()),
    );
}

/// A strategy that drops half its input and repeats an index — the shape a
/// third-party ranker gets wrong. `rank_with` must still return every frame.
struct Misbehaving;
impl RankingStrategy for Misbehaving {
    fn policy_name(&self) -> &str {
        "misbehaving"
    }
    fn order(&self, frames: &[(String, ContextFrame)]) -> Vec<usize> {
        if frames.is_empty() {
            return Vec::new();
        }
        vec![0, 0, usize::MAX]
    }
}

#[test]
fn every_shipped_strategy_returns_a_permutation() {
    // The trait contract, asserted the way a third-party strategy's own
    // tests should assert it.
    let frames = generous_and_conservative();
    let n = frames.len();
    let trust = lex_trusted();
    let last_resort = TrustWeighted::default().with_weight("sem", 0);
    let reranked = reranked_lex_first();
    // A partial order with a repeat and an identity matching no frame: the
    // shapes a real reranker's output takes when it times out or misfires.
    let partial = PrecomputedOrder::new(
        "partial-rerank",
        [
            id_of("sem", "sem-3"),
            id_of("sem", "sem-3"),
            FrameId::new("nowhere", "ghost", None),
        ],
    );
    for strategy in [
        &ScoreDescending as &dyn RankingStrategy,
        &RoundRobinByRank,
        &PerProviderQuota::new(2),
        &PerProviderQuota::default(),
        &trust,
        &last_resort,
        &reranked,
        &partial,
    ] {
        assert!(
            is_ranking_permutation(&strategy.order(&frames), n),
            "{} broke the ranking contract",
            strategy.policy_name()
        );
    }
    assert!(is_ranking_permutation(&[], 0));
    assert!(
        !is_ranking_permutation(&[0, 0], 2),
        "a repeat is not a permutation"
    );
    assert!(!is_ranking_permutation(&[0, 5], 2), "out of range");
    assert!(!is_ranking_permutation(&[0], 2), "too short");
}

#[test]
fn a_strategy_that_drops_frames_cannot_make_the_host_lose_evidence() {
    let frames = generous_and_conservative();
    let expected = frames.len();
    let ranked = rank_with(&Misbehaving, frames);
    assert_eq!(
        ranked.len(),
        expected,
        "every offered frame survives ranking, whoever wrote the strategy"
    );
    // The index it did place leads; the rest follow in canonical order.
    assert_eq!(ranked[0].1.id, "sem-1");
    assert_eq!(
        ranked[1..]
            .iter()
            .map(|(_, f)| f.id.clone())
            .collect::<Vec<_>>(),
        ["sem-2", "sem-3", "sem-4", "lex-1", "lex-2", "lex-3"],
    );
}

// ---- the budget bound holds under every strategy ----

/// The 64-bit LCG the sibling module's property loop uses (Numerical
/// Recipes constants), so the loop is reproducible without a dependency.
struct Lcg(u64);
impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

#[test]
fn no_strategy_can_select_a_set_that_exceeds_the_budget() {
    // Ranking decides *order*; the packer decides what fits. If a strategy
    // could push the composition over budget that would be a bug, not a
    // policy choice — so the bound is re-proved for each of them, and the
    // audit stays a total partition that explains every drop.
    let trust = TrustWeighted::default()
        .with_weight("prov0", 3)
        .with_weight("prov1", 0);
    let strategies: [&dyn RankingStrategy; 5] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(1),
        &PerProviderQuota::new(3),
        &trust,
    ];
    let mut rng = Lcg(0x5EED_1234_ABCD_9876);
    for iter in 0..300u64 {
        let provider_count = 1 + rng.below(5);
        let frame_count = rng.below(14);
        let budget = rng.below(150) as u32;
        let frames: Vec<(String, ContextFrame)> = (0..frame_count)
            .map(|i| {
                let provider = format!("prov{}", rng.below(provider_count));
                let len = rng.below(121) as usize;
                let score = (rng.below(101) as f32) / 100.0;
                let mut frame = ContextFrame::full(
                    format!("f{i}"),
                    FrameKind::Doc,
                    format!("f{i}"),
                    "z".repeat(len),
                    score,
                    budget_tokens(&"z".repeat(len)),
                );
                frame.content_digest = Some(format!("sha256:{provider}-{i}-{len}"));
                frame.citation_label = Some(format!("f{i} cite"));
                (provider, frame)
            })
            .collect();
        let borrowed: Vec<(&str, &ContextFrame)> =
            frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
        // A reranker's order over this iteration's frames — reversed arrival,
        // an order no shipped strategy would produce — naming only every
        // other frame, so the unnamed fallback is exercised too.
        let reranked = PrecomputedOrder::new(
            "reverse-arrival-rerank",
            frames
                .iter()
                .rev()
                .step_by(2)
                .map(|(p, f)| f.identity(p.as_str())),
        );

        for strategy in strategies
            .into_iter()
            .chain([&reranked as &dyn RankingStrategy])
        {
            let composed = compose_for_prompt_with(borrowed.iter().copied(), budget, strategy);
            let audit = &composed.audit;
            assert_eq!(
                audit.ranking_policy,
                strategy.policy_name(),
                "iter {iter}: the audit names the policy that ran"
            );
            assert!(
                audit.tokens_used <= budget,
                "iter {iter} / {}: tokens_used {} > budget {budget}",
                strategy.policy_name(),
                audit.tokens_used
            );
            assert_eq!(
                audit.entries.len(),
                frames.len(),
                "iter {iter} / {}: every offered frame is accounted for",
                strategy.policy_name()
            );
            assert!(audit.explains_every_drop(), "iter {iter}");
            // An independent re-sum of the included canonical costs.
            let included: Vec<_> = audit.included().cloned().collect();
            let resum: u32 = frames
                .iter()
                .filter(|(p, f)| included.contains(&f.identity(p)))
                .map(|(p, f)| rendered_token_cost(p, f))
                .sum();
            assert_eq!(audit.tokens_used, resum, "iter {iter}");
            // Nothing is excluded for a reason the packer did not record.
            for entry in audit.excluded() {
                assert!(matches!(
                    entry.disposition,
                    FrameDisposition::Excluded {
                        reason: ExclusionReason::Duplicate { .. }
                            | ExclusionReason::OverBudget { .. }
                    }
                ));
            }
        }
    }
}

// ---- trust weights (issue #115) ----

/// The canonical identity `mk` gives a frame, for naming frames in a
/// precomputed order.
fn id_of(provider: &str, id: &str) -> FrameId {
    FrameId::new(provider, id, Some(format!("sha256:{provider}-{id}")))
}

/// A host that trusts the conservative provider twice as much as it trusts
/// anyone else.
fn lex_trusted() -> TrustWeighted {
    TrustWeighted::default().with_weight("lex", 2)
}

/// A reranker's verdict over `generous_and_conservative`, naming three of
/// its seven frames — the shape a reranker that timed out partway produces.
fn reranked_lex_first() -> PrecomputedOrder {
    PrecomputedOrder::new(
        "test-reranker:lex-first",
        [
            id_of("lex", "lex-3"),
            id_of("sem", "sem-4"),
            id_of("lex", "lex-1"),
        ],
    )
}

#[test]
fn a_trust_weight_deals_a_provider_that_many_frames_per_round() {
    // `lex` weight 2, `sem` the default 1: `lex`'s best two, `sem`'s best,
    // `lex`'s third, then `sem`'s remainder one per round.
    assert_eq!(
        ids(&lex_trusted(), generous_and_conservative()),
        [
            "lex-1", "lex-2", "sem-1", "lex-3", "sem-2", "sem-3", "sem-4"
        ],
    );
    // Move the trust and the allocation moves with it.
    assert_eq!(
        ids(
            &TrustWeighted::default().with_weight("sem", 3),
            generous_and_conservative()
        ),
        [
            "lex-1", "sem-1", "sem-2", "sem-3", "lex-2", "sem-4", "lex-3"
        ],
    );
}

#[test]
fn a_trust_weight_scales_allocation_never_score() {
    // F10: a weighted score is still a score. The witness is that the
    // strategy is blind to the *scale* of a provider's scores — rescale
    // `sem` from the 0.9 band to the 0.05 band, keeping its internal order,
    // and the trust-weighted ranking does not move. Raw score ordering
    // moves completely, which is the control.
    let rescaled = || -> Vec<(String, ContextFrame)> {
        generous_and_conservative()
            .into_iter()
            .map(|(provider, mut frame)| {
                if provider == "sem" {
                    frame.score /= 18.0;
                }
                (provider, frame)
            })
            .collect()
    };
    for trust in [
        lex_trusted(),
        TrustWeighted::default().with_weight("sem", 3),
        TrustWeighted::new(2).with_weight("lex", 0),
    ] {
        assert_eq!(
            ids(&trust, rescaled()),
            ids(&trust, generous_and_conservative()),
            "weights {:?}: a provider's score scale must not reach the ranking",
            trust.weights().collect::<Vec<_>>()
        );
    }
    assert_ne!(
        ids(&ScoreDescending, rescaled()),
        ids(&ScoreDescending, generous_and_conservative()),
        "the control: raw score is exactly what the rescale moves"
    );
}

#[test]
fn a_trust_weight_decides_how_much_of_a_tight_budget_each_provider_gets() {
    // The consequence a host actually buys: under the four-frame budget of
    // the starvation scenario, the weight decides each provider's share —
    // and both providers are seated, which raw score cannot promise.
    let frames = generous_and_conservative();
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let budget: u32 = 4 * rendered_token_cost("sem", &frames[0].1);
    let share = |strategy: &TrustWeighted| -> (usize, usize) {
        let composed = compose_for_prompt_with(borrowed.iter().copied(), budget, strategy);
        let cited = |provider: &str| {
            composed
                .citations
                .iter()
                .filter(|c| c.frame.provider_id == provider)
                .count()
        };
        (cited("lex"), cited("sem"))
    };
    assert_eq!(share(&TrustWeighted::default()), (2, 2), "equal trust");
    assert_eq!(
        share(&TrustWeighted::default().with_weight("sem", 3)),
        (1, 3)
    );
    assert_eq!(
        share(&TrustWeighted::default().with_weight("lex", 3)),
        (3, 1)
    );
}

#[test]
fn a_zero_weight_is_a_last_resort_not_an_exclusion() {
    // A strategy ranks; it never filters. A provider the host does not
    // trust still has its frames ranked — after everyone else's — and a
    // budget that runs out first records them as over-budget in the audit.
    let distrust_sem = TrustWeighted::default().with_weight("sem", 0);
    assert_eq!(
        ids(&distrust_sem, generous_and_conservative()),
        [
            "lex-1", "lex-2", "lex-3", "sem-1", "sem-2", "sem-3", "sem-4"
        ],
    );

    let frames = generous_and_conservative();
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let budget: u32 = 4 * rendered_token_cost("sem", &frames[0].1);
    let composed = compose_for_prompt_with(borrowed.iter().copied(), budget, &distrust_sem);
    assert_eq!(composed.audit.entries.len(), frames.len());
    let excluded_sem = composed
        .audit
        .excluded()
        .filter(|entry| entry.frame.provider_id == "sem")
        .count();
    assert_eq!(
        excluded_sem, 3,
        "sem-1 fits; the rest are dropped with a reason"
    );
    assert!(composed.audit.explains_every_drop());
}

#[test]
fn trust_weighting_degenerates_to_the_unweighted_strategies() {
    // Unconfigured is a round robin; uniform weight `k` is a quota of `k`.
    // The weighted strategy generalizes the two; it does not compete with
    // them.
    assert_eq!(
        ids(&TrustWeighted::default(), generous_and_conservative()),
        ids(&RoundRobinByRank, generous_and_conservative()),
    );
    for k in [2u32, 3, 5] {
        assert_eq!(
            ids(&TrustWeighted::new(k), generous_and_conservative()),
            ids(
                &PerProviderQuota::new(k as usize),
                generous_and_conservative()
            ),
            "uniform weight {k}"
        );
    }
    let trust = TrustWeighted::new(2)
        .with_weight("sem", 4)
        .with_weight("lex", 1);
    assert_eq!(trust.weight_for("sem"), 4);
    assert_eq!(trust.weight_for("unlisted"), 2);
    assert_eq!(trust.default_weight(), 2);
    assert_eq!(
        trust.weights().collect::<Vec<_>>(),
        [("lex", 1), ("sem", 4)],
        "the table is reported in provider-id order, whatever the build order"
    );
}

// ---- a host's own reranker, run before composition (issue #115) ----

#[test]
fn a_precomputed_order_leads_and_the_unnamed_frames_follow_in_round_robin() {
    // The three named frames lead in the reranker's order; the four it did
    // not name keep the relative order `RoundRobinByRank` gives them over the
    // whole set, so the fallback makes no cross-provider score comparison of
    // its own. Ranks count the named frames too: `lex-2` is `lex`'s rank 1,
    // so `sem-1` (rank 0) leads the fallback even though the reranker lifted
    // `lex-1` out of it.
    assert_eq!(
        ids(&reranked_lex_first(), generous_and_conservative()),
        [
            "lex-3", "sem-4", "lex-1", "sem-1", "lex-2", "sem-2", "sem-3"
        ],
    );
    let reranked = reranked_lex_first();
    assert_eq!(reranked.position_of(&id_of("sem", "sem-4")), Some(1));
    assert_eq!(reranked.position_of(&id_of("sem", "sem-1")), None);
}

#[test]
fn a_precomputed_order_survives_repeats_and_strangers() {
    // A repeated identity keeps its first position; an identity that names
    // no offered frame is ignored. Neither moves or drops anything else.
    let messy = PrecomputedOrder::new(
        "messy",
        [
            id_of("sem", "sem-2"),
            FrameId::new("nowhere", "ghost", None),
            id_of("lex", "lex-2"),
            id_of("sem", "sem-2"),
        ],
    );
    assert_eq!(messy.position_of(&id_of("sem", "sem-2")), Some(0));
    let ranked = ids(&messy, generous_and_conservative());
    assert_eq!(&ranked[..2], &["sem-2", "lex-2"]);
    assert_eq!(ranked.len(), 7);
}

#[test]
fn a_reranked_order_is_keyed_by_identity_so_dedup_cannot_shift_it() {
    // The host reranks everything it received, before composition — and
    // composition then de-duplicates, which changes the slice (and every
    // index) the strategy sees. Keyed by `FrameId`, the host's order still
    // lands exactly: the dropped duplicate simply is not there to place.
    let shared = |provider: &str, id: &str, score: f32| {
        let (provider, mut frame) = mk(provider, id, score, "same");
        frame.content_digest = Some("sha256:shared".to_string());
        (provider, frame)
    };
    let frames = [
        shared("sem", "shared-a", 0.9),
        shared("lex", "shared-b", 0.3),
        mk("lex", "lex-x", 0.2, "xxxx"),
        mk("sem", "sem-y", 0.5, "yyyy"),
    ];
    // The reranker preferred the copy dedup is about to discard.
    let reranked = PrecomputedOrder::new(
        "test-reranker",
        [
            FrameId::new("lex", "shared-b", Some("sha256:shared".to_string())),
            id_of("lex", "lex-x"),
            id_of("sem", "sem-y"),
            FrameId::new("sem", "shared-a", Some("sha256:shared".to_string())),
        ],
    );
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let composed = compose_for_prompt_with(borrowed.iter().copied(), 10_000, &reranked);
    let included: Vec<String> = composed
        .audit
        .included()
        .map(|id| id.frame_id.clone())
        .collect();
    assert_eq!(included, ["lex-x", "sem-y", "shared-a"]);
    assert_eq!(composed.audit.excluded().count(), 1, "the duplicate");
    assert_eq!(composed.audit.ranking_policy, "test-reranker");
}

// ---- the audit names the policy (issue #116) ----

#[test]
fn the_audit_records_which_ranking_policy_ordered_the_frames() {
    // F10 says a host that orders frames across providers must document the
    // choice as its own. The choice is made per call, so the audit of the
    // call is where it is recorded.
    let frames = generous_and_conservative();
    let borrowed: Vec<(&str, &ContextFrame)> =
        frames.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let budget: u32 = 4 * rendered_token_cost("sem", &frames[0].1);

    let by_score = compose_for_prompt_with(borrowed.iter().copied(), budget, &ScoreDescending);
    let interleaved = compose_for_prompt_with(borrowed.iter().copied(), budget, &RoundRobinByRank);
    assert_eq!(by_score.audit.ranking_policy, "score-descending");
    assert_eq!(interleaved.audit.ranking_policy, "round-robin-by-rank");
    assert_ne!(
        by_score.audit.ranking_policy,
        interleaved.audit.ranking_policy
    );

    // The default entry point records its default, rather than nothing.
    let defaulted = compose_for_prompt(borrowed.iter().copied(), budget);
    assert_eq!(defaulted.audit, by_score.audit);

    // A host-named strategy records the host's name for it.
    let rerank_strategy = reranked_lex_first();
    let reranked = compose_for_prompt_with(borrowed.iter().copied(), budget, &rerank_strategy);
    assert_eq!(reranked.audit.ranking_policy, "test-reranker:lex-first");
    let trust = lex_trusted();
    let trusted = compose_for_prompt_with(borrowed.iter().copied(), budget, &trust);
    assert_eq!(trusted.audit.ranking_policy, "trust-weighted");
}

#[test]
fn two_policies_that_agree_on_every_frame_still_leave_different_audits() {
    // With one provider every strategy selects and orders the same frames,
    // so every entry matches — and the audits still differ, because the
    // rule that produced the composition is part of the record.
    let single = [
        mk("only", "a", 0.9, "aaaa"),
        mk("only", "b", 0.5, "bbbb"),
        mk("only", "c", 0.7, "cccc"),
    ];
    let borrowed: Vec<(&str, &ContextFrame)> =
        single.iter().map(|(p, f)| (p.as_str(), f)).collect();
    let budget: u32 = 2 * rendered_token_cost("only", &single[0].1);
    let quota_of_two = PerProviderQuota::new(2);
    let by_score = compose_for_prompt_with(borrowed.iter().copied(), budget, &ScoreDescending);
    let quota = compose_for_prompt_with(borrowed.iter().copied(), budget, &quota_of_two);
    assert_eq!(by_score.audit.entries, quota.audit.entries);
    assert_eq!(by_score.prompt, quota.prompt);
    assert_ne!(
        by_score.audit, quota.audit,
        "the same frames under two policies are two different compositions"
    );
}
