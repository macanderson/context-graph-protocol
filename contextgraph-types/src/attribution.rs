//! Retrieval attribution: closing the loop from a served frame to what it did
//! (`SPEC.md` §14; issue #31).
//!
//! The Context Frame spec's sixth question is *"why was each item included, and
//! can its effect be evaluated later?"* Provenance answers the first half —
//! where an item came from, and (§6.2) whether its bytes are still what they
//! claim. Nothing answered the second. A host could tell you a frame cost 42
//! tokens and came from `retry-policy.md`, and nothing at all about whether
//! including it helped.
//!
//! That gap is not theoretical: a host in this ecosystem already A/B-suppresses
//! recall on a fraction of turns to measure whether retrieval earns its budget,
//! entirely outside the protocol, because the protocol gave it no vocabulary to
//! say so.
//!
//! # What this is, and what it deliberately is not
//!
//! This is a **host-produced record**, exactly like [`UsageReport`](crate::UsageReport)
//! — not a wire method. There is no `context/feedback` envelope, no
//! `Capabilities.feedback`, and no host API that transmits any of this to a
//! provider.
//!
//! That restraint is the point. [ADR 0004](../../docs/adr/0004-dead-capability-surface.md)
//! purged `upsert`, `subscribe`, and `filters` from the 1.0 surface for being
//! capabilities no host could exercise, and §Q1 had to be written because
//! `kinds` shipped as a request field that every implementation ignored. Adding
//! a negotiated feedback method days before a freeze — with no provider
//! consuming it and no conformance check able to witness it — would recreate
//! precisely the defect that work removed.
//!
//! So the attribution *vocabulary* is specified now, because it is the half
//! that has to be shared for scores to be comparable across implementations,
//! and the wire hop that ships it back to a provider is deferred to a 1.x
//! additive minor (`docs/sketches/attribution-feedback.md`). Hosts can score
//! retrieval locally today; when a provider exists that consumes the signal,
//! the shape it consumes is already agreed.
//!
//! # The identity is not new
//!
//! Attribution needs a stable per-item handle, and the protocol already has
//! one: [`FrameId`](crate::FrameId), the `(provider id, frame id, content
//! digest)` triple that composition, dedup, usage reports, and `verify` all
//! key on. Minting a second id for attribution would let the two disagree —
//! and a disagreement between "the frame that was billed" and "the frame that
//! was cited" is exactly the confusion this record exists to prevent.

use serde::{Deserialize, Serialize};

use crate::identity::FrameId;
use crate::usage::UsageReport;

/// What actually became of one served frame, as three independent observations
/// (the `context_use` vocabulary of [ADR 0007](../../docs/adr/0007-protocol-product-boundary.md)).
///
/// They are deliberately **not** a single enum or a score. Each is a distinct,
/// separately-observable fact, and collapsing them would destroy the signal
/// that matters most: a frame that was `selected` and `rendered` but never
/// `cited` is the interesting case — the host paid its tokens and the model
/// read it, and it changed nothing. A single "used/unused" flag cannot express
/// that, and a 0–1 usefulness score would invent a precision nobody measured.
///
/// The three are ordered by inclusion in practice — a frame is rendered only if
/// selected, cited only if rendered — but that is an observation about honest
/// hosts, not an invariant this type enforces. See [`is_coherent`](Self::is_coherent).
// No `Default`, for the same reason [`FrameId`] has none: a record about no
// particular frame is not a sensible starting value, it is an un-reconcilable
// one. Build from [`ContextUse::selected`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextUse {
    /// The frame this record is about — the same identity the usage report
    /// billed and `verify` revalidates, never a separate attribution id.
    pub frame: FrameId,
    /// The host chose this frame from the fan-out results: it survived consent,
    /// the budget audit, and ranking.
    #[serde(default)]
    pub selected: bool,
    /// The frame's content was actually composed into the prompt the model saw.
    /// Distinct from `selected`: a frame can win ranking and still be dropped
    /// by budget packing before it reaches the prompt.
    #[serde(default)]
    pub rendered: bool,
    /// The model's output referred to this frame — by citation label, or by
    /// whatever attribution the host can observe.
    ///
    /// A *claim about observable output*, never an inference about influence.
    /// Whether a frame changed the model's reasoning is unobservable from
    /// outside; recording "it was cited" is a fact, recording "it helped" would
    /// be a guess wearing a fact's clothes.
    #[serde(default)]
    pub cited: bool,
}

