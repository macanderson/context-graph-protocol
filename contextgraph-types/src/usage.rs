//! Usage reports — a per-request roll-up of context cost
//! (`docs/context-reuse.md` §2).
//!
//! Every frame carries an honest `token_cost` (§protocol-surface B1), but the
//! protocol otherwise stops at the frame. A host that meters context into a
//! billing system — the usage-events → ClickHouse → Stripe loop platforms run —
//! needs an *aggregate*: which providers served how many frames, at what token
//! cost, against which budget. Left unspecified, every host invents that shape
//! independently and context cost stays unauditable one level up from the wire
//! — the blob-pipe problem reborn at the accounting layer.
//!
//! A [`UsageReport`] is that aggregate. It is a **host-side artifact**, not a
//! wire message: it rides no new envelope variant and no new required field, so
//! a provider implements nothing to make one possible. The reference host
//! produces it from a fan-out
//! ([`FanOut::usage_report`](https://docs.rs/contextgraph-host/latest/contextgraph_host/host/struct.FanOut.html#method.usage_report)).
//!
//! Each served frame is recorded as a [`ServedFrame`]: its stable
//! [`FrameId`](crate::FrameId) identity *and* the `token_cost` the provider
//! declared for it. Storing the pair is what lets an auditor walk from a billed
//! total back to the exact frames behind it — and it makes the report
//! **self-verifying**: the per-provider and request totals re-sum from these
//! entries, so a corrupted total is a checkable arithmetic failure, not a
//! silent misbill (the conformance case in §2).

use serde::{Deserialize, Serialize};

use crate::identity::FrameId;

/// One served frame's contribution to a [`UsageReport`]: its stable identity
/// and the `token_cost` its provider declared for it.
///
/// The identity is the auditor's walk-back handle — from a billed line to the
/// exact `(provider id, frame id, content digest)` behind it — and the paired
/// `token_cost` is what the request total re-sums from, so the report needs no
/// out-of-band lookup to be checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedFrame {
    /// The frame's stable identity (`docs/context-reuse.md` §1).
    pub frame: FrameId,
    /// The `token_cost` the provider declared for this frame — the value that
    /// was budget-audited before the frame was accepted.
    pub token_cost: u32,
}

/// One provider's usage within a single query: how many frames it served, how
/// many were rejected, the summed token cost, and the served frames'
/// identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    /// The host-facing id of the provider (the routing/consent key).
    pub provider_id: String,
    /// Frames the host accepted from this provider (passed consent, timeout,
    /// and the budget-honesty audit). Equal to `served_frames.len()`.
    pub frames_served: u32,
    /// Frames the provider offered that the host rejected — a provider that
    /// blew the budget has its frames dropped as a `token_cost` lie
    /// (§protocol-surface B2), and once [verification](crate::verify) is wired
    /// a frame verified `stale`/`gone` is rejected too.
    pub frames_rejected: u32,
    /// Summed `token_cost` of the served frames — the provider's contribution
    /// to the request's consumed budget.
    pub token_cost: u64,
    /// The served frames, each by stable identity + declared cost, so an
    /// auditor can walk from this provider's total to the exact frames.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub served_frames: Vec<ServedFrame>,
}

impl ProviderUsage {
    /// Re-sum the served frames' declared costs. Equals [`token_cost`](Self::token_cost)
    /// for a consistently-built report — the arithmetic identity a metering
    /// pipeline checks before trusting the total (§2).
    pub fn served_token_cost(&self) -> u64 {
        self.served_frames
            .iter()
            .map(|served| served.token_cost as u64)
            .sum()
    }

    /// Whether this provider's aggregate agrees with its itemized frames:
    /// `frames_served == served_frames.len()` and
    /// `token_cost == served_token_cost()`.
    pub fn is_consistent(&self) -> bool {
        self.frames_served as usize == self.served_frames.len()
            && self.token_cost == self.served_token_cost()
    }
}

/// A per-request roll-up of context cost across every provider a query fanned
/// out to (`docs/context-reuse.md` §2).
///
/// This is the shape a host maps into a metering pipeline: one row per
/// `(request, provider)` with a frame-cited token total, plus the request-level
/// budget requested vs. consumed and the accounting snapshot time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReport {
    /// The query's `max_tokens` — the budget the host asked providers to
    /// respect.
    pub budget_requested: u32,
    /// The summed `token_cost` of every served frame across all providers —
    /// what the request actually consumed.
    pub budget_consumed: u64,
    /// The accounting snapshot time (RFC 3339), supplied by the host that
    /// produced the report. This is the *report's* as-of, distinct from a
    /// query's bi-temporal `as_of` retrieval pin.
    pub as_of: String,
    /// Per-provider usage, one entry per provider the query fanned out to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<ProviderUsage>,
}

