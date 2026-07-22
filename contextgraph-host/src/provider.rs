//! The uniform provider handle.
//!
//! Every context source — an in-process built-in, a child process over
//! stdio, or a remote HTTP endpoint — reaches the host through one trait,
//! [`ContextProvider`]. Its shape mirrors the two Context Graph Protocol methods a host always
//! needs: capability negotiation (cached from the handshake, SPEC.md §3) and
//! `context/query` (SPEC.md §5). `stella-context` and any other Rust agent drive
//! all three provider kinds through this single interface
//! (`SPEC.md` §1 — "usable by any other Rust agent that wants Context Graph Protocol
//! support").

use async_trait::async_trait;
use contextgraph_types::{
    Capabilities, ContextQuery, ContextQueryResult, FrameKind, ProviderInfo, Verdict,
    VerifyRequest, VerifyResponse,
};

use crate::error::HostError;

/// A registered Context Graph Protocol provider, queryable behind one handle regardless of
/// transport. `info()`/`capabilities()` return values cached at handshake
/// time, so they are cheap synchronous getters even for out-of-process
/// providers.
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// The provider's host-facing id — its routing key and its consent key
    /// (`SPEC.md` §4 and §10).
    fn id(&self) -> &str;

    /// Identity + declared data-flow direction, surfaced at consent time
    /// (SPEC.md §3, ).
    fn info(&self) -> &ProviderInfo;

    /// Capabilities negotiated at the handshake (SPEC.md §3) — which frame kinds
    /// and filters this provider serves, whether it upserts, does graph, is
    /// an embedder, or supports subscriptions.
    fn capabilities(&self) -> &Capabilities;

    /// Answer a context query with budgeted, provenance-carrying frames
    /// (SPEC.md §5). The host — not the provider — enforces the budget and consent;
    /// a provider that over-runs its budget is caught by the host, not
    /// trusted (`crate::host`).
    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError>;

    /// Revalidate frames the host already holds, without any frame body
    /// travelling (`docs/context-reuse.md` §4 `context/verify`).
    ///
    /// Defaults to answering [`Verdict::Unknown`](contextgraph_types::Verdict::Unknown)
    /// for every requested identity, so an existing provider implements
    /// nothing and is simply treated as unable to vouch for its frames — the
    /// host then re-queries them. A provider that overrides this **MUST** also
    /// advertise [`Capabilities::verify`](contextgraph_types::Capabilities::verify),
    /// since the host only asks providers that declare support.
    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, HostError> {
        Ok(VerifyResponse::uniform(request, Verdict::Unknown))
    }

    /// Shut the provider down cleanly (SPEC.md §3 lifecycle). In-process providers
    /// default to a no-op; transport-backed providers send `shutdown` and
    /// reap their child. Overridable.
    async fn shutdown(&self) -> Result<(), HostError> {
        Ok(())
    }
}

/// The snake_case wire name of a [`FrameKind`], matching its `serde`
/// representation and the strings a provider lists in
/// [`Capabilities::query`]'s `kinds`.
pub fn frame_kind_name(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::Snippet => "snippet",
        FrameKind::Symbol => "symbol",
        FrameKind::Fact => "fact",
        FrameKind::Doc => "doc",
        FrameKind::Memory => "memory",
        FrameKind::Episode => "episode",
        FrameKind::Graph => "graph",
    }
}

/// Whether a provider is worth querying for a given request. A query with no
/// `kinds` filter matches every provider (the host wants everyone's best
/// frames, SPEC.md §5); otherwise a provider matches when it declares at least one
/// of the requested frame kinds. Used by `query_all` to fan out only to
/// relevant providers.
pub fn capability_matches(caps: &Capabilities, query: &ContextQuery) -> bool {
    if query.kinds.is_empty() {
        return true;
    }
    query.kinds.iter().any(|requested| {
        let name = frame_kind_name(*requested);
        caps.query.kinds.iter().any(|served| served == name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::capability::QueryCapability;

    fn caps_for(kinds: &[&str]) -> Capabilities {
        Capabilities {
            query: QueryCapability {
                kinds: kinds.iter().map(|k| k.to_string()).collect(),
            },
            ..Capabilities::default()
        }
    }

    fn query_for(kinds: Vec<FrameKind>) -> ContextQuery {
        ContextQuery {
            goal: "g".into(),
            query_text: None,
            embedding: None,
            kinds,
            anchors: vec![],
            max_frames: 5,
            max_tokens: 1000,
            as_of: None,
            representation_preferences: vec![],
        }
    }

    #[test]
    fn frame_kind_names_match_serde_snake_case() {
        // The names a provider lists must be exactly the frames' serde names.
        for (kind, name) in [
            (FrameKind::Snippet, "snippet"),
            (FrameKind::Symbol, "symbol"),
            (FrameKind::Fact, "fact"),
            (FrameKind::Doc, "doc"),
            (FrameKind::Memory, "memory"),
            (FrameKind::Episode, "episode"),
            (FrameKind::Graph, "graph"),
        ] {
            assert_eq!(frame_kind_name(kind), name);
            let serde_name = serde_json::to_value(kind).unwrap();
            assert_eq!(serde_name, serde_json::Value::String(name.to_string()));
        }
    }

    #[test]
    fn an_empty_kind_filter_matches_every_provider() {
        let caps = caps_for(&["doc"]);
        assert!(capability_matches(&caps, &query_for(vec![])));
    }

    #[test]
    fn a_kind_filter_matches_only_overlapping_providers() {
        let doc_provider = caps_for(&["doc", "snippet"]);
        assert!(capability_matches(
            &doc_provider,
            &query_for(vec![FrameKind::Doc])
        ));
        assert!(capability_matches(
            &doc_provider,
            &query_for(vec![FrameKind::Fact, FrameKind::Snippet])
        ));
        assert!(!capability_matches(
            &doc_provider,
            &query_for(vec![FrameKind::Memory, FrameKind::Episode])
        ));
    }
}
