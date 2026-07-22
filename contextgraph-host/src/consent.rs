//! Consent gating for egress providers (`SPEC.md` §4 and §10;
//! `SPEC.md` §7 point 4).
//!
//! The security-critical rule: a provider that declares `egress` — anything
//! that could send workspace content off the local machine — MUST NOT be
//! queried until the user has recorded explicit, one-time consent that
//! **names what leaves**. A host never auto-enables egress. Read/write-only
//! providers carry no such gate. The store is in-memory and serde-able so a
//! host can persist the user's decisions across runs (task deliverable 4).
//!
//! Scope-level consent is recorded as a
//! [`ConsentReceipt`](contextgraph_types::ConsentReceipt) — a protocol-defined
//! shape that lives in `contextgraph-types` alongside the usage report, since
//! any host claiming the consent guarantee must produce it and any auditor must
//! be able to read it. This module holds the host machinery that *consumes*
//! receipts: the append-only ledger and the gate.

use std::collections::HashMap;

use contextgraph_types::{ConsentReceipt, DataFlow, EgressScope, ProviderInfo};
use serde::{Deserialize, Serialize};

/// A recorded consent decision for one provider. `granted_scope` is the
/// human-readable description of what data flows out, shown to the user at
/// consent time and retained as the audit of what they agreed to (SPEC.md §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// The provider id this consent applies to (the host's routing key).
    pub provider_id: String,
    /// The data-flow direction the user consented to — names what leaves.
    pub data_flow: DataFlow,
    /// Human-readable scope: what content is permitted to leave the machine.
    pub granted_scope: String,
    /// When consent was granted (RFC 3339), if the host records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<String>,
}

impl ConsentRecord {
    /// Record consent for a provider, naming the data-flow direction and the
    /// scope of what may leave.
    pub fn new(
        provider_id: impl Into<String>,
        data_flow: DataFlow,
        granted_scope: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            data_flow,
            granted_scope: granted_scope.into(),
            granted_at: None,
        }
    }
}

/// The host's pre-query consent verdict for one provider — the gate result the
/// host acts on before transmitting a query (`docs/context-reuse.md` §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentDecision {
    /// The query may be transmitted: the provider is local, or every off-machine
    /// egress scope it declares has a recorded receipt, or its legacy boolean
    /// egress consent is on file.
    Permitted,
    /// The provider declares `egress` with **no** egress scopes (the pre-scope
    /// boolean contract) and no consent is recorded. Transmitting is refused.
    NeedsConsent,
    /// The provider declares off-machine egress scope(s) with **no** recorded
    /// consent receipt. Carries exactly the scopes still lacking a receipt, so
    /// the host's typed error names what would leave unconsented.
    NeedsReceipts(Vec<EgressScope>),
}

/// The set of consent decisions a host holds: a keyed table of legacy boolean
/// [`ConsentRecord`]s and an **append-only** ledger of scope-level
/// [`ConsentReceipt`]s (`docs/context-reuse.md` §3). Both are serde-able so a
/// host can persist a user's decisions — and the receipt ledger — across runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsentStore {
    #[serde(default)]
    records: HashMap<String, ConsentRecord>,
    /// Append-only: receipts are pushed, never removed or mutated, so the full
    /// history of what was agreed survives as the audit trail.
    #[serde(default)]
    receipts: Vec<ConsentReceipt>,
}