impl ContextUse {
    /// A record for a frame the host selected but has not yet observed further.
    pub fn selected(frame: FrameId) -> Self {
        Self {
            frame,
            selected: true,
            rendered: false,
            cited: false,
        }
    }

    /// Mark the frame as having reached the prompt.
    pub fn rendered(mut self) -> Self {
        self.rendered = true;
        self
    }

    /// Mark the frame as referred to by the model's output.
    pub fn cited(mut self) -> Self {
        self.cited = true;
        self
    }

    /// Whether the three observations are mutually consistent: a frame cannot
    /// be cited without having been rendered, nor rendered without having been
    /// selected.
    ///
    /// A host that reports an incoherent record has an accounting bug, and
    /// scoring on it would silently mis-attribute value — so this is checkable
    /// rather than assumed.
    pub fn is_coherent(&self) -> bool {
        (!self.cited || self.rendered) && (!self.rendered || self.selected)
    }

    /// The frame reached the prompt and earned nothing observable — the case
    /// worth paying attention to, because it is pure spent budget.
    pub fn is_rendered_but_uncited(&self) -> bool {
        self.rendered && !self.cited
    }
}

/// Every [`ContextUse`] for one request, alongside the [`UsageReport`] that
/// says what those frames cost.
///
/// Cost and outcome are kept in one place on purpose: separately, each is
/// nearly useless. "This frame cost 400 tokens" prompts no decision, and "this
/// frame was never cited" prompts the wrong one if it cost four. Together they
/// give [`value_per_token`](Self::cited_token_share) — the ranking signal #31
/// and the token-cost work (#8) each supply half of.
// No `Default`: an attribution report without the usage report it reconciles
// against is not a degenerate case, it is a meaningless one — every ratio this
// type computes needs the cost side to exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributionReport {
    /// One record per frame the host selected, keyed by the same identity the
    /// usage report bills.
    #[serde(default)]
    pub uses: Vec<ContextUse>,
    /// The cost side of the ledger for the same request.
    pub usage: UsageReport,
}

impl AttributionReport {
    /// Pair outcome records with the cost report for the same request.
    pub fn new(uses: Vec<ContextUse>, usage: UsageReport) -> Self {
        Self { uses, usage }
    }

    /// The record for one frame identity, if the host observed it.
    pub fn use_of(&self, frame: &FrameId) -> Option<&ContextUse> {
        self.uses.iter().find(|entry| &entry.frame == frame)
    }

    /// Summed `token_cost` of frames that reached the prompt and were cited.
    pub fn cited_tokens(&self) -> u64 {
        self.tokens_where(|entry| entry.cited)
    }

    /// Summed `token_cost` of frames that reached the prompt and were **not**
    /// cited — the budget retrieval spent without visible return.
    pub fn uncited_rendered_tokens(&self) -> u64 {
        self.tokens_where(ContextUse::is_rendered_but_uncited)
    }

    /// The share of rendered budget that earned a citation, in `[0, 1]`.
    ///
    /// `None` when nothing was rendered — a request that retrieved nothing has
    /// no retrieval quality to report, and returning `0.0` would drag an
    /// average down with a turn that never asked anything of retrieval.
    pub fn cited_token_share(&self) -> Option<f64> {
        let rendered = self.tokens_where(|entry| entry.rendered);
        if rendered == 0 {
            return None;
        }
        Some(self.cited_tokens() as f64 / rendered as f64)
    }

    /// Whether every record is internally coherent and names a frame the usage
    /// report actually billed.
    ///
    /// The second half is what makes attribution auditable: a record about a
    /// frame nobody was charged for cannot be reconciled against the bill, and
    /// is the shape a mis-keyed identity takes.
    pub fn is_reconcilable(&self) -> bool {
        self.uses.iter().all(|entry| {
            entry.is_coherent()
                && self.usage.providers.iter().any(|provider| {
                    provider
                        .served_frames
                        .iter()
                        .any(|s| s.frame == entry.frame)
                })
        })
    }

