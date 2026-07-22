//! `context/query` request/response shapes
//! (`SPEC.md` §5). Budget-aware
//! by contract: every query carries `max_tokens`; a conforming provider
//! never returns more than the budget and never lies about cost.

use serde::{Deserialize, Serialize};

use crate::frame::{ContextFrame, FrameKind, Representation};

/// A request to a CGP provider for context frames relevant to a goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextQuery {
    /// The task/turn goal driving retrieval.
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<FrameKind>,
    /// Anchor URIs (open files, mentioned symbols) used for graph-proximity
    /// scoring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<String>,
    pub max_frames: u32,
    pub max_tokens: u32,
    /// Pin retrieval to a point in time for bi-temporal facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    /// Ordered [frame representation](Representation) preference. The provider
    /// returns the first supported representation it can satisfy. Empty on the
    /// wire ⇒ the default `[full]`, so pre-representation hosts are unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub representation_preferences: Vec<Representation>,
}

impl ContextQuery {
    /// The effective ordered representation preference, defaulting to `[full]`
    /// when the host stated none (the legacy behavior).
    pub fn preferred_representations(&self) -> Vec<Representation> {
        if self.representation_preferences.is_empty() {
            vec![Representation::Full]
        } else {
            self.representation_preferences.clone()
        }
    }

    /// The representation a provider should return: the first
    /// [preferred](Self::preferred_representations) one it supports. `None` ⇒
    /// none of the requested representations is supported and the provider must
    /// answer `unsupported_representation`.
    pub fn select_representation(&self, supported: &[Representation]) -> Option<Representation> {
        self.preferred_representations()
            .into_iter()
            .find(|wanted| supported.contains(wanted))
    }
}

/// The response to a `context/query` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextQueryResult {
    pub frames: Vec<ContextFrame>,
    /// True if the provider had more candidates than fit the budget.
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_estimate: Option<u32>,
}

impl ContextQueryResult {
    /// Sum of `token_cost` across returned frames — must never exceed the
    /// query's `max_tokens` for a conforming provider (checked in
    /// `contextgraph-conformance`, phase 3; this is the cheap client-side sanity
    /// check any host can run today).
    pub fn total_token_cost(&self) -> u64 {
        self.frames.iter().map(|f| f.token_cost as u64).sum()
    }

    pub fn respects_budget(&self, max_tokens: u32) -> bool {
        self.total_token_cost() <= max_tokens as u64
    }

    /// Whether the provider honored the query's `max_frames` cap
    /// (`SPEC.md` §B4).
    ///
    /// `max_frames` was part of the query contract from the beginning and was
    /// audited by nothing: a provider returning ten thousand one-token frames
    /// against `max_frames: 8` passed every check. Frame count is a real cost
    /// — each frame carries a title, a citation label, and rendering chrome the
    /// token budget does not capture.
    pub fn respects_frame_limit(&self, max_frames: u32) -> bool {
        self.frames.len() as u64 <= max_frames as u64
    }

    /// Frames whose declared `token_cost` does not match the canonical count
    /// for their content (`SPEC.md` §B3).
    ///
    /// Returns ids so a host's audit report can name the offending frames
    /// rather than only the provider.
    pub fn frames_with_dishonest_cost(&self) -> Vec<&str> {
        self.frames
            .iter()
            .filter(|f| !f.declares_honest_token_cost())
            .map(|f| f.id.as_str())
            .collect()
    }

    /// The sum of the *canonical* costs of the returned frames — what the
    /// provider's frames actually cost, as opposed to what it claimed.
    pub fn canonical_token_cost(&self) -> u64 {
        self.frames
            .iter()
            .map(|f| f.expected_inline_token_cost() as u64)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::ContextFrame;

    fn frame_with_cost(id: &str, cost: u32) -> ContextFrame {
        ContextFrame::full(id, FrameKind::Snippet, id, String::new(), 0.5, cost)
    }

    #[test]
    fn context_query_roundtrips() {
        let query = ContextQuery {
            goal: "fix the failing test".into(),
            query_text: Some("failing test".into()),
            embedding: None,
            kinds: vec![FrameKind::Symbol, FrameKind::Doc],
            anchors: vec!["file:///repo/src/lib.rs".into()],
            max_frames: 20,
            max_tokens: 4000,
            as_of: None,
            representation_preferences: vec![],
        };
        let json = serde_json::to_string(&query).unwrap();
        let back: ContextQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back, query);
    }