impl ConsentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or replace) legacy boolean consent for a provider.
    pub fn record(&mut self, record: ConsentRecord) {
        self.records.insert(record.provider_id.clone(), record);
    }

    /// Append a scope-level consent receipt to the audit ledger. Append-only:
    /// this never removes or edits an earlier receipt (§3).
    pub fn record_receipt(&mut self, receipt: ConsentReceipt) {
        self.receipts.push(receipt);
    }

    /// The full append-only receipt ledger, in the order receipts were granted
    /// — the audit trail.
    pub fn receipts(&self) -> &[ConsentReceipt] {
        &self.receipts
    }

    /// Every receipt recorded for a provider, in grant order.
    pub fn receipts_for<'a>(
        &'a self,
        provider_id: &'a str,
    ) -> impl Iterator<Item = &'a ConsentReceipt> {
        self.receipts
            .iter()
            .filter(move |receipt| receipt.provider_id == provider_id)
    }

    /// Whether any recorded receipt authorizes `scope` for `provider_id`
    /// (**presence**, ignoring expiry). This is what the zero-clock runtime gate
    /// consults; a host that also enforces expiry uses [`live_receipt`](Self::live_receipt).
    pub fn has_receipt(&self, provider_id: &str, scope: &EgressScope) -> bool {
        self.receipts
            .iter()
            .any(|receipt| receipt.provider_id == provider_id && &receipt.scope == scope)
    }

    /// The receipt authorizing `scope` for `provider_id` that is live at `now`
    /// (presence **and** non-expiry), if any. A host enforcing expiry gates on
    /// this against its own clock (`docs/context-reuse.md` §3).
    pub fn live_receipt(
        &self,
        provider_id: &str,
        scope: &EgressScope,
        now: &str,
    ) -> Option<&ConsentReceipt> {
        self.receipts.iter().find(|receipt| {
            receipt.provider_id == provider_id && &receipt.scope == scope && receipt.is_live(now)
        })
    }

    /// Withdraw consent for a provider, returning the prior record if any.
    pub fn revoke(&mut self, provider_id: &str) -> Option<ConsentRecord> {
        self.records.remove(provider_id)
    }

    /// The recorded decision for a provider, if consent was granted.
    pub fn get(&self, provider_id: &str) -> Option<&ConsentRecord> {
        self.records.get(provider_id)
    }

    /// Whether consent has been recorded for a provider.
    pub fn is_consented(&self, provider_id: &str) -> bool {
        self.records.contains_key(provider_id)
    }

    /// Whether a provider needs consent before any query: a provider needs it
    /// if it declares the boolean `egress` flag **or** any off-machine egress
    /// scope (§3.5, §3). A purely local provider is always permitted — nothing
    /// it can do leaves the machine.
    pub fn requires_consent(info: &ProviderInfo) -> bool {
        info.data_flow.egress || info.data_flow.off_machine_scopes().next().is_some()
    }

    /// The host's pre-query consent gate: may we transmit a query to this
    /// provider right now, and if not, *why* (`docs/context-reuse.md` §3)?
    ///
    /// - A provider declaring **off-machine egress scopes** is governed by the
    ///   receipt gate: permitted only when every off-machine scope has a
    ///   recorded receipt; otherwise [`NeedsReceipts`](ConsentDecision::NeedsReceipts)
    ///   names the scopes still missing one. (This is presence-based — a host
    ///   enforcing expiry prunes/consults live receipts with its own clock.)
    /// - A provider declaring only the **boolean `egress`** flag (no scopes) is
    ///   governed by the legacy gate: permitted with a recorded [`ConsentRecord`],
    ///   else [`NeedsConsent`](ConsentDecision::NeedsConsent).
    /// - A purely local provider is [`Permitted`](ConsentDecision::Permitted).
    pub fn evaluate(&self, id: &str, info: &ProviderInfo) -> ConsentDecision {
        let off_machine: Vec<&EgressScope> = info.data_flow.off_machine_scopes().collect();
        if !off_machine.is_empty() {
            let missing: Vec<EgressScope> = off_machine
                .into_iter()
                .filter(|scope| !self.has_receipt(id, scope))
                .cloned()
                .collect();
            if missing.is_empty() {
                ConsentDecision::Permitted
            } else {
                ConsentDecision::NeedsReceipts(missing)
            }
        } else if info.data_flow.egress {
            if self.is_consented(id) {
                ConsentDecision::Permitted
            } else {
                ConsentDecision::NeedsConsent
            }
        } else {
            ConsentDecision::Permitted
        }
    }

    /// The boolean form of [`evaluate`](Self::evaluate): may we send the payload
    /// to this provider right now?
    pub fn permits(&self, id: &str, info: &ProviderInfo) -> bool {
        matches!(self.evaluate(id, info), ConsentDecision::Permitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn egress_info() -> ProviderInfo {
        ProviderInfo {
            name: "contextgraph-github".into(),
            version: "0.1.0".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![],
            },
        }
    }

    fn scoped_info() -> ProviderInfo {
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

    fn local_info() -> ProviderInfo {
        ProviderInfo {
            name: "contextgraph-docs".into(),
            version: "0.1.0".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: false,
                egress_scopes: vec![],
            },
        }
    }

    #[test]
    fn local_providers_never_need_consent() {
        let store = ConsentStore::new();
        let info = local_info();
        assert!(!ConsentStore::requires_consent(&info));
        assert!(store.permits("contextgraph-docs", &info));
    }

    #[test]
    fn egress_providers_are_gated_until_consent_is_recorded() {
        let mut store = ConsentStore::new();
        let info = egress_info();
        assert!(ConsentStore::requires_consent(&info));
        // No consent yet → the gate is shut.
        assert!(!store.permits("contextgraph-github", &info));

        store.record(ConsentRecord::new(
            "contextgraph-github",
            info.data_flow.clone(),
            "open issue titles + bodies leave to github.com",
        ));
        assert!(store.permits("contextgraph-github", &info));
        assert_eq!(
            store
                .get("contextgraph-github")
                .map(|r| r.granted_scope.as_str()),
            Some("open issue titles + bodies leave to github.com")
        );
    }

    #[test]
    fn revoking_consent_reshuts_the_gate() {
        let mut store = ConsentStore::new();
        let info = egress_info();
        store.record(ConsentRecord::new(
            "contextgraph-github",
            info.data_flow.clone(),
            "issues",
        ));
        assert!(store.permits("contextgraph-github", &info));
        let revoked = store
            .revoke("contextgraph-github")
            .expect("a record existed");
        assert_eq!(revoked.provider_id, "contextgraph-github");
        assert!(!store.permits("contextgraph-github", &info));
    }

    #[test]
    fn consent_store_is_serde_able_for_persistence() {
        let mut store = ConsentStore::new();
        store.record(ConsentRecord::new(
            "contextgraph-github",
            DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![],
            },
            "issues + PRs",
        ));
        let json = serde_json::to_string(&store).unwrap();
        let back: ConsentStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back, store);
        assert!(back.is_consented("contextgraph-github"));
    }

    use contextgraph_types::Grantor;

    fn receipt(provider: &str, scope: EgressScope) -> ConsentReceipt {
        ConsentReceipt::new(
            provider,
            &scoped_info(),
            scope,
            Grantor::Human("alice".into()),
            "2026-07-21T00:00:00Z",
        )
    }

    #[test]
    fn a_scoped_provider_is_gated_until_every_off_machine_scope_has_a_receipt() {
        let mut store = ConsentStore::new();
        let info = scoped_info();
        assert!(ConsentStore::requires_consent(&info));

        // No receipt yet → the gate names the missing scope, and the query is
        // refused with the scope-specific decision (not the legacy boolean).
        match store.evaluate("contextgraph-cloud", &info) {
            ConsentDecision::NeedsReceipts(missing) => {
                assert_eq!(missing, vec![EgressScope::ThirdPartyModel]);
            }
            other => panic!("expected NeedsReceipts, got {other:?}"),
        }
        assert!(!store.permits("contextgraph-cloud", &info));

        // A boolean ConsentRecord does NOT satisfy a scope gate — only a
        // receipt for the declared scope does.
        store.record(ConsentRecord::new(
            "contextgraph-cloud",
            info.data_flow.clone(),
            "legacy boolean consent",
        ));
        assert!(!store.permits("contextgraph-cloud", &info));

        // Record the receipt for the declared scope → permitted.
        store.record_receipt(receipt("contextgraph-cloud", EgressScope::ThirdPartyModel));
        assert_eq!(
            store.evaluate("contextgraph-cloud", &info),
            ConsentDecision::Permitted
        );
        assert!(store.permits("contextgraph-cloud", &info));
    }

    #[test]
    fn a_receipt_for_the_wrong_scope_does_not_unlock_a_different_scope() {
        let mut store = ConsentStore::new();
        let info = ProviderInfo {
            name: "contextgraph-cloud".into(),
            version: "0.1.0".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![EgressScope::ThirdPartyIndex, EgressScope::ThirdPartyModel],
            },
        };
        // Only the index scope is consented; the model scope is still missing.
        store.record_receipt(receipt("contextgraph-cloud", EgressScope::ThirdPartyIndex));
        match store.evaluate("contextgraph-cloud", &info) {
            ConsentDecision::NeedsReceipts(missing) => {
                assert_eq!(missing, vec![EgressScope::ThirdPartyModel]);
            }
            other => panic!("expected NeedsReceipts for the model scope, got {other:?}"),
        }
    }

    #[test]
    fn a_local_only_scope_needs_no_receipt() {
        let store = ConsentStore::new();
        let info = ProviderInfo {
            name: "contextgraph-docs".into(),
            version: "0.1.0".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: false,
                egress_scopes: vec![EgressScope::LocalOnly],
            },
        };
        // local-only is on-machine, so it triggers no receipt gate at all.
        assert!(!ConsentStore::requires_consent(&info));
        assert!(store.permits("contextgraph-docs", &info));
    }

    #[test]
    fn receipts_are_append_only_and_carry_the_full_audit_trail() {
        let mut store = ConsentStore::new();
        store.record_receipt(receipt("contextgraph-cloud", EgressScope::ThirdPartyModel));
        store.record_receipt(
            ConsentReceipt::new(
                "contextgraph-cloud",
                &scoped_info(),
                EgressScope::ThirdPartyIndex,
                Grantor::Policy("data-egress-policy-v2".into()),
                "2026-07-22T00:00:00Z",
            )
            .with_expiry("2026-08-22T00:00:00Z"),
        );
        // Both receipts retained (append-only), in grant order.
        assert_eq!(store.receipts().len(), 2);
        assert_eq!(store.receipts_for("contextgraph-cloud").count(), 2);
        assert_eq!(store.receipts()[0].scope, EgressScope::ThirdPartyModel);
        assert!(matches!(store.receipts()[1].grantor, Grantor::Policy(_)));
    }

    #[test]
    fn an_expired_receipt_is_not_live_but_stays_in_the_ledger() {
        let mut store = ConsentStore::new();
        store.record_receipt(
            receipt("contextgraph-cloud", EgressScope::ThirdPartyModel)
                .with_expiry("2026-07-22T00:00:00Z"),
        );
        // Live before expiry, not after — but the receipt is never removed.
        assert!(
            store
                .live_receipt(
                    "contextgraph-cloud",
                    &EgressScope::ThirdPartyModel,
                    "2026-07-21T12:00:00Z",
                )
                .is_some()
        );
        assert!(
            store
                .live_receipt(
                    "contextgraph-cloud",
                    &EgressScope::ThirdPartyModel,
                    "2026-07-23T00:00:00Z",
                )
                .is_none()
        );
        assert_eq!(
            store.receipts().len(),
            1,
            "expiry never prunes the audit trail"
        );
    }

    #[test]
    fn a_serialized_store_carries_its_receipt_ledger_across_runs() {
        // The ledger is the durable audit artifact, so it must survive the
        // round-trip a host does when persisting decisions between sessions.
        let mut store = ConsentStore::new();
        store.record_receipt(receipt("contextgraph-cloud", EgressScope::ThirdPartyModel));
        let back: ConsentStore = serde_json::from_str(&serde_json::to_string(&store).unwrap())
            .expect("a store with receipts round-trips");
        assert_eq!(back, store);
        assert!(back.has_receipt("contextgraph-cloud", &EgressScope::ThirdPartyModel));
    }
}
