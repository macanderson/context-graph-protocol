//! Stable frame identity and the canonical composition order
//! (`docs/context-reuse.md` §1).
//!
//! A [`FrameId`] is the triple `(provider id, frame id, content digest)` that
//! names a frame's *exact bytes*. It is the spine the context-reuse guarantees
//! share: deterministic composition orders frames by it (§1), a usage report
//! references served frames by it (§2), and a `context/verify` request carries
//! it so a provider can answer "is this still valid?" without any frame body
//! travelling (§4).
//!
//! The `content_digest` is **provider-declared and opaque**: a provider picks
//! the algorithm (the reference frames use `sha256:<hex>`, matching the
//! `provenance` digests) and the protocol never re-derives it. That is
//! deliberate — a host that computed the digest from its own serialization
//! would force every out-of-Rust provider to byte-exactly reproduce that
//! serialization just to answer a verify request, which is precisely the
//! lock-in the protocol exists to avoid. A frame that declares no digest
//! (`content_digest: None`) is simply *not verifiable*, and a host falls back
//! to re-querying it (§4).

use serde::{Deserialize, Serialize};

/// The stable identity of one frame's exact content bytes: `(provider id,
/// frame id, content digest)`.
///
/// The derived [`Ord`] is the protocol's **canonical composition order** —
/// fields compare in declaration order, i.e. by `provider_id`, then
/// `frame_id`, then `content_digest`. Sorting a frame set by [`FrameId`] is
/// what makes an unchanged set render byte-identically across turns and across
/// hosts (`docs/context-reuse.md` §1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FrameId {
    /// The host-facing id of the provider that served the frame — the same
    /// routing/consent key the host registered it under.
    pub provider_id: String,
    /// The provider-scoped frame id ([`ContextFrame::id`](crate::ContextFrame::id)),
    /// stable for dedup across queries.
    pub frame_id: String,
    /// The provider-declared digest of the frame's content bytes — opaque to
    /// the protocol (e.g. `sha256:<hex>`). `None` when the provider declared
    /// none: such a frame is not verifiable and a host re-queries it rather
    /// than trusting it stale (`docs/context-reuse.md` §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl FrameId {
    /// Build an identity from its parts.
    pub fn new(
        provider_id: impl Into<String>,
        frame_id: impl Into<String>,
        content_digest: Option<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            frame_id: frame_id.into(),
            content_digest,
        }
    }

    /// Whether this identity carries a content digest and can therefore be
    /// revalidated by a `context/verify` request. A frame without one is
    /// re-queried instead (`docs/context-reuse.md` §4).
    pub fn is_verifiable(&self) -> bool {
        self.content_digest.is_some()
    }
}

/// Sort a set of frame identities into the protocol's canonical composition
/// order (`docs/context-reuse.md` §1). A thin, explicit wrapper over the
/// derived [`Ord`] so call sites read as intent, not a bare `.sort()`.
pub fn canonical_order(ids: &mut [FrameId]) {
    ids.sort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_id_roundtrips_through_json() {
        let id = FrameId::new("repo-graph", "retry-doc", Some("sha256:9f2c".into()));
        let json = serde_json::to_string(&id).unwrap();
        let back: FrameId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn an_absent_digest_is_omitted_and_marks_the_frame_unverifiable() {
        let id = FrameId::new("repo-graph", "retry-doc", None);
        assert!(!id.is_verifiable());
        let json = serde_json::to_string(&id).unwrap();
        assert!(
            !json.contains("content_digest"),
            "an absent digest must be omitted, not serialized as null: {json}"
        );
        let back: FrameId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn canonical_order_is_by_provider_then_frame_then_digest() {
        let mut ids = vec![
            FrameId::new("b-provider", "frame-1", None),
            FrameId::new("a-provider", "frame-2", None),
            FrameId::new("a-provider", "frame-1", Some("sha256:zz".into())),
            FrameId::new("a-provider", "frame-1", Some("sha256:aa".into())),
        ];
        canonical_order(&mut ids);
        assert_eq!(
            ids,
            vec![
                // a-provider sorts before b-provider…
                FrameId::new("a-provider", "frame-1", Some("sha256:aa".into())),
                // …then by frame id, then by digest (aa before zz).
                FrameId::new("a-provider", "frame-1", Some("sha256:zz".into())),
                FrameId::new("a-provider", "frame-2", None),
                FrameId::new("b-provider", "frame-1", None),
            ]
        );
    }

    #[test]
    fn ordering_is_total_and_stable_regardless_of_input_order() {
        let canonical = {
            let mut ids = vec![
                FrameId::new("p", "c", None),
                FrameId::new("p", "a", None),
                FrameId::new("p", "b", None),
            ];
            canonical_order(&mut ids);
            ids
        };
        // Any permutation sorts to the same sequence.
        let mut shuffled = vec![
            FrameId::new("p", "b", None),
            FrameId::new("p", "c", None),
            FrameId::new("p", "a", None),
        ];
        canonical_order(&mut shuffled);
        assert_eq!(shuffled, canonical);
    }
}
