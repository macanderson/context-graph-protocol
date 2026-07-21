//! `ContextFrame` — the unit of exchange between an Context Graph Protocol host and a provider.
//! `docs/specs/stella-rust-cli/06-context-protocol.md` §3.4 fixes this exact
//! shape; frames, never blobs, carry relevance, cost, and provenance so a
//! budgeting, citing host can compose sources honestly.

use serde::{Deserialize, Serialize};

use crate::token::budget_tokens;
use crate::validate::{is_protocol_timestamp, is_well_formed_digest};

/// What kind of thing a frame represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    Snippet,
    Symbol,
    Fact,
    Doc,
    Memory,
    Episode,
    Graph,
}

/// One link in a frame's provenance chain, ordered closest-to-source first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// e.g. "file", "derivation", "episode".
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

impl Provenance {
    /// Whether this link addresses bytes a host could independently re-read.
    pub fn is_file_provenance(&self) -> bool {
        self.kind == "file"
    }

    /// Whether the digest, if present, matches the grammar in `SPEC.md` §F5.
    ///
    /// Absent counts as *not* well-formed: for file provenance the digest is
    /// what makes tamper-detection possible at all, and treating "no digest"
    /// as acceptable is how the guarantee stayed decorative for so long.
    pub fn has_well_formed_digest(&self) -> bool {
        self.digest.as_deref().is_some_and(is_well_formed_digest)
    }
}

/// The recommended relation vocabulary (`SPEC.md` §Graph).
///
/// `Relation.rel` is an **open** vocabulary — a provider may emit any string,
/// and a host must not reject an unknown one. These constants exist so that
/// independent providers converge on the same spelling for the same edge
/// instead of each inventing `calls` / `call` / `code.call`. Using them is
/// SHOULD-level, not MUST.
///
/// Namespacing is the part that matters: a provider-specific edge belongs
/// under its own prefix (`myindex.owns`), which keeps the shared namespace
/// meaningful and makes a future registry possible.
pub mod rel {
    /// The subject calls the target.
    pub const CODE_CALLS: &str = "code.calls";
    /// The subject imports the target.
    pub const CODE_IMPORTS: &str = "code.imports";
    /// The subject defines the target.
    pub const CODE_DEFINES: &str = "code.defines";
    /// The subject references the target without calling it.
    pub const CODE_REFERENCES: &str = "code.references";
    /// The subject documents the target.
    pub const DOC_DOCUMENTS: &str = "doc.documents";
    /// The subject episode follows the target episode in time.
    pub const EPISODE_FOLLOWS: &str = "episode.follows";

    /// Every relation this revision names. A registry, not a restriction.
    pub const RECOMMENDED: &[&str] = &[
        CODE_CALLS,
        CODE_IMPORTS,
        CODE_DEFINES,
        CODE_REFERENCES,
        DOC_DOCUMENTS,
        EPISODE_FOLLOWS,
    ];
}

/// A graph relation a frame participates in, surfaced with a human label —
/// raw ids are never the primary identifier (`SPEC.md` §G1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    /// The edge label. See [`rel`] for the recommended vocabulary; unknown
    /// values are valid and **MUST NOT** be rejected by a host.
    pub rel: String,
    pub target_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl Relation {
    /// Whether this edge can be surfaced to a human by name.
    ///
    /// The "never a raw id" rule has been documented since the protocol's first
    /// draft and was checked by nothing; `SPEC.md` §G1 now makes it a
    /// conformance requirement for graph-capable providers, and this is the
    /// predicate behind it.
    pub fn has_display_name(&self) -> bool {
        self.display_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    }

    /// Whether `rel` uses the recommended vocabulary. Advisory only — a `false`
    /// here is a hint for a provider author, never a conformance failure.
    pub fn uses_recommended_vocabulary(&self) -> bool {
        rel::RECOMMENDED.contains(&self.rel.as_str())
    }
}

/// The optional embedding carried by a frame. The vector itself is
/// elidable — a host may want the fingerprint without the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameEmbedding {
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

