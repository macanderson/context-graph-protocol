//! Consent receipts — the durable, audit-grade record of a granted egress
//! permission (`docs/context-reuse.md` §3).
//!
//! A boolean consent flag answers "is this provider allowed?" *in the moment it
//! is asked*, inside a process that will exit. It cannot answer the question an
//! auditor actually asks months later: "**what** left the machine, **to whom**,
//! **who** agreed, and **when**?" A [`ConsentReceipt`] is that answer — consent
//! as an *artifact* rather than an event.
//!
//! Like [`UsageReport`](crate::UsageReport), a receipt is a **host-side
//! artifact, not a wire message**: it rides no envelope variant, and a provider
//! implements nothing to make one possible. It lives here rather than in the
//! host crate for the same reason the usage report does — it is a *protocol-
//! defined shape*. Any host, in any language, that claims to implement the
//! consent guarantee must produce this shape, and any auditor reading a ledger
//! must be able to parse it without depending on one particular host
//! implementation. The gate that *consumes* receipts
//! ([`ConsentStore`](https://docs.rs/contextgraph-host/latest/contextgraph_host/consent/struct.ConsentStore.html))
//! is host machinery and stays in the host crate.
//!
//! The receipt pins the provider's identity at grant time, names an accountable
//! [`Grantor`], and carries an optional expiry. Hosts hold receipts in an
//! **append-only** ledger: a new grant never edits or erases an old one, so the
//! history of consent is itself the audit trail.

use serde::{Deserialize, Serialize};

use crate::capability::ProviderInfo;
use crate::scope::EgressScope;

/// Who granted a consent receipt (`docs/context-reuse.md` §3). Recorded so the
/// audit trail names an accountable party, not just a moment.
///
/// Serializes as a tagged object — `{"kind": "human", "id": "…"}` — so a
/// grantor is self-describing in a persisted ledger rather than a bare string
/// whose meaning depends on out-of-band convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Grantor {
    /// A human user, identified however the host names them at consent time
    /// (a user id, email, or display name).
    Human(String),
    /// An automated policy, identified by its policy id or name — consent
    /// granted by a rule rather than a person present in the moment.
    Policy(String),
}

/// An audit-grade record that consent was granted for one provider to send
/// content under one [egress scope](EgressScope) (`docs/context-reuse.md` §3).
///
/// It pins the provider's identity at grant time (so a later rename can't
/// retroactively rewrite what was agreed), names the [`Grantor`], and carries
/// an optional expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentReceipt {
    /// The provider id (host routing/consent key) this receipt authorizes.
    pub provider_id: String,
    /// The egress scope consented to. Content leaving this provider under this
    /// scope is authorized; any other off-machine scope it declares is not,
    /// until its own receipt exists.
    pub scope: EgressScope,
    /// The provider's declared name at grant time — pinned so the audit trail
    /// survives the provider being renamed or swapped.
    pub provider_name: String,
    /// The provider's declared version at grant time.
    pub provider_version: String,
    /// Who granted consent (a human or a policy).
    pub grantor: Grantor,
    /// When consent was granted (RFC 3339), supplied by the host's clock.
    pub granted_at: String,
    /// When consent expires (RFC 3339), if it does. `None` ⇒ open-ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl ConsentReceipt {
    /// Record consent for `provider` to egress under `scope`, granted by
    /// `grantor` at `granted_at` (an RFC 3339 instant from the host's clock).
    /// The provider's identity is copied out of `info` and pinned into the
    /// receipt. Open-ended by default; add an expiry with
    /// [`with_expiry`](Self::with_expiry).
    pub fn new(
        provider_id: impl Into<String>,
        info: &ProviderInfo,
        scope: EgressScope,
        grantor: Grantor,
        granted_at: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            scope,
            provider_name: info.name.clone(),
            provider_version: info.version.clone(),
            grantor,
            granted_at: granted_at.into(),
            expires_at: None,
        }
    }

    /// Set the receipt's expiry (RFC 3339).
    pub fn with_expiry(mut self, expires_at: impl Into<String>) -> Self {
        self.expires_at = Some(expires_at.into());
        self
    }

    /// Whether this receipt is still live at `now` (an RFC 3339 instant). A
    /// receipt with no expiry is always live; otherwise it is live while
    /// `now < expires_at`.
    ///
    /// The comparison is lexicographic on the RFC 3339 strings, which is
    /// correct for fixed-width UTC (`Z`) timestamps — the form a host stamps —
    /// so liveness needs no calendar parsing and the type stays dependency-free.
    /// The runtime consent gate is presence-based (it does not carry a clock);
    /// a host that enforces expiry consults this against its own `now`.
    pub fn is_live(&self, now: &str) -> bool {
        match &self.expires_at {
            Some(expiry) => now < expiry.as_str(),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::DataFlow;

    fn info() -> ProviderInfo {
        ProviderInfo {
            name: "contextgraph-cloud".into(),
            version: "0.1.0".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![EgressScope::ThirdPartyModel],
            },
        }
    }

    #[test]
    fn a_receipt_pins_provider_identity_and_round_trips() {
        let receipt = ConsentReceipt::new(
            "contextgraph-cloud",
            &info(),
            EgressScope::ThirdPartyModel,
            Grantor::Human("ops@oxagen.sh".into()),
            "2026-07-21T00:00:00Z",
        );
        assert_eq!(receipt.provider_name, "contextgraph-cloud");
        assert_eq!(receipt.provider_version, "0.1.0");

        let json = serde_json::to_string(&receipt).unwrap();
        let back: ConsentReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(back, receipt);
        // The grantor is a self-describing tagged object in a persisted ledger.
        assert!(json.contains("\"kind\":\"human\""));
        assert!(json.contains("\"id\":\"ops@oxagen.sh\""));
    }

    #[test]
    fn a_policy_grantor_round_trips_distinctly_from_a_human() {
        let receipt = ConsentReceipt::new(
            "contextgraph-cloud",
            &info(),
            EgressScope::OrgTenant,
            Grantor::Policy("data-egress-policy-v2".into()),
            "2026-07-21T00:00:00Z",
        );
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(json.contains("\"kind\":\"policy\""));
        let back: ConsentReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.grantor,
            Grantor::Policy("data-egress-policy-v2".into())
        );
        assert_ne!(
            back.grantor,
            Grantor::Human("data-egress-policy-v2".into()),
            "a policy grant must never be mistaken for a human's"
        );
    }

    #[test]
    fn an_open_ended_receipt_omits_expiry_and_is_always_live() {
        let receipt = ConsentReceipt::new(
            "contextgraph-cloud",
            &info(),
            EgressScope::ThirdPartyModel,
            Grantor::Human("alice".into()),
            "2026-07-21T00:00:00Z",
        );
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(
            !json.contains("expires_at"),
            "an absent expiry must be omitted, not serialized as null: {json}"
        );
        assert!(receipt.is_live("2099-01-01T00:00:00Z"));
    }

    #[test]
    fn expiry_bounds_the_window_of_authorized_egress() {
        let receipt = ConsentReceipt::new(
            "contextgraph-cloud",
            &info(),
            EgressScope::ThirdPartyModel,
            Grantor::Human("alice".into()),
            "2026-07-21T00:00:00Z",
        )
        .with_expiry("2026-10-21T00:00:00Z");
        assert!(receipt.is_live("2026-08-01T00:00:00Z"));
        assert!(!receipt.is_live("2026-11-01T00:00:00Z"));
        // The boundary instant is not live: liveness is `now < expires_at`.
        assert!(!receipt.is_live("2026-10-21T00:00:00Z"));
    }
}
