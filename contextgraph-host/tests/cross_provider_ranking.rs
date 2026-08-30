//! Cross-provider ranking policy, driven through the public host API only
//! (`SPEC.md` §6.6, F10; issue #95).
//!
//! An integration test rather than a unit one, because the thing under test is
//! precisely that a host *outside* this crate can bring its own cross-provider
//! ranking. Everything here is reachable from `contextgraph_host`'s root.

use contextgraph_host::{
    ComposedPrompt, ExclusionReason, FrameDisposition, PerProviderQuota, RankingStrategy,
    RoundRobinByRank, ScoreDescending, compose_for_prompt_with, is_ranking_permutation, rank_with,
};
use contextgraph_types::{ContextFrame, FrameKind, budget_tokens};

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
    // Four frames fit (each is one token of content under the canonical
    // accounting), so `score` ordering spends the entire budget on `sem`.
    let budget: u32 = 4;
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
    let strategies: [&dyn RankingStrategy; 3] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(2),
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
    let strategies: [&dyn RankingStrategy; 3] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(3),
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
        assert_eq!(first.audit.entries, second.audit.entries);
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
}

#[test]
fn an_empty_set_ranks_to_an_empty_set() {
    for strategy in [
        &ScoreDescending as &dyn RankingStrategy,
        &RoundRobinByRank,
        &PerProviderQuota::default(),
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
    for strategy in [
        &ScoreDescending as &dyn RankingStrategy,
        &RoundRobinByRank,
        &PerProviderQuota::new(2),
        &PerProviderQuota::default(),
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
    let strategies: [&dyn RankingStrategy; 4] = [
        &ScoreDescending,
        &RoundRobinByRank,
        &PerProviderQuota::new(1),
        &PerProviderQuota::new(3),
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

        for strategy in strategies {
            let composed = compose_for_prompt_with(borrowed.iter().copied(), budget, strategy);
            let audit = &composed.audit;
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
                .map(|(_, f)| f.expected_inline_token_cost())
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
