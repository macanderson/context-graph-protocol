//! Adversarial proof: a provider that is conformant on every §7 budget rule can
//! still drive the host's real prompt far past the budget it was given.
//!
//! §B3 anchors `token_cost` to `ceil(utf8_len(content) / 4)` — `content` alone.
//! §7.2 is explicit that the host's rendering chrome is "the host's cost to
//! budget". But `title` and `citation_label` are *provider*-controlled bytes
//! that §F2/§F3 force the host to render, and the reference packer charged
//! neither. A frame with empty content therefore cost zero budget tokens while
//! contributing an unbounded number of real ones.

use contextgraph_host::compose::{EVIDENCE_PREAMBLE, compose_for_prompt};
use contextgraph_types::{ContextFrame, FrameKind, budget_tokens};

/// A frame that satisfies every frame-validity and budget rule the conformance
/// suite checks: non-empty title (§F2), non-empty citation label (§F3), and a
/// `token_cost` exactly equal to the canonical count of its content (§B3).
fn conformant_frame(id: &str, content: &str, label: &str) -> ContextFrame {
    ContextFrame {
        id: id.into(),
        kind: FrameKind::Doc,
        title: id.into(),
        content: Some(content.into()),
        citation_label: Some(label.into()),
        score: 0.9,
        token_cost: budget_tokens(content),
        content_digest: None,
        uri: None,
        representation: Default::default(),
        content_fidelity: None,
        canonical_content_hash: None,
        content_ref: None,
        transform: None,
        minimum_content_fidelity: None,
        inline_content_requirement: None,
        canonical_token_cost: None,
        tokenizer_ref: None,
        valid_from: None,
        valid_to: None,
        recorded_at: None,
        provenance: vec![],
        embedding: None,
        relations: vec![],
    }
}

/// The cost of everything in a composed prompt that the *provider* controls:
/// the whole prompt less the host's own fixed preamble. A host knows the
/// preamble is coming and what it costs — [`EVIDENCE_PREAMBLE`] is a public
/// constant it can budget for. What it cannot know in advance, and what the
/// budget contract must therefore bound, is how many bytes the provider will
/// contribute.
fn provider_contributed_cost(prompt: &str) -> u32 {
    let body = prompt
        .strip_prefix(EVIDENCE_PREAMBLE)
        .expect("every composed prompt opens with the preamble");
    budget_tokens(body)
}

#[test]
fn a_budget_conformant_frame_cannot_smuggle_unbudgeted_bytes_into_the_prompt() {
    // 40 KiB of citation label. Every §7 rule still holds: the frame's content
    // is empty, so its honest canonical cost is 0.
    let label = "A".repeat(40 * 1024);
    let smuggler = conformant_frame("smuggler", "", &label);
    assert_eq!(smuggler.token_cost, 0, "empty content costs nothing (§B3)");

    let budget = 64;
    let composed = compose_for_prompt([("p", &smuggler)], budget);

    assert!(
        composed.audit.tokens_used <= budget,
        "the audit may never report spending more than the budget"
    );

    let smuggled = provider_contributed_cost(&composed.prompt);
    assert!(
        smuggled <= budget,
        "a frame with an honest token_cost of 0 put {smuggled} budget tokens of \
         provider-controlled chrome into the prompt against a budget of {budget}"
    );
}

/// The property that actually matters, and the one a single-size test cannot
/// state: the prompt's cost is bounded by the host's budget plus a *host*
/// constant, and does not grow with the size of the provider's chrome. If an
/// attacker can buy prompt bytes by making its labels longer, the budget is
/// advisory no matter what any single measurement shows.
#[test]
fn the_prompt_does_not_grow_with_the_size_of_a_providers_chrome() {
    let budget = 64;
    let mut costs = Vec::new();
    for kib in [1usize, 64, 512] {
        let label = "A".repeat(kib * 1024);
        let smuggler = conformant_frame("smuggler", "", &label);
        let composed = compose_for_prompt([("p", &smuggler)], budget);
        costs.push((kib, budget_tokens(&composed.prompt)));
    }
    let baseline = costs[0].1;
    for (kib, cost) in &costs {
        assert_eq!(
            *cost, baseline,
            "growing the citation label to {kib} KiB changed the composed prompt's \
             cost to {cost} (baseline {baseline}) — chrome is buying prompt bytes"
        );
    }
}