    fn tokens_where(&self, predicate: impl Fn(&ContextUse) -> bool) -> u64 {
        self.uses
            .iter()
            .filter(|entry| predicate(entry))
            .filter_map(|entry| {
                self.usage
                    .providers
                    .iter()
                    .flat_map(|provider| provider.served_frames.iter())
                    .find(|served| served.frame == entry.frame)
                    .map(|served| served.token_cost as u64)
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{ProviderUsage, ServedFrame};

    fn frame(id: &str) -> FrameId {
        FrameId::new("docs", id, Some(format!("sha256:{id}")))
    }

    fn report(costs: &[(&str, u32)]) -> UsageReport {
        let served: Vec<ServedFrame> = costs
            .iter()
            .map(|(id, cost)| ServedFrame {
                frame: frame(id),
                token_cost: *cost,
            })
            .collect();
        let total: u64 = served.iter().map(|s| s.token_cost as u64).sum();
        UsageReport {
            budget_requested: 4096,
            budget_consumed: total,
            as_of: "2026-07-25T00:00:00Z".into(),
            providers: vec![ProviderUsage {
                provider_id: "docs".into(),
                frames_served: served.len() as u32,
                frames_rejected: 0,
                token_cost: total,
                served_frames: served,
            }],
        }
    }

    #[test]
    fn cost_and_outcome_together_give_a_value_signal() {
        let usage = report(&[("a", 100), ("b", 300)]);
        let attribution = AttributionReport::new(
            vec![
                ContextUse::selected(frame("a")).rendered().cited(),
                ContextUse::selected(frame("b")).rendered(),
            ],
            usage,
        );

        assert_eq!(attribution.cited_tokens(), 100);
        assert_eq!(attribution.uncited_rendered_tokens(), 300);
        // A quarter of the rendered budget earned a citation — the number
        // neither cost nor outcome could produce alone.
        assert_eq!(attribution.cited_token_share(), Some(0.25));
    }

    #[test]
    fn a_request_that_rendered_nothing_has_no_quality_to_report() {
        // Not 0.0: a turn that never asked anything of retrieval must not drag
        // down an average of turns that did.
        let attribution = AttributionReport::new(
            vec![ContextUse::selected(frame("a"))],
            report(&[("a", 100)]),
        );
        assert_eq!(attribution.cited_token_share(), None);
    }

    #[test]
    fn selected_but_unrendered_costs_nothing_against_quality() {
        // Ranked in, then dropped by budget packing before the prompt. It was
        // never shown, so it is neither credit nor debit.
        let attribution = AttributionReport::new(
            vec![
                ContextUse::selected(frame("a")).rendered().cited(),
                ContextUse::selected(frame("b")),
            ],
            report(&[("a", 100), ("b", 300)]),
        );
        assert_eq!(attribution.cited_token_share(), Some(1.0));
        assert_eq!(attribution.uncited_rendered_tokens(), 0);
    }

    #[test]
    fn incoherent_records_are_detectable() {
        let cited_without_rendering = ContextUse {
            frame: frame("a"),
            selected: true,
            rendered: false,
            cited: true,
        };
        assert!(!cited_without_rendering.is_coherent());

        let rendered_without_selecting = ContextUse {
            frame: frame("a"),
            selected: false,
            rendered: true,
            cited: false,
        };
        assert!(!rendered_without_selecting.is_coherent());

        assert!(
            ContextUse::selected(frame("a"))
                .rendered()
                .cited()
                .is_coherent()
        );
    }

    #[test]
    fn a_record_naming_an_unbilled_frame_does_not_reconcile() {
        // The shape a mis-keyed identity takes: attribution that cannot be
        // walked back to the bill is not auditable.
        let attribution = AttributionReport::new(
            vec![ContextUse::selected(frame("ghost")).rendered()],
            report(&[("a", 100)]),
        );
        assert!(!attribution.is_reconcilable());

        let honest = AttributionReport::new(
            vec![ContextUse::selected(frame("a")).rendered()],
            report(&[("a", 100)]),
        );
        assert!(honest.is_reconcilable());
    }
}
