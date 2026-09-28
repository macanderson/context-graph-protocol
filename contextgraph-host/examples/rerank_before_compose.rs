//! A host's own reranker, run **before** composition ([ADR 0015], amendment
//! of 2026-09-28; issue #115).
//!
//! Run it:
//!
//! ```text
//! cargo run -p contextgraph-host --example rerank_before_compose
//! ```
//!
//! A reranker does I/O — a cross-encoder call, a local model, a remote API —
//! and `RankingStrategy::order` is a synchronous pure function, so a
//! reranker cannot *be* a strategy. It does not need to be. The host already
//! awaits its fan-out; it awaits the reranker beside it, then hands the
//! verdict to composition as data through [`PrecomputedOrder`]. The reranker
//! never enters the crate, composition stays pure and byte-reproducible, and
//! the audit records the reranker by name.
//!
//! The example walks the whole flow:
//!
//! 1. the frames a fan-out returned — a *generous* semantic provider scoring
//!    in the `0.9` band and a *conservative* lexical one near `0.4`, the exact
//!    shape `SPEC.md` §6.6 (F10) warns about;
//! 2. compose them by raw `score` under a tight budget, and watch the
//!    lexical provider — which holds the one line that answers the question —
//!    fall out of the prompt;
//! 3. `await` a reranker over the same frames, wrap its verdict in a
//!    [`PrecomputedOrder`], and compose again: the answer is seated, and the
//!    audit names the reranker as the policy that decided it;
//! 4. for contrast, the same frames under [`TrustWeighted`] — a host that has
//!    no reranker but trusts the lexical provider more.
//!
//! The reranker here is a deterministic term-overlap stand-in, so the example
//! runs offline and prints the same bytes every time. A real host swaps in its
//! model call; nothing else in the flow changes.
//!
//! [ADR 0015]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0015-cross-provider-ranking-strategies.md

use std::collections::BTreeSet;
use std::time::Duration;

use contextgraph_host::{
    ComposedPrompt, FrameDisposition, PrecomputedOrder, ScoreDescending, TrustWeighted,
    compose_for_prompt_with, rendered_token_cost,
};
use contextgraph_types::{ContextFrame, FrameId, FrameKind, budget_tokens};

/// What the user asked. The reranker scores every frame against it.
const QUERY: &str = "why does the retry loop give up after three attempts";

#[tokio::main]
async fn main() {
    // ---- 1. What the fan-out returned ------------------------------------
    // A real host holds these after `Host::query_all`; they are built inline
    // here so the example needs no running provider.
    let frames = fanned_out_frames();
    println!("== query ==\n{QUERY}\n");
    println!("== frames offered (provider-local scores) ==");
    for (provider, frame) in &frames {
        println!("  {provider:<9} {:<18} score {:.2}", frame.id, frame.score);
    }

    // A budget that fits exactly the three frames raw `score` ranks first:
    // the generous provider's whole set.
    let budget: u32 = frames
        .iter()
        .filter(|(provider, _)| provider == "semantic")
        .map(|(provider, frame)| rendered_token_cost(provider, frame))
        .sum();
    let borrowed: Vec<(&str, &ContextFrame)> = frames
        .iter()
        .map(|(provider, frame)| (provider.as_str(), frame))
        .collect();
    println!("\nbudget: {budget} tokens (rendered cost, chrome included)");

    // ---- 2. Raw score: the documented default, and its failure mode ------
    let by_score = compose_for_prompt_with(borrowed.iter().copied(), budget, &ScoreDescending);
    report(&by_score);
    assert!(
        !cites(&by_score, "retry.rs:42"),
        "raw score spends the budget on the generous provider"
    );

    // ---- 3. Rerank first, then compose -----------------------------------
    // The only async step, and it happens here — in the host, beside the
    // fan-out — not inside composition.
    let verdict = rerank(QUERY, &frames).await;
    let reranked = PrecomputedOrder::new(RERANKER, verdict);
    let composed = compose_for_prompt_with(borrowed.iter().copied(), budget, &reranked);
    report(&composed);
    assert!(
        cites(&composed, "retry.rs:42"),
        "the reranker seats the line that answers the question"
    );
    assert_eq!(composed.audit.ranking_policy, RERANKER);

    // Composition is still a pure function of (frames, budget, order): the
    // same verdict composes to the same bytes, so a provider prompt cache
    // keeps hitting. The reranker's own determinism is the reranker's
    // business; composition adds no nondeterminism of its own.
    let again = compose_for_prompt_with(borrowed.iter().copied(), budget, &reranked);
    assert_eq!(again.prompt, composed.prompt);
    assert_eq!(again.audit, composed.audit);

    // ---- 4. No reranker, but a trust weighting ---------------------------
    // A weight scales how many frames a provider is dealt per round — never
    // its score, which F10 forbids treating as cross-provider relevance.
    let trust = TrustWeighted::default().with_weight("grep", 2);
    let trusted = compose_for_prompt_with(borrowed.iter().copied(), budget, &trust);
    report(&trusted);
    println!(
        "  weights: default {} + {:?}",
        trust.default_weight(),
        trust.weights().collect::<Vec<_>>()
    );
}

