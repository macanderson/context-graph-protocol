//! `context/query` request/response shapes
//! (`docs/specs/stella-rust-cli/06-context-protocol.md` §3.3). Budget-aware
//! by contract: every query carries `max_tokens`; a conforming provider
//! never returns more than the budget and never lies about cost.

use serde::{Deserialize, Serialize};

use crate::frame::{ContextFrame, FrameKind, Representation};

/// A request to an Context Graph Protocol provider for context frames relevant to a goal.
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
}