/// One context frame returned from `context/query`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextFrame {
    /// Provider-scoped, stable for dedup across queries.
    pub id: String,
    pub kind: FrameKind,
    /// Human label — never a bare uuid.
    pub title: String,
    /// Text the host may quote into a prompt. Untrusted data: a conforming
    /// host delimits this as quoted material, never as instructions.
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Provider-normalized relevance in `[0, 1]`.
    pub score: f32,
    /// Honest, conformance-audited token cost.
    pub token_cost: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<Provenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<FrameEmbedding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<Relation>,
}

impl ContextFrame {
    /// Score must be normalized into `[0, 1]` per the protocol contract.
    /// Conformance suites assert this; providers should self-check too.
    pub fn has_valid_score(&self) -> bool {
        (0.0..=1.0).contains(&self.score)
    }

    /// The cost this frame's content is *required* to declare
    /// (`SPEC.md` §B3) — see [`budget_tokens`](crate::budget_tokens).
    pub fn canonical_token_cost(&self) -> u32 {
        budget_tokens(&self.content)
    }

    /// Whether `token_cost` matches the canonical count for this frame's
    /// content.
    ///
    /// This is the check that turned budget honesty from arithmetic into
    /// truth: previously a provider could declare `token_cost: 1` on a
    /// ten-thousand-token frame and pass every check in the suite.
    pub fn declares_honest_token_cost(&self) -> bool {
        self.token_cost == self.canonical_token_cost()
    }