    #[test]
    fn representation_preferences_default_to_full_and_select_first_supported() {
        // A host that states nothing gets the legacy `[full]` behavior, and the
        // field is omitted from the wire.
        let mut query = ContextQuery {
            goal: "g".into(),
            query_text: None,
            embedding: None,
            kinds: vec![],
            anchors: vec![],
            max_frames: 1,
            max_tokens: 10,
            as_of: None,
            representation_preferences: vec![],
        };
        assert_eq!(
            query.preferred_representations(),
            vec![Representation::Full]
        );
        assert!(
            !serde_json::to_string(&query)
                .unwrap()
                .contains("representation_preferences")
        );
        assert_eq!(
            query.select_representation(&[Representation::Full]),
            Some(Representation::Full)
        );

        // With an explicit preference, the provider returns the first it can
        // satisfy; if none is supported, it must answer unsupported.
        query.representation_preferences = vec![Representation::Reference, Representation::Full];
        assert_eq!(
            query.select_representation(&[Representation::Full]),
            Some(Representation::Full),
        );
        assert_eq!(
            query.select_representation(&[Representation::Reference, Representation::Full]),
            Some(Representation::Reference),
        );
        assert_eq!(
            query.select_representation(&[Representation::Compact]),
            None
        );
    }

    #[test]
    fn respects_budget_true_when_under_or_at_limit() {
        let result = ContextQueryResult {
            frames: vec![frame_with_cost("a", 100), frame_with_cost("b", 200)],
            truncated: false,
            dropped_estimate: None,
        };
        assert_eq!(result.total_token_cost(), 300);
        assert!(result.respects_budget(300));
        assert!(result.respects_budget(500));
    }

    #[test]
    fn respects_budget_false_when_provider_lies_about_cost() {
        let result = ContextQueryResult {
            frames: vec![frame_with_cost("a", 400)],
            truncated: false,
            dropped_estimate: None,
        };
        assert!(!result.respects_budget(300));
    }

    /// A frame whose declared cost is the canonical cost of its content.
    fn honest_frame(id: &str, content: &str) -> ContextFrame {
        let mut frame = frame_with_cost(id, 0);
        frame.content = Some(content.to_string());
        frame.token_cost = frame.expected_inline_token_cost();
        frame
    }

    #[test]
    fn frame_limit_catches_the_provider_that_floods_with_cheap_frames() {
        // The exact hole from issue #10: ten thousand one-token frames against
        // `max_frames: 8` used to pass everything, because only the token
        // budget was audited.
        let flood = ContextQueryResult {
            frames: (0..50)
                .map(|i| honest_frame(&format!("f{i}"), "x"))
                .collect(),
            truncated: false,
            dropped_estimate: None,
        };
        assert!(flood.respects_budget(10_000), "the token budget is fine");
        assert!(!flood.respects_frame_limit(8), "but the frame cap is not");
        assert!(flood.respects_frame_limit(50), "boundary is inclusive");
    }

    #[test]
    fn an_honest_result_reports_no_dishonest_frames() {
        let result = ContextQueryResult {
            frames: vec![honest_frame("a", "abcd"), honest_frame("b", "abcdefgh")],
            truncated: false,
            dropped_estimate: None,
        };
        assert!(result.frames_with_dishonest_cost().is_empty());
        assert_eq!(result.total_token_cost(), result.canonical_token_cost());
    }

    #[test]
    fn dishonest_frames_are_named_and_the_true_cost_is_recoverable() {
        let mut liar = honest_frame("liar", &"x".repeat(4_000));
        liar.token_cost = 1; // claims 1, actually costs 1_000
        let result = ContextQueryResult {
            frames: vec![honest_frame("honest", "abcd"), liar],
            truncated: false,
            dropped_estimate: None,
        };

        assert_eq!(result.frames_with_dishonest_cost(), vec!["liar"]);
        // The declared sum sails under a budget the real content blows past.
        assert_eq!(result.total_token_cost(), 2);
        assert_eq!(result.canonical_token_cost(), 1_001);
        assert!(result.respects_budget(100));
    }
}