impl UsageReport {
    /// Re-sum every provider's `token_cost`. Equals [`budget_consumed`](Self::budget_consumed)
    /// for a consistently-built report.
    pub fn total_provider_cost(&self) -> u64 {
        self.providers.iter().map(|p| p.token_cost).sum()
    }

    /// The number of frames served across all providers.
    pub fn total_frames_served(&self) -> u64 {
        self.providers.iter().map(|p| p.frames_served as u64).sum()
    }

    /// The core arithmetic identity a metering pipeline checks before trusting
    /// the report: the consumed total equals the summed cost of every served
    /// frame, and each provider's aggregate agrees with its itemized frames.
    /// A report that fails this is a corrupted total, never a silent misbill
    /// (§2 conformance).
    pub fn is_consistent(&self) -> bool {
        self.budget_consumed == self.total_provider_cost()
            && self.providers.iter().all(ProviderUsage::is_consistent)
    }

    /// Whether the request stayed within its requested budget. A conformant
    /// host drops budget-lying providers *before* they reach a report, so a
    /// report built by such a host always satisfies this.
    pub fn within_budget(&self) -> bool {
        self.budget_consumed <= self.budget_requested as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(provider: &str, frame: &str, digest: &str, cost: u32) -> ServedFrame {
        ServedFrame {
            frame: FrameId::new(provider, frame, Some(digest.into())),
            token_cost: cost,
        }
    }

    fn sample_report() -> UsageReport {
        let graph = ProviderUsage {
            provider_id: "repo-graph".into(),
            frames_served: 2,
            frames_rejected: 0,
            token_cost: 64,
            served_frames: vec![
                served("repo-graph", "retry-doc", "sha256:aa", 41),
                served("repo-graph", "retry-sym", "sha256:bb", 23),
            ],
        };
        let liar = ProviderUsage {
            provider_id: "cloud-docs".into(),
            frames_served: 0,
            frames_rejected: 3,
            token_cost: 0,
            served_frames: vec![],
        };
        UsageReport {
            budget_requested: 1024,
            budget_consumed: 64,
            as_of: "2026-07-21T00:00:00Z".into(),
            providers: vec![graph, liar],
        }
    }

    #[test]
    fn usage_report_roundtrips_through_json() {
        let report = sample_report();
        let json = serde_json::to_string(&report).unwrap();
        let back: UsageReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn a_consistent_report_re_sums_from_its_served_frames() {
        let report = sample_report();
        assert!(report.is_consistent());
        assert_eq!(report.total_provider_cost(), 64);
        assert_eq!(report.budget_consumed, report.total_provider_cost());
        assert_eq!(report.total_frames_served(), 2);
        assert!(report.within_budget());
    }

    #[test]
    fn a_tampered_total_fails_the_arithmetic_identity() {
        // Inflate the consumed total without touching the served frames: the
        // report no longer re-sums, and the check catches it — exactly the
        // misbill a metering pipeline must refuse.
        let mut report = sample_report();
        report.budget_consumed = 9_999;
        assert!(!report.is_consistent());

        // A per-provider aggregate that disagrees with its frames also fails.
        let mut report = sample_report();
        report.providers[0].token_cost = 100; // frames still sum to 64
        assert!(!report.is_consistent());
        assert!(!report.providers[0].is_consistent());
    }

    #[test]
    fn served_frames_carry_the_stable_identity_for_audit_walk_back() {
        let report = sample_report();
        let first = &report.providers[0].served_frames[0];
        assert_eq!(first.frame.provider_id, "repo-graph");
        assert_eq!(first.frame.frame_id, "retry-doc");
        assert_eq!(first.frame.content_digest.as_deref(), Some("sha256:aa"));
        assert_eq!(first.token_cost, 41);
    }

    #[test]
    fn empty_provider_and_served_lists_are_omitted_when_absent() {
        let report = UsageReport {
            budget_requested: 100,
            budget_consumed: 0,
            as_of: "2026-07-21T00:00:00Z".into(),
            providers: vec![],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("providers"));
        let back: UsageReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }
}