    /// The names of any temporal fields that are not in the protocol's
    /// timestamp profile (`SPEC.md` §F4).
    ///
    /// Returns the field *names* rather than a bare bool so a conformance
    /// failure can say which field was wrong — an evidence string reading
    /// "valid_from" is actionable in a way that "temporal validation failed"
    /// is not.
    pub fn invalid_temporal_fields(&self) -> Vec<&'static str> {
        [
            ("valid_from", self.valid_from.as_deref()),
            ("valid_to", self.valid_to.as_deref()),
            ("recorded_at", self.recorded_at.as_deref()),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_some_and(|v| !is_protocol_timestamp(v)))
        .map(|(name, _)| name)
        .collect()
    }

    /// Whether every temporal field present on this frame is well-formed.
    pub fn has_valid_temporal_fields(&self) -> bool {
        self.invalid_temporal_fields().is_empty()
    }

    /// Provenance entries that address a file but carry a malformed or missing
    /// digest (`SPEC.md` §F5).
    ///
    /// File provenance is held to a stricter standard than other kinds because
    /// it is the one the host can independently verify: the bytes are on disk.
    /// A `derivation` or `episode` link has no addressable bytes, so requiring
    /// a digest of it would be theatre.
    pub fn provenance_with_unusable_digests(&self) -> Vec<usize> {
        self.provenance
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_file_provenance() && !p.has_well_formed_digest())
            .map(|(index, _)| index)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> ContextFrame {
        ContextFrame {
            id: "frm_1".into(),
            kind: FrameKind::Snippet,
            title: "workspace.ts L120-160".into(),
            content: "export interface Workspace { ... }".into(),
            uri: Some("file:///repo/workspace.ts".into()),
            score: 0.83,
            token_cost: 412,
            valid_from: None,
            valid_to: None,
            recorded_at: Some("2026-07-10T00:00:00Z".into()),
            provenance: vec![Provenance {
                kind: "file".into(),
                uri: Some("file:///repo/workspace.ts".into()),
                range: Some("L120-160".into()),
                digest: Some("sha256:abc".into()),
                method: None,
                by: None,
            }],
            citation_label: Some("workspace.ts L120-160".into()),
            embedding: None,
            relations: vec![],
        }
    }

    #[test]
    fn context_frame_roundtrips_through_json() {
        let frame = sample_frame();
        let json = serde_json::to_string(&frame).unwrap();
        let back: ContextFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back, frame);
    }

    #[test]
    fn score_out_of_range_fails_the_conformance_check() {
        let mut frame = sample_frame();
        assert!(frame.has_valid_score());
        frame.score = 1.5;
        assert!(!frame.has_valid_score());
    }

    #[test]
    fn an_honest_frame_declares_the_canonical_cost_of_its_content() {
        let mut frame = sample_frame();
        frame.content = "abcd".repeat(10).into(); // 40 bytes -> 10 budget tokens
        frame.token_cost = 10;
        assert!(frame.declares_honest_token_cost());
        assert_eq!(frame.canonical_token_cost(), 10);
    }

    #[test]
    fn the_budget_lie_that_used_to_pass_every_check_is_now_caught() {
        // Issue #8's headline case: a provider reporting `token_cost: 1` on a
        // huge frame satisfied `sum(token_cost) <= max_tokens` perfectly.
        let mut frame = sample_frame();
        frame.content = "x".repeat(10_000);
        frame.token_cost = 1;
        assert!(!frame.declares_honest_token_cost());
        assert_eq!(frame.canonical_token_cost(), 2_500);
    }

    #[test]
    fn over_reporting_cost_is_a_lie_too_even_though_it_is_self_harming() {
        // Exact equality, not an upper bound: an inflated cost would let a
        // provider crowd honest peers out of a shared budget.
        let mut frame = sample_frame();
        frame.content = "abcd".into();
        frame.token_cost = 500;
        assert!(!frame.declares_honest_token_cost());
    }

    #[test]
    fn malformed_temporal_fields_are_reported_by_name() {
        let mut frame = sample_frame();
        frame.valid_from = Some("last tuesday".into());
        frame.valid_to = Some("2026-08-01T00:00:00Z".into());
        frame.recorded_at = Some("2026-07-10".into());

        // The names are what make a conformance failure actionable.
        assert_eq!(
            frame.invalid_temporal_fields(),
            vec!["valid_from", "recorded_at"]
        );
        assert!(!frame.has_valid_temporal_fields());
    }

    #[test]
    fn absent_temporal_fields_are_valid_because_they_are_optional() {
        let mut frame = sample_frame();
        frame.valid_from = None;
        frame.valid_to = None;
        frame.recorded_at = None;
        assert!(frame.has_valid_temporal_fields());
    }

    #[test]
    fn file_provenance_without_a_usable_digest_is_flagged_by_index() {
        let mut frame = sample_frame();
        // `sha256:abc` is the placeholder the pre-spec fixtures used.
        assert_eq!(frame.provenance_with_unusable_digests(), vec![0]);

        frame.provenance[0].digest = Some(format!("sha256:{}", "a".repeat(64)));
        assert!(frame.provenance_with_unusable_digests().is_empty());
    }

    #[test]
    fn non_file_provenance_is_not_required_to_carry_a_digest() {
        // A derivation has no addressable bytes to digest, so demanding one
        // would be theatre rather than integrity.
        let mut frame = sample_frame();
        frame.provenance = vec![Provenance {
            kind: "derivation".into(),
            uri: None,
            range: None,
            digest: None,
            method: Some("summarized".into()),
            by: Some("contextgraph-docs".into()),
        }];
        assert!(frame.provenance_with_unusable_digests().is_empty());
    }

    #[test]
    fn a_graph_edge_must_be_citable_by_a_human_label() {
        let edge = Relation {
            rel: rel::CODE_CALLS.into(),
            target_uri: "file:///repo/src/net.rs#retry".into(),
            display_name: Some("net::retry".into()),
        };
        assert!(edge.has_display_name());
        assert!(edge.uses_recommended_vocabulary());

        // A raw id with no label is exactly what the "never a bare uuid" rule
        // forbids, and nothing checked it before.
        let unlabeled = Relation {
            rel: "myindex.owns".into(),
            target_uri: "file:///repo/src/net.rs".into(),
            display_name: None,
        };
        assert!(!unlabeled.has_display_name());
        // ...but an out-of-vocabulary `rel` is perfectly legal.
        assert!(!unlabeled.uses_recommended_vocabulary());
    }

    #[test]
    fn a_whitespace_only_display_name_does_not_count_as_a_label() {
        let edge = Relation {
            rel: rel::DOC_DOCUMENTS.into(),
            target_uri: "file:///docs/net.md".into(),
            display_name: Some("   ".into()),
        };
        assert!(!edge.has_display_name());
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let frame = sample_frame();
        let mut minimal = frame.clone();
        minimal.uri = None;
        minimal.valid_from = None;
        minimal.provenance.clear();
        let json = serde_json::to_string(&minimal).unwrap();
        assert!(!json.contains("\"uri\""));
        assert!(!json.contains("\"provenance\""));
    }
}
