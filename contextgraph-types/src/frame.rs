//! `ContextFrame` — the unit of exchange between an Context Graph Protocol host and a provider.
//! `docs/specs/stella-rust-cli/06-context-protocol.md` §3.4 fixes this exact
//! shape; frames, never blobs, carry relevance, cost, and provenance so a
//! budgeting, citing host can compose sources honestly.
//!
//! ## Frame representations (CGEP lifecycle, phase 2)
//!
//! A frame states how it carries its content through [`Representation`]:
//! `full` inlines the content (the legacy default), `compact` inlines a
//! transformed rendering alongside a resolver handle, and `reference` carries
//! no inline content at all — only a [`ContentRef`] and a
//! [`canonical_content_hash`](ContextFrame::canonical_content_hash) so a host
//! can rehydrate honestly and verifiably. `representation` absent means `full`,
//! so pre-representation providers and stored frames deserialize unchanged.

use serde::{Deserialize, Serialize};

use crate::identity::FrameId;

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

/// How a frame carries its content
/// (CGEP lifecycle build prompt, §"ContextFrame representations").
///
/// - `full`: canonical inline [`content`](ContextFrame::content) is required.
/// - `compact`: inline content, inline hash, canonical hash, a [`Transform`]
///   identity, and a [`ContentRef`] are all required.
/// - `reference`: inline content is **absent**; a [`ContentRef`] and
///   [`canonical_content_hash`](ContextFrame::canonical_content_hash) are
///   required; the inline content hash and transform are omitted.
///
/// A `full` frame omits this field on the wire ([`is_full`](Self::is_full)) so
/// legacy frames round-trip byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    #[default]
    Full,
    Compact,
    Reference,
}

impl Representation {
    /// Whether this is the legacy default representation. A `full` frame omits
    /// the `representation` field on the wire, so a frame emitted before this
    /// field existed round-trips unchanged and `representation` absent means
    /// `full`.
    pub fn is_full(&self) -> bool {
        matches!(self, Representation::Full)
    }
}

/// The fidelity of a frame's carried content relative to its canonical source.
/// A missing value means `exact` for a legacy full frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFidelity {
    Exact,
    Normalized,
    Summarized,
    Omitted,
}

/// Whether a frame's point of use requires the content inline, or accepts a
/// resolvable reference. Blocking constraints, guarded rules, ordered
/// procedures, and executable contracts require inline content at their point
/// of use; this keeps a reference choice from being confused with fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineContentRequirement {
    Required,
    ResolvableReferenceAllowed,
}

/// An opaque resolver handle for a `compact`/`reference` frame's content.
///
/// [`ContextFrame::uri`] identifies the source resource; `ContentRef::uri` is a
/// **distinct** opaque resolver handle. A `ContentRef` also names the exact
/// [`provider_id`](Self::provider_id) that returned it, so a fan-out host routes
/// resolution back to that provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRef {
    /// The exact provider that returned this frame; a fan-out host routes
    /// `context/resolve` back to it.
    pub provider_id: String,
    /// Opaque resolver handle, distinct from [`ContextFrame::uri`].
    pub uri: String,
    /// When the handle stops resolving. Absent ⇒ no declared expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// The transformation identity a `compact` frame applies to its source to
/// produce the inline rendering, so a consumer knows what it is reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// e.g. `extractive_summary`, `truncation`.
    pub method: String,
    /// e.g. `provider_default`, or a named implementation.
    pub implementation: String,
    pub version: String,
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

