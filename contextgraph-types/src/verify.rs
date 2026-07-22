//! `context/verify` — pull-based revalidation of frames a host already holds
//! (`docs/context-reuse.md` §4).
//!
//! Deterministic composition (§1) makes reusing context *cheap*: an unchanged
//! frame set renders byte-identically and rides the provider's prompt cache.
//! But cheap reuse is only safe if the frames are still **true**. Without a way
//! to ask, a host faces a bad pair of choices at every turn boundary: re-query
//! everything (paying tokens and latency, and destroying the very prefix
//! stability §1 bought) or reuse silently and risk citing evidence that changed
//! underneath it.
//!
//! A verify exchange is the cheap third option. The host sends a batch of
//! [`FrameId`]s — `(provider id, frame id, content digest)` — and the provider
//! answers one [`Verdict`] per frame. **No frame body travels in either
//! direction.** That is the whole economic point: verification costs *bytes*,
//! not *tokens*, so a host can afford to do it every turn on frames it would
//! otherwise have re-queried in full.
//!
//! The digest is the ground truth. A provider compares the digest the host
//! presents against the digest its source has *now*: equal ⇒ [`Valid`](Verdict::Valid),
//! different ⇒ [`Stale`](Verdict::Stale), source gone ⇒ [`Gone`](Verdict::Gone),
//! can't tell ⇒ [`Unknown`](Verdict::Unknown). Because the digest is
//! provider-declared and opaque (§1), the provider is the only party that can
//! answer — which is exactly why §4's conformance case exists to hold it honest.
//!
//! **Verify is the pull counterpart to subscribe (#6), not a replacement.** A
//! provider that can watch its sources pushes invalidations; one that cannot —
//! a stateless HTTP endpoint, a batch-rebuilt index — can still answer a
//! question asked of it. Both are capability-gated, and a host that has neither
//! falls back to re-querying.

use serde::{Deserialize, Serialize};

use crate::identity::FrameId;

/// A host's request to revalidate frames it already holds
/// (`docs/context-reuse.md` §4).
///
/// Carries identities only — never frame bodies. Every identity in one request
/// belongs to the provider it is sent to; a host holding frames from several
/// providers sends one request each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyRequest {
    /// The frame identities to revalidate. A host **SHOULD** only include
    /// identities that carry a digest ([`FrameId::is_verifiable`]) — a
    /// digest-less frame cannot be revalidated and is re-queried instead.
    #[serde(default)]
    pub frames: Vec<FrameId>,
}

impl VerifyRequest {
    /// A request for the given identities.
    pub fn new(frames: Vec<FrameId>) -> Self {
        Self { frames }
    }

    /// How many identities this request asks about.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// A provider's answer for one frame (`docs/context-reuse.md` §4).
///
/// Serializes as an internally-tagged object keyed on `status`, so a verdict is
/// self-describing on the wire and gains variants without breaking parsers:
/// `{"status": "valid"}`, `{"status": "stale", "replacement_digest": "sha256:…"}`,
/// `{"status": "gone"}`, `{"status": "unknown"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verdict {
    /// The frame's content is unchanged: the digest the host presented matches
    /// the provider's current digest for that frame. The host **MAY** keep
    /// reusing the body it already holds.
    Valid,
    /// The frame still exists but its content changed — the presented digest no
    /// longer matches. The host **MUST NOT** keep serving the body it holds.
    ///
    /// `replacement_digest` is the provider's *current* digest for the frame,
    /// offered so a host can tell "changed again since I last looked" from
    /// "changed to something I already fetched" without a round trip. It is
    /// optional: a provider that knows the content differs but not what it is
    /// now still answers `stale` honestly. **It is a digest, never a body** —
    /// the host re-queries if it wants the new content.
    Stale {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replacement_digest: Option<String>,
    },
    /// The frame no longer exists — the source was deleted, or the provider no
    /// longer serves it. The host **MUST** drop it, and re-querying *this
    /// identity* is pointless.
    Gone,
    /// The provider cannot say. It may never have served this frame, may have
    /// lost the history needed to compare digests (a rebuilt index), or may not
    /// recognize the identity. The host **MUST NOT** treat this as validity —
    /// an unverifiable frame is re-queried, never reused on a shrug.
    Unknown,
}

impl Verdict {
    /// Whether a host may keep reusing the frame body it already holds. **Only**
    /// [`Valid`](Self::Valid) — reuse requires a positive answer, never the
    /// absence of a negative one (`docs/context-reuse.md` §4, requirement V2).
    pub fn permits_reuse(&self) -> bool {
        matches!(self, Self::Valid)
    }

