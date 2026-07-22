//! Deterministic context composition (`docs/context-reuse.md` §1).
//!
//! Provider prompt caches (Anthropic's 0.1× cache reads, OpenAI's and
//! Gemini's automatic prefix caching) reward a **byte-stable prompt prefix**,
//! and retrieved context is the part of a prompt most likely to destroy that
//! stability: a host that re-queries every turn and pastes frames in arrival
//! order emits a different prefix each turn, silently forfeiting the cache and
//! multiplying the very token costs this protocol exists to make honest.
//!
//! [`compose_context`] is the reference answer. It renders a frame set into a
//! block that is a pure function of the frames' **content identity**:
//!
//! - frames are emitted in the protocol's canonical order — sorted by
//!   [`FrameId`](contextgraph_types::FrameId), i.e. by `(provider id, frame
//!   id, content digest)` — so the same set renders byte-identically across
//!   turns *and* across hosts;
//! - the per-frame rendering excludes `score` (query-dependent relevance) and
//!   `token_cost` (a derived quantity), so a re-query that only re-ranks the
//!   same frames does not bust the cached prefix;
//! - identical identities are de-duplicated, so a frame served by two queries
//!   contributes one block, not two.
//!
//! Frame `content` is untrusted data: it is emitted inside an explicit
//! `<frame>…</frame>` fence as quoted material, never as instructions
//! (`docs/protocol-surface.md` R3). Hardened injection-resistant delimiting
//! (an unguessable fence, dedup-by-content, budget packing) is the reference
//! *composition module*'s job (issue #15); this function is the narrower
//! **determinism contract** any composition — reference or not — can satisfy.

use contextgraph_types::{ContextFrame, FrameId};

use crate::provider::frame_kind_name;

/// Render a set of `(provider id, frame)` pairs into a byte-stable context
/// block (`docs/context-reuse.md` §1).
///
/// The output is deterministic: it depends only on the *set* of frames and
/// their content, never on iteration order, and re-rendering the same set
/// yields identical bytes. Passing the same set with fluctuating `score`s
/// yields the same bytes too — relevance is not part of a frame's rendered
/// identity.
pub fn compose_context<'a, I>(frames: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a ContextFrame)>,
{
    // Pair each frame with its canonical identity, then order by it. Sorting
    // the identities *is* the canonical ordering rule (§1).
    let mut blocks: Vec<(FrameId, String)> = frames
        .into_iter()
        .map(|(provider_id, frame)| {
            (
                frame.identity(provider_id),
                render_frame(provider_id, frame),
            )
        })
        .collect();
    blocks.sort_by(|(a, _), (b, _)| a.cmp(b));
    // Identical identity ⇒ identical bytes: collapse duplicates so a frame
    // served twice contributes a single block.
    blocks.dedup_by(|(a, _), (b, _)| a == b);

    let mut rendered = String::new();
    for (_, block) in &blocks {
        rendered.push_str(block);
    }
    rendered
}

/// Render one frame as a fixed, delimited block. Deliberately excludes `score`
/// and `token_cost` so the bytes track only the frame's content identity
/// (`docs/context-reuse.md` §1).
fn render_frame(provider_id: &str, frame: &ContextFrame) -> String {
    // Cite by the human label, never a bare id (whole-protocol convention).
    let cite = frame
        .citation_label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(&frame.title);
    format!(
        "<frame provider=\"{provider}\" id=\"{id}\" kind=\"{kind}\" cite=\"{cite}\">\n{content}\n</frame>\n",
        provider = provider_id,
        id = frame.id,
        kind = frame_kind_name(frame.kind),
        // A `reference` frame carries no inline content — it must be resolved
        // (`context/resolve`, a later phase) before composition; here it renders
        // as empty rather than fabricating bytes.
        content = frame.content.as_deref().unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::FrameKind;

    fn frame(id: &str, content: &str, digest: Option<&str>) -> ContextFrame {
        ContextFrame {
            id: id.into(),
            kind: FrameKind::Doc,
            title: id.into(),
            content: Some(content.into()),
            content_digest: digest.map(Into::into),
            uri: None,
            representation: Default::default(),
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score: 0.5,
            token_cost: 10,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![],
            citation_label: Some(format!("{id} cite")),
            embedding: None,
            relations: vec![],
        }
    }

    #[test]
    fn same_frame_set_renders_byte_identically_twice() {
        let a = frame("a", "alpha", Some("sha256:a"));
        let b = frame("b", "beta", Some("sha256:b"));
        let set = [("p", &a), ("p", &b)];
        let first = compose_context(set);
        let second = compose_context(set);
        assert_eq!(
            first, second,
            "composition must be a pure function of the set"
        );
        assert!(!first.is_empty());
    }

    #[test]
    fn input_order_does_not_change_the_rendering() {
        let a = frame("a", "alpha", Some("sha256:a"));
        let b = frame("b", "beta", Some("sha256:b"));
        let c = frame("c", "gamma", Some("sha256:c"));
        let forward = compose_context([("p", &a), ("p", &b), ("p", &c)]);
        let shuffled = compose_context([("p", &c), ("p", &a), ("p", &b)]);
        assert_eq!(
            forward, shuffled,
            "canonical ordering must make the rendering independent of arrival order"
        );
    }

    #[test]
    fn canonical_order_is_by_provider_then_frame_id() {
        let a = frame("a", "alpha", Some("sha256:a"));
        let z = frame("z", "zeta", Some("sha256:z"));
        // Register providers/frames out of order; the rendering sorts them.
        let rendered = compose_context([("prov-b", &a), ("prov-a", &z)]);
        let prov_a = rendered.find("provider=\"prov-a\"").unwrap();
        let prov_b = rendered.find("provider=\"prov-b\"").unwrap();
        assert!(prov_a < prov_b, "prov-a must render before prov-b");
    }

    #[test]
    fn relevance_and_cost_are_not_part_of_the_rendered_bytes() {
        // The whole point of prefix-stability: a re-query that only re-ranks
        // the same frames must not change the composed bytes.
        let base = frame("a", "alpha", Some("sha256:a"));
        let mut reranked = base.clone();
        reranked.score = 0.99;
        reranked.token_cost = 4096;
        assert_eq!(
            compose_context([("p", &base)]),
            compose_context([("p", &reranked)]),
            "changing only score/token_cost must not change the rendering"
        );
    }

    #[test]
    fn identical_identities_are_deduplicated() {
        let a = frame("a", "alpha", Some("sha256:a"));
        let again = a.clone();
        let rendered = compose_context([("p", &a), ("p", &again)]);
        assert_eq!(
            rendered.matches("id=\"a\"").count(),
            1,
            "a frame served twice must contribute a single block"
        );
    }

    #[test]
    fn content_is_fenced_as_quoted_material() {
        let a = frame("a", "untrusted payload", Some("sha256:a"));
        let rendered = compose_context([("p", &a)]);
        assert!(rendered.contains("<frame provider=\"p\" id=\"a\""));
        assert!(rendered.contains("untrusted payload"));
        assert!(rendered.contains("</frame>"));
    }
}