/// A graph relation a frame participates in, surfaced with a human label —
/// raw ids are never the primary identifier (§3.3 "display_name mandatory").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub rel: String,
    pub target_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
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
    ///
    /// Present for `full`/`compact` frames; **absent** for `reference` frames,
    /// which carry only a [`content_ref`](Self::content_ref). A reference is
    /// never encoded as `content: ""` — the field is omitted entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The provider-declared digest of this frame's **inline** content bytes —
    /// the third component of its stable [`FrameId`](crate::FrameId) identity,
    /// opaque to the protocol (e.g. `sha256:<hex>`). This is the spec's
    /// `content_hash` (SHA-256 over the exact inline UTF-8 content) under its
    /// established name; see [`canonical_content_hash`](Self::canonical_content_hash)
    /// for the full-source hash. Absent ⇒ the frame is not verifiable and a
    /// host re-queries it rather than reusing it unchecked
    /// (`docs/context-reuse.md` §1, §4). A `reference` frame omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// How this frame carries its content. Absent ⇒ [`Representation::Full`],
    /// so legacy frames deserialize unchanged and full frames omit the field.
    #[serde(default, skip_serializing_if = "Representation::is_full")]
    pub representation: Representation,
    /// Fidelity of the carried content relative to the source. Absent ⇒ `exact`
    /// for a legacy full frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_fidelity: Option<ContentFidelity>,
    /// SHA-256 over the **complete source content** bytes (distinct from the
    /// inline [`content_digest`](Self::content_digest)). Required for
    /// `compact`/`reference` frames so a resolved rehydration is verifiable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_content_hash: Option<String>,
    /// The opaque resolver handle for a `compact`/`reference` frame's content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<ContentRef>,
    /// The transformation a `compact` frame applied to its source. Omitted for
    /// `full`/`reference` frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// The lowest content fidelity acceptable at this frame's point of use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_content_fidelity: Option<ContentFidelity>,
    /// Whether this frame's point of use requires inline content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_content_requirement: Option<InlineContentRequirement>,
    /// Provider-normalized relevance in `[0, 1]`.
    pub score: f32,
    /// Honest, conformance-audited token cost of the **inline** rendering.
    pub token_cost: u32,
    /// Token cost of the complete canonical source content, when the provider
    /// declares it. If present, [`tokenizer_ref`](Self::tokenizer_ref) SHOULD
    /// name the tokenizer it was measured with. Hosts compute model-specific
    /// costs when providers omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_token_cost: Option<u32>,
    /// Identifies the tokenizer that produced the declared costs
    /// (e.g. `openai:o200k_base`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_ref: Option<String>,
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
    /// A `full`-representation frame carrying inline `content` — the shape every
    /// legacy provider emits. The representation/cost/resolver fields default to
    /// absent, so a call site need only supply the core, then set extras as
    /// needed (the build prompt asks for constructors to reduce source
    /// breakage).
    pub fn full(
        id: impl Into<String>,
        kind: FrameKind,
        title: impl Into<String>,
        content: impl Into<String>,
        score: f32,
        token_cost: u32,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            content: Some(content.into()),
            content_digest: None,
            uri: None,
            representation: Representation::Full,
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score,
            token_cost,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: Vec::new(),
            citation_label: None,
            embedding: None,
            relations: Vec::new(),
        }
    }

    /// A `reference`-representation frame: no inline content, only a resolver
    /// handle and the canonical source hash for honest, verifiable rehydration.
    /// `token_cost` is the inline cost (0 — nothing is inlined).
    pub fn reference(
        id: impl Into<String>,
        kind: FrameKind,
        title: impl Into<String>,
        content_ref: ContentRef,
        canonical_content_hash: impl Into<String>,
        score: f32,
    ) -> Self {
        Self {
            representation: Representation::Reference,
            content: None,
            content_ref: Some(content_ref),
            canonical_content_hash: Some(canonical_content_hash.into()),
            ..Self::full(id, kind, title, String::new(), score, 0)
        }
        // `..full(..)` seeds every other field; the overrides above make this a
        // structurally honest reference (content absent, content_digest None,
        // transform None) that satisfies `representation_invariants`.
    }

    /// Score must be normalized into `[0, 1]` per the protocol contract.
    /// Conformance suites assert this; providers should self-check too.
    pub fn has_valid_score(&self) -> bool {
        (0.0..=1.0).contains(&self.score)
    }

    /// The frame's stable identity under the given provider: `(provider id,
    /// frame id, content digest)`. The digest is carried through from
    /// [`content_digest`](Self::content_digest), so a frame without one yields
    /// an unverifiable identity (`docs/context-reuse.md` §1).
    pub fn identity(&self, provider_id: impl Into<String>) -> FrameId {
        FrameId::new(provider_id, self.id.clone(), self.content_digest.clone())
    }

    /// Whether this frame's fields satisfy the invariants of its declared
    /// [`representation`](Self::representation). Providers emit conforming
    /// frames; hosts reject a frame that lies about its shape (e.g. a
    /// `reference` carrying inline content, or a `compact` missing its
    /// canonical hash). The `Err` string names the exact violation.
    pub fn representation_invariants(&self) -> Result<(), String> {
        match self.representation {
            Representation::Full => {
                if self.content.is_none() {
                    return Err("full frame requires inline content".into());
                }
            }
            Representation::Compact => {
                if self.content.is_none() {
                    return Err("compact frame requires inline content".into());
                }
                if self.content_digest.is_none() {
                    return Err(
                        "compact frame requires an inline content hash (content_digest)".into(),
                    );
                }
                if self.canonical_content_hash.is_none() {
                    return Err("compact frame requires canonical_content_hash".into());
                }
                if self.transform.is_none() {
                    return Err("compact frame requires a transform identity".into());
                }
                if self.content_ref.is_none() {
                    return Err("compact frame requires content_ref".into());
                }
            }
            Representation::Reference => {
                // "Never encode a reference as content: \"\"" — any inline
                // content, empty or not, is a violation.
                if self.content.is_some() {
                    return Err("reference frame must not carry inline content".into());
                }
                if self.content_ref.is_none() {
                    return Err("reference frame requires content_ref".into());
                }
                if self.canonical_content_hash.is_none() {
                    return Err("reference frame requires canonical_content_hash".into());
                }
                if self.content_digest.is_some() {
                    return Err(
                        "reference frame must omit the inline content hash (content_digest)".into(),
                    );
                }
                if self.transform.is_some() {
                    return Err("reference frame must omit transform".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> ContextFrame {
        let mut frame = ContextFrame::full(
            "frm_1",
            FrameKind::Snippet,
            "workspace.ts L120-160",
            "export interface Workspace { ... }",
            0.83,
            412,
        );
        frame.content_digest = Some("sha256:abc".into());
        frame.uri = Some("file:///repo/workspace.ts".into());
        frame.recorded_at = Some("2026-07-10T00:00:00Z".into());
        frame.provenance = vec![Provenance {
            kind: "file".into(),
            uri: Some("file:///repo/workspace.ts".into()),
            range: Some("L120-160".into()),
            digest: Some("sha256:abc".into()),
            method: None,
            by: None,
        }];
        frame.citation_label = Some("workspace.ts L120-160".into());
        frame
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
    fn identity_carries_the_provider_scope_and_content_digest() {
        let frame = sample_frame();
        let id = frame.identity("repo-graph");
        assert_eq!(id.provider_id, "repo-graph");
        assert_eq!(id.frame_id, "frm_1");
        assert_eq!(id.content_digest.as_deref(), Some("sha256:abc"));
        assert!(id.is_verifiable());

        // A frame without a declared digest yields an unverifiable identity.
        let mut undigested = frame;
        undigested.content_digest = None;
        assert!(!undigested.identity("repo-graph").is_verifiable());
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let frame = sample_frame();
        let mut minimal = frame.clone();
        minimal.uri = None;
        minimal.valid_from = None;
        minimal.content_digest = None;
        minimal.provenance.clear();
        let json = serde_json::to_string(&minimal).unwrap();
        assert!(!json.contains("\"uri\""));
        assert!(!json.contains("\"provenance\""));
        assert!(!json.contains("\"content_digest\""));
    }

    #[test]
    fn full_frame_omits_representation_on_the_wire() {
        // A full frame keeps the legacy wire shape: `representation` is absent,
        // so pre-representation consumers see no new field.
        let frame = sample_frame();
        assert_eq!(frame.representation, Representation::Full);
        let json = serde_json::to_string(&frame).unwrap();
        assert!(
            !json.contains("representation"),
            "full frames must omit the representation field: {json}"
        );
        assert!(frame.representation_invariants().is_ok());
    }

    #[test]
    fn reference_frame_omits_content_and_round_trips_its_handle() {
        let frame = ContextFrame::reference(
            "frm_ref_1",
            FrameKind::Doc,
            "Deployment runbook",
            ContentRef {
                provider_id: "provider_example".into(),
                uri: "context://provider_example/records/doc_runbook_v1".into(),
                expires_at: None,
            },
            "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            0.9,
        );
        frame
            .representation_invariants()
            .expect("constructed reference frame must be structurally honest");

        let json = serde_json::to_string(&frame).unwrap();
        assert!(
            !json.contains("\"content\""),
            "a reference frame must not carry inline content: {json}"
        );
        assert!(json.contains("\"representation\":\"reference\""));

        let back: ContextFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back, frame);
        assert_eq!(back.representation, Representation::Reference);
        assert_eq!(
            back.content_ref.as_ref().unwrap().provider_id,
            "provider_example"
        );
    }

    #[test]
    fn a_reference_with_inline_content_violates_its_invariants() {
        let mut frame = ContextFrame::reference(
            "frm_ref_2",
            FrameKind::Doc,
            "Runbook",
            ContentRef {
                provider_id: "p".into(),
                uri: "context://p/r".into(),
                expires_at: None,
            },
            "sha256:aa",
            0.5,
        );
        // Even an empty string is a lie for a reference.
        frame.content = Some(String::new());
        assert!(frame.representation_invariants().is_err());
    }

    #[test]
    fn compact_frame_requires_its_full_metadata_set() {
        let mut frame = sample_frame();
        frame.representation = Representation::Compact;
        // full()-seeded frame lacks the compact metadata → invalid.
        assert!(frame.representation_invariants().is_err());

        frame.content_digest = Some("sha256:inline".into());
        frame.canonical_content_hash = Some("sha256:canonical".into());
        frame.transform = Some(Transform {
            method: "extractive_summary".into(),
            implementation: "provider_default".into(),
            version: "1".into(),
        });
        frame.content_ref = Some(ContentRef {
            provider_id: "provider_example".into(),
            uri: "context://provider_example/records/x".into(),
            expires_at: None,
        });
        frame.content = Some("summary…".into());
        assert!(frame.representation_invariants().is_ok());
    }
}