    /// Whether re-querying the provider could recover usable content. True for
    /// [`Stale`](Self::Stale) (content exists, it changed) and
    /// [`Unknown`](Self::Unknown) (the provider couldn't say, so ask properly).
    /// False for [`Gone`](Self::Gone) — the frame is not there to re-fetch — and
    /// for [`Valid`](Self::Valid), which needs no re-query at all.
    pub fn warrants_requery(&self) -> bool {
        matches!(self, Self::Stale { .. } | Self::Unknown)
    }

    /// The verdict's wire name, for reports and log lines.
    pub fn status(&self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Stale { .. } => "stale",
            Self::Gone => "gone",
            Self::Unknown => "unknown",
        }
    }
}

/// One frame's verdict, paired with the identity it answers
/// (`docs/context-reuse.md` §4).
///
/// The identity is **echoed in full** rather than implied by position: a host
/// correlates verdicts by matching `frame`, so a provider that reorders,
/// omits, or duplicates entries can't silently shift a `valid` onto the wrong
/// frame. Entries a host didn't ask about are ignored; identities that come
/// back with no entry are treated as [`Unknown`](Verdict::Unknown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameVerdict {
    /// The identity being answered — echoed from the request.
    pub frame: FrameId,
    /// The provider's answer for it.
    #[serde(flatten)]
    pub verdict: Verdict,
}

impl FrameVerdict {
    /// Pair an identity with its verdict.
    pub fn new(frame: FrameId, verdict: Verdict) -> Self {
        Self { frame, verdict }
    }
}

/// A provider's answer to a [`VerifyRequest`] (`docs/context-reuse.md` §4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyResponse {
    /// One verdict per frame the provider is answering for.
    #[serde(default)]
    pub verdicts: Vec<FrameVerdict>,
}

impl VerifyResponse {
    /// A response carrying the given verdicts.
    pub fn new(verdicts: Vec<FrameVerdict>) -> Self {
        Self { verdicts }
    }

    /// Answer every requested identity with the same verdict — the shape a
    /// provider without real verification support returns
    /// ([`Unknown`](Verdict::Unknown)), and a convenience for tests.
    pub fn uniform(request: &VerifyRequest, verdict: Verdict) -> Self {
        Self {
            verdicts: request
                .frames
                .iter()
                .map(|frame| FrameVerdict::new(frame.clone(), verdict.clone()))
                .collect(),
        }
    }