/// The name the audit records. A real host names the model and its version,
/// so a reader of the audit knows which ranker decided the evidence set.
const RERANKER: &str = "term-overlap-reranker:example-v1";

/// A stand-in for a cross-encoder: the number of distinct query terms a
/// frame's title and content share with the query, best first, ties broken on
/// the canonical [`FrameId`] so the verdict is reproducible.
///
/// It is `async` because the real thing is: a network round trip or a model
/// invocation. The sleep stands in for that latency and is the reason this
/// function cannot be a `RankingStrategy`.
async fn rerank(query: &str, frames: &[(String, ContextFrame)]) -> Vec<FrameId> {
    tokio::time::sleep(Duration::from_millis(5)).await;

    let wanted = terms(query);
    let mut scored: Vec<(usize, FrameId)> = frames
        .iter()
        .map(|(provider, frame)| {
            let text = format!("{} {}", frame.title, frame.content.as_deref().unwrap_or(""));
            let overlap = terms(&text).intersection(&wanted).count();
            (overlap, frame.identity(provider.as_str()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, id)| id).collect()
}

/// Lowercased alphanumeric words longer than two characters.
fn terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(str::to_lowercase)
        .collect()
}

/// Print what a composition included, what it dropped, and under which
/// policy — the three things its audit exists to answer.
fn report(composed: &ComposedPrompt) {
    let audit = &composed.audit;
    println!(
        "\n== composed under `{}` — {} of {} tokens ==",
        audit.ranking_policy, audit.tokens_used, audit.global_budget
    );
    for entry in &audit.entries {
        let verdict = match &entry.disposition {
            FrameDisposition::Included { .. } => "included".to_string(),
            FrameDisposition::Excluded { reason } => format!("excluded ({reason:?})"),
        };
        println!(
            "  {:<9} {:<18} {verdict}",
            entry.frame.provider_id, entry.frame.frame_id
        );
    }
}

/// Whether the composed prompt cites the frame with this id.
fn cites(composed: &ComposedPrompt, frame_id: &str) -> bool {
    composed
        .citations
        .iter()
        .any(|citation| citation.frame.frame_id == frame_id)
}

/// The fan-out's result: two honest providers on different scales.
fn fanned_out_frames() -> Vec<(String, ContextFrame)> {
    vec![
        frame(
            "semantic",
            "retry-guide",
            0.94,
            "The client retries idempotent requests with exponential backoff.",
        ),
        frame(
            "semantic",
            "backoff-notes",
            0.91,
            "Backoff spreads load; jitter keeps clients from retrying in lockstep.",
        ),
        frame(
            "semantic",
            "client-intro",
            0.88,
            "The HTTP client wraps its transport and adds a tracing span to each call.",
        ),
        frame(
            "grep",
            "retry.rs:42",
            0.41,
            "if attempts >= MAX_ATTEMPTS { return Err(GaveUp) } // the retry loop gives up after three attempts",
        ),
        frame(
            "grep",
            "config.toml:7",
            0.33,
            "max_attempts = 3  # the retry loop stops after this many attempts",
        ),
    ]
}

/// One frame, as a provider would return it: canonical `token_cost`
/// (`SPEC.md` §B3), a citation label, and a digest so dedup and identity work
/// as they do on real evidence. The digest is a stand-in label, not a hash of
/// the content — nothing in this example verifies it.
fn frame(provider: &str, id: &str, score: f32, content: &str) -> (String, ContextFrame) {
    let mut frame = ContextFrame::full(
        id,
        FrameKind::Doc,
        id,
        content,
        score,
        budget_tokens(content),
    );
    frame.content_digest = Some(format!("sha256:{provider}-{id}"));
    frame.citation_label = Some(format!("{provider}/{id}"));
    (provider.to_string(), frame)
}
