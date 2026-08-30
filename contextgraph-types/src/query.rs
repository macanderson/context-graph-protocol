//! `context/query` request/response shapes
//! (`SPEC.md` §5). Budget-aware
//! by contract: every query carries `max_tokens`; a conforming provider
//! never returns more than the budget and never lies about cost.

use serde::{Deserialize, Serialize};

use crate::attest::{FrameAttestation, ProvenanceAttestation};
use crate::frame::{ContextFrame, FrameKind, Representation};
use crate::identity::FrameId;

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
///
/// # Where an attestation rides
///
/// [`frame_attestations`](Self::frame_attestations) and
/// [`result_attestation`](Self::result_attestation) are the wire home of
/// `SPEC.md` §6.5's evidence (§6.5.5, F11–F13). They sit on the *result* rather
/// than on the `frames` envelope because an attestation is a property of the
/// answer, exactly like `truncated`: the envelope carries only what the
/// transport needs (`type`, the correlation `id`), and an in-process provider
/// that returns a `ContextQueryResult` with no envelope at all must still be
/// able to sign what it serves.
///
/// Both are optional and both are omitted when empty, so an unsigned answer is
/// byte-identical to one from a provider written before this existed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextQueryResult {
    pub frames: Vec<ContextFrame>,
    /// True if the provider had more candidates than fit the budget.
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_estimate: Option<u32>,
    /// Detached per-frame evidence, one entry per attested frame
    /// (`SPEC.md` §6.5.5). Never a parallel array: each entry names the
    /// [`FrameId`] it covers in full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_attestations: Vec<FrameAttestation>,
    /// One signature over the whole answer: a detached attestation whose
    /// `signed_commitment` is the §6.5.3 Merkle root over the commitments of
    /// exactly the frames in [`frames`](Self::frames), in canonical order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_attestation: Option<ProvenanceAttestation>,
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

    /// Whether this answer carries any detached evidence at all
    /// (`SPEC.md` §6.5.5).
    pub fn is_attested(&self) -> bool {
        self.result_attestation.is_some()
            || self.frame_attestations.iter().any(|a| a.carries_evidence())
    }

    /// The attestation entry covering one frame identity, if the provider sent
    /// one.
    ///
    /// Matching is on the whole `(provider_id, frame_id, content_digest)`
    /// triple, never on the frame id alone: two frames sharing an id but not a
    /// digest are different bytes, and handing the first one's evidence to the
    /// second is the substitution the identity binding exists to prevent
    /// (`SPEC.md` §6.5.2).
    pub fn attestation_for(&self, frame: &FrameId) -> Option<&FrameAttestation> {
        self.frame_attestations.iter().find(|a| &a.frame == frame)
    }

    /// Attestation entries naming a frame this result does not carry
    /// (`SPEC.md` §6.5.5, F11).
    ///
    /// An entry with no frame beside it is evidence for something the host was
    /// never shown. It is not merely useless: a host that counted entries
    /// rather than matching them would report an answer as more thoroughly
    /// attested than it is. Returned as identities so a report can name them.
    pub fn orphaned_attestations(&self, provider_id: &str) -> Vec<&FrameId> {
        let present: Vec<FrameId> = self
            .frames
            .iter()
            .map(|frame| frame.identity(provider_id))
            .collect();
        self.frame_attestations
            .iter()
            .map(|entry| &entry.frame)
            .filter(|id| !present.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::{InclusionProof, InclusionStep};
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };

        assert_eq!(result.frames_with_dishonest_cost(), vec!["liar"]);
        // The declared sum sails under a budget the real content blows past.
        assert_eq!(result.total_token_cost(), 2);
        assert_eq!(result.canonical_token_cost(), 1_001);
        assert!(result.respects_budget(100));
    }

    // -----------------------------------------------------------------------
    // Attestations on the wire (`SPEC.md` §6.5.5, F11–F13; ADR 0014)
    // -----------------------------------------------------------------------

    fn sample_attestation(commitment: &str) -> ProvenanceAttestation {
        ProvenanceAttestation::new(
            commitment,
            "key-1",
            crate::ALGORITHM_ED25519,
            "example-provider",
            "ab".repeat(64),
            "2026-08-29T00:00:00Z",
        )
    }

    fn attested_result() -> ContextQueryResult {
        let mut frame = frame_with_cost("frame:a", 4);
        frame.content_digest = Some(format!("sha256:{}", "11".repeat(32)));
        let identity = frame.identity("example-provider");
        ContextQueryResult {
            frames: vec![frame],
            truncated: false,
            dropped_estimate: None,
            frame_attestations: vec![
                FrameAttestation::signed(
                    identity,
                    sample_attestation(&format!("sha256:{}", "22".repeat(32))),
                )
                .with_inclusion_proof(InclusionProof {
                    leaf_index: 0,
                    leaf_count: 1,
                    path: vec![],
                }),
            ],
            result_attestation: Some(sample_attestation(&format!("sha256:{}", "33".repeat(32)))),
        }
    }

    #[test]
    fn an_attested_result_round_trips_byte_for_byte() {
        // The whole point of putting the attestation on the result: it has to
        // survive the trip. A shape that serializes but does not come back is
        // evidence a host cannot store.
        let result = attested_result();
        let json = serde_json::to_string(&result).unwrap();
        let back: ContextQueryResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        assert!(result.is_attested());
    }

    #[test]
    fn the_attestation_never_travels_inside_the_frame_it_covers() {
        // F6/F11. Detachment is the reason re-signing and key rotation cannot
        // perturb a frame's content-addressed identity, so it is checked on the
        // serialized bytes rather than trusted to the struct layout.
        let result = attested_result();
        let value = serde_json::to_value(&result).unwrap();
        let frame = &value["frames"][0];
        for member in ["attestation", "frame_attestations", "result_attestation"] {
            assert!(
                frame.get(member).is_none(),
                "a frame must carry no attestation member, found `{member}`: {frame}"
            );
        }
        assert!(value.get("frame_attestations").is_some());
        assert!(value.get("result_attestation").is_some());
    }

    #[test]
    fn an_unsigned_answer_is_byte_identical_to_one_from_a_provider_that_predates_this() {
        // Additive within contextgraph/1: a provider that signs nothing must
        // emit exactly the bytes it emitted before these members existed, or
        // every existing golden fixture and cache key moves.
        let result = ContextQueryResult {
            frames: vec![frame_with_cost("a", 1)],
            truncated: false,
            dropped_estimate: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("frame_attestations"), "{json}");
        assert!(!json.contains("result_attestation"), "{json}");
        assert!(!result.is_attested());
    }

    #[test]
    fn an_old_consumer_ignoring_the_new_members_still_parses_the_envelope() {
        // SPEC.md §13 U1 in the direction that matters here: the members are
        // optional, so a 1.0 peer that drops them still reads a signed answer
        // as a valid answer.
        let attested = serde_json::to_value(attested_result()).unwrap();
        let mut stripped = attested.as_object().unwrap().clone();
        stripped.remove("frame_attestations");
        stripped.remove("result_attestation");
        let back: ContextQueryResult =
            serde_json::from_value(serde_json::Value::Object(stripped)).unwrap();
        assert!(!back.is_attested());
        assert_eq!(back.frames, attested_result().frames);
    }

    #[test]
    fn an_entry_is_matched_on_the_whole_identity_triple_not_the_frame_id() {
        // The substitution §6.5.2's identity binding exists to prevent: two
        // frames sharing an id but not a digest are different bytes, and one's
        // evidence must not answer for the other.
        let result = attested_result();
        let served = result.frames[0].identity("example-provider");
        assert!(result.attestation_for(&served).is_some());

        let same_id_other_bytes = FrameId::new(
            "example-provider",
            "frame:a",
            Some(format!("sha256:{}", "99".repeat(32))),
        );
        assert!(
            result.attestation_for(&same_id_other_bytes).is_none(),
            "different bytes must not inherit another frame's attestation"
        );
        let other_provider = FrameId::new(
            "impostor",
            "frame:a",
            result.frames[0].content_digest.clone(),
        );
        assert!(result.attestation_for(&other_provider).is_none());
    }

    #[test]
    fn an_attestation_for_a_frame_the_host_never_received_is_reported_as_orphaned() {
        let mut result = attested_result();
        assert!(result.orphaned_attestations("example-provider").is_empty());

        let ghost = FrameId::new("example-provider", "frame:never-sent", None);
        result.frame_attestations.push(FrameAttestation::signed(
            ghost.clone(),
            sample_attestation("sha256:00"),
        ));
        assert_eq!(
            result.orphaned_attestations("example-provider"),
            vec![&ghost],
            "evidence for a frame nobody was shown is not evidence"
        );
    }

    #[test]
    fn an_entry_carrying_neither_a_signature_nor_a_proof_asserts_nothing() {
        let entry = FrameAttestation {
            frame: FrameId::new("example-provider", "frame:a", None),
            attestation: None,
            inclusion_proof: None,
        };
        assert!(!entry.carries_evidence());
        let result = ContextQueryResult {
            frames: vec![frame_with_cost("frame:a", 1)],
            truncated: false,
            dropped_estimate: None,
            frame_attestations: vec![entry],
            result_attestation: None,
        };
        assert!(
            !result.is_attested(),
            "a bare identity must not read as an attested answer"
        );
    }

    #[test]
    fn a_root_signed_set_needs_no_per_frame_signature() {
        // The cheapest honest shape: one signature over the root, one proof per
        // frame, no per-frame signatures. If `attestation` were required this
        // would be unrepresentable and a provider would sign n times to say
        // what one signature says.
        let entry = FrameAttestation::proven(
            FrameId::new("example-provider", "frame:a", None),
            InclusionProof {
                leaf_index: 0,
                leaf_count: 2,
                path: vec![InclusionStep {
                    sibling: format!("sha256:{}", "44".repeat(32)),
                    sibling_is_left: false,
                }],
            },
        );
        assert!(entry.carries_evidence());
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("\"attestation\""),
            "an absent per-frame signature must be omitted, not null: {json}"
        );
        let back: FrameAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }
}