    /// The verdict for one identity, matched on the **full** identity rather
    /// than position. `None` when the provider returned no entry for it — which
    /// a host treats as [`Unknown`](Verdict::Unknown).
    pub fn verdict_for(&self, frame: &FrameId) -> Option<&Verdict> {
        self.verdicts
            .iter()
            .find(|entry| &entry.frame == frame)
            .map(|entry| &entry.verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(frame: &str, digest: Option<&str>) -> FrameId {
        FrameId::new("repo-graph", frame, digest.map(String::from))
    }

    #[test]
    fn a_request_carries_identities_and_no_frame_bodies() {
        let request = VerifyRequest::new(vec![
            id("retry-doc", Some("sha256:9f2c")),
            id("timeout-doc", Some("sha256:aa01")),
        ]);
        let json = serde_json::to_string(&request).unwrap();
        // The economic guarantee of §4: verification costs bytes, not tokens.
        assert!(
            !json.contains("content\"") && !json.contains("title"),
            "a verify request must never carry frame bodies: {json}"
        );
        let back: VerifyRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn every_verdict_round_trips_through_its_tagged_wire_form() {
        for (verdict, status) in [
            (Verdict::Valid, "valid"),
            (
                Verdict::Stale {
                    replacement_digest: None,
                },
                "stale",
            ),
            (
                Verdict::Stale {
                    replacement_digest: Some("sha256:beef".into()),
                },
                "stale",
            ),
            (Verdict::Gone, "gone"),
            (Verdict::Unknown, "unknown"),
        ] {
            let json = serde_json::to_string(&verdict).unwrap();
            assert!(
                json.contains(&format!("\"status\":\"{status}\"")),
                "verdict must be tagged on `status`: {json}"
            );
            assert_eq!(verdict.status(), status);
            let back: Verdict = serde_json::from_str(&json).unwrap();
            assert_eq!(back, verdict);
        }
    }

    #[test]
    fn a_stale_verdict_without_a_replacement_omits_the_field() {
        let json = serde_json::to_string(&Verdict::Stale {
            replacement_digest: None,
        })
        .unwrap();
        assert!(
            !json.contains("replacement_digest"),
            "an absent replacement must be omitted, not null: {json}"
        );
        // A provider that knows content changed but not what it is now is still
        // answering honestly.
        let back: Verdict = serde_json::from_str(&json).unwrap();
        assert!(!back.permits_reuse());
        assert!(back.warrants_requery());
    }

    #[test]
    fn a_stale_verdict_carries_a_replacement_digest_but_never_a_body() {
        let verdict = Verdict::Stale {
            replacement_digest: Some("sha256:beef".into()),
        };
        let json = serde_json::to_string(&verdict).unwrap();
        assert!(json.contains("sha256:beef"));
        let back: Verdict = serde_json::from_str(&json).unwrap();
        assert_eq!(back, verdict);
    }

    #[test]
    fn only_valid_permits_reuse() {
        // The default-deny rule (V2): reuse needs a positive answer, never the
        // mere absence of a negative one.
        assert!(Verdict::Valid.permits_reuse());
        for verdict in [
            Verdict::Stale {
                replacement_digest: None,
            },
            Verdict::Gone,
            Verdict::Unknown,
        ] {
            assert!(
                !verdict.permits_reuse(),
                "{} must not permit reuse",
                verdict.status()
            );
        }
    }

    #[test]
    fn gone_is_the_one_verdict_that_does_not_warrant_a_requery() {
        // Nothing to re-fetch: the frame is not there anymore.
        assert!(!Verdict::Gone.warrants_requery());
        assert!(
            Verdict::Stale {
                replacement_digest: None
            }
            .warrants_requery()
        );
        assert!(Verdict::Unknown.warrants_requery());
        // A valid frame needs no re-query — that is the whole point.
        assert!(!Verdict::Valid.warrants_requery());
    }

    #[test]
    fn a_response_round_trips_and_flattens_the_verdict_into_each_entry() {
        let response = VerifyResponse::new(vec![
            FrameVerdict::new(id("retry-doc", Some("sha256:9f2c")), Verdict::Valid),
            FrameVerdict::new(
                id("timeout-doc", Some("sha256:aa01")),
                Verdict::Stale {
                    replacement_digest: Some("sha256:bb02".into()),
                },
            ),
        ]);
        let json = serde_json::to_string(&response).unwrap();
        // `verdict` is flattened, so an entry reads as one flat object.
        assert!(
            !json.contains("\"verdict\""),
            "verdict must flatten: {json}"
        );
        let back: VerifyResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn a_verdict_is_correlated_by_full_identity_not_by_position() {
        // A provider that reorders its answers must not shift a `valid` onto
        // the wrong frame.
        let valid_frame = id("retry-doc", Some("sha256:9f2c"));
        let stale_frame = id("timeout-doc", Some("sha256:aa01"));
        let response = VerifyResponse::new(vec![
            FrameVerdict::new(stale_frame.clone(), Verdict::Gone),
            FrameVerdict::new(valid_frame.clone(), Verdict::Valid),
        ]);
        assert_eq!(response.verdict_for(&valid_frame), Some(&Verdict::Valid));
        assert_eq!(response.verdict_for(&stale_frame), Some(&Verdict::Gone));
    }

    #[test]
    fn a_verdict_for_a_different_digest_does_not_match_the_asked_identity() {
        // The digest is part of the identity: an answer about other bytes is an
        // answer about a different question.
        let asked = id("retry-doc", Some("sha256:9f2c"));
        let response = VerifyResponse::new(vec![FrameVerdict::new(
            id("retry-doc", Some("sha256:0000")),
            Verdict::Valid,
        )]);
        assert_eq!(
            response.verdict_for(&asked),
            None,
            "a verdict about a different digest must not answer for this one"
        );
    }

    #[test]
    fn a_uniform_response_answers_every_requested_identity() {
        // The shape a provider without verification support returns.
        let request = VerifyRequest::new(vec![
            id("retry-doc", Some("sha256:9f2c")),
            id("timeout-doc", Some("sha256:aa01")),
        ]);
        let response = VerifyResponse::uniform(&request, Verdict::Unknown);
        assert_eq!(response.verdicts.len(), 2);
        for frame in &request.frames {
            assert_eq!(response.verdict_for(frame), Some(&Verdict::Unknown));
        }
    }
}
