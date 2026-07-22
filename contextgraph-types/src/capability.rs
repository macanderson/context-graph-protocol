//! Handshake and capability negotiation types
//! (`docs/specs/stella-rust-cli/06-context-protocol.md` §3.2). `DataFlow` is
//! the security-critical field: hosts surface it at install/consent time,
//! and `egress: true` providers must never be auto-enabled (§3.5).

use serde::{Deserialize, Serialize};

use crate::frame::Representation;
use crate::scope::EgressScope;

/// Declares what a provider does with data, so a host can gate consent
/// before ever sending it a query.
///
/// Not `Copy`: [`egress_scopes`](Self::egress_scopes) is an owned `Vec`, so a
/// `DataFlow` is cloned, not bit-copied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFlow {
    /// Can see workspace content via query payloads.
    #[serde(default)]
    pub reads: bool,
    /// Persists `context/upsert` writes.
    #[serde(default)]
    pub writes: bool,
    /// Sends anything off the local machine. A host MUST require explicit,
    /// one-time consent before enabling a provider with `egress: true`.
    #[serde(default)]
    pub egress: bool,
    /// The [egress scopes](EgressScope) this provider's served content falls
    /// under (`docs/context-reuse.md` §3). Empty ⇒ the provider declares only
    /// the boolean `egress` posture (the pre-scope contract). An off-machine
    /// scope here is only consistent with `egress == true`
    /// (see [`scopes_consistent`](Self::scopes_consistent)); a scope governs
    /// every frame the provider serves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub egress_scopes: Vec<EgressScope>,
}

impl DataFlow {
    /// The declared scopes whose content leaves the machine
    /// ([`EgressScope::is_off_machine`]).
    pub fn off_machine_scopes(&self) -> impl Iterator<Item = &EgressScope> {
        self.egress_scopes.iter().filter(|s| s.is_off_machine())
    }

    /// Whether the declared scopes are truthful and well-formed
    /// (`docs/context-reuse.md` §3, requirement C5). A host holds a provider to
    /// this at the handshake:
    ///
    /// - every declared scope MUST be well-formed ([`EgressScope::is_valid`] —
    ///   custom scopes must be namespaced);
    /// - an **off-machine scope alongside `egress: false` is a lie** — a
    ///   provider cannot claim `local-only` posture while declaring content
    ///   leaves. (The converse is allowed: `egress: true` with no scopes is the
    ///   legacy boolean contract.)
    pub fn scopes_consistent(&self) -> bool {
        if !self.egress_scopes.iter().all(EgressScope::is_valid) {
            return false;
        }
        // An off-machine scope requires the egress bit set.
        self.egress || self.off_machine_scopes().next().is_none()
    }
}

/// Provider identity reported at `initialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub version: String,
    pub data_flow: DataFlow,
}

/// What a provider can do, negotiated at handshake time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub query: QueryCapability,
    #[serde(default)]
    pub upsert: bool,
    #[serde(default)]
    pub graph: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embeddings_fingerprint: Option<String>,
    #[serde(default)]
    pub subscribe: bool,
    /// Whether this provider answers `context/verify` — pull-based
    /// revalidation of frames a host already holds (`docs/context-reuse.md`
    /// §4). Defaults to `false`, so a provider that says nothing is treated as
    /// not supporting it and the host falls back to re-querying.
    ///
    /// Independent of [`subscribe`](Self::subscribe): push and pull freshness
    /// are complementary, and a provider may advertise either, both, or
    /// neither.
    #[serde(default)]
    pub verify: bool,
    /// The [frame representations](Representation) this provider can return
    /// (build prompt §"Capability negotiation"). Empty ⇒ `full` only, the
    /// legacy default, so a provider that says nothing keeps working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub representations: Vec<Representation>,
    /// Whether this provider answers `context/resolve` for a frame's
    /// [`content_ref`](crate::ContentRef). Compact/reference support **implies**
    /// resolve support ([`representations_consistent`](Self::representations_consistent)).
    #[serde(default)]
    pub resolve: bool,
}

impl Capabilities {
    /// The representations this provider actually offers, defaulting to `[full]`
    /// when it advertised none (the legacy contract).
    pub fn offered_representations(&self) -> Vec<Representation> {
        if self.representations.is_empty() {
            vec![Representation::Full]
        } else {
            self.representations.clone()
        }
    }

    /// Whether the advertised representations are honest: `compact`/`reference`
    /// both hand the host a [`content_ref`](crate::ContentRef) to rehydrate, so
    /// a provider that cannot [`resolve`](Self::resolve) must not advertise
    /// them. A provider offering only `full` is always consistent.
    pub fn representations_consistent(&self) -> bool {
        if self.resolve {
            return true;
        }
        !self
            .representations
            .iter()
            .any(|rep| matches!(rep, Representation::Compact | Representation::Reference))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryCapability {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub filters: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::EgressScope;

    #[test]
    fn verify_support_defaults_off_and_is_independent_of_subscribe() {
        // A provider that says nothing must not be assumed to verify — the
        // host falls back to re-querying (§4).
        let caps = Capabilities::default();
        assert!(!caps.verify);
        assert!(!caps.subscribe);

        // Absent from the wire ⇒ still false, so pre-§4 providers keep working.
        let back: Capabilities = serde_json::from_str(r#"{"upsert":false}"#).unwrap();
        assert!(!back.verify);

        // Push and pull freshness are independent axes.
        let pull_only: Capabilities =
            serde_json::from_str(r#"{"verify":true,"subscribe":false}"#).unwrap();
        assert!(pull_only.verify && !pull_only.subscribe);
        let push_only: Capabilities =
            serde_json::from_str(r#"{"verify":false,"subscribe":true}"#).unwrap();
        assert!(!push_only.verify && push_only.subscribe);
    }

    #[test]
    fn egress_provider_data_flow_roundtrips() {
        let flow = DataFlow {
            reads: true,
            writes: false,
            egress: true,
            egress_scopes: vec![EgressScope::ThirdPartyModel],
        };
        let json = serde_json::to_string(&flow).unwrap();
        let back: DataFlow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flow);
        assert!(
            back.egress,
            "egress providers must be inspectable by hosts before consent"
        );
    }

    #[test]
    fn provider_info_defaults_data_flow_to_no_egress() {
        let flow = DataFlow::default();
        assert!(
            !flow.egress,
            "default DataFlow must never imply egress consent"
        );
        assert!(flow.egress_scopes.is_empty());
        assert!(flow.scopes_consistent());
    }

    #[test]
    fn empty_egress_scopes_are_omitted_from_the_wire() {
        let flow = DataFlow {
            reads: true,
            writes: false,
            egress: false,
            egress_scopes: vec![],
        };
        let json = serde_json::to_string(&flow).unwrap();
        assert!(
            !json.contains("egress_scopes"),
            "an empty scope list must be omitted so the pre-scope wire form is unchanged: {json}"
        );
    }

    #[test]
    fn an_off_machine_scope_with_egress_false_is_inconsistent() {
        // C5: a provider cannot claim local posture while declaring content
        // leaves.
        let lying = DataFlow {
            reads: true,
            writes: false,
            egress: false,
            egress_scopes: vec![EgressScope::ThirdPartyIndex],
        };
        assert!(!lying.scopes_consistent());

        // local-only alongside egress:false is fine.
        let honest_local = DataFlow {
            reads: true,
            writes: false,
            egress: false,
            egress_scopes: vec![EgressScope::LocalOnly],
        };
        assert!(honest_local.scopes_consistent());

        // An off-machine scope with egress:true is fine.
        let honest_egress = DataFlow {
            reads: true,
            writes: false,
            egress: true,
            egress_scopes: vec![EgressScope::ThirdPartyModel],
        };
        assert!(honest_egress.scopes_consistent());
        assert_eq!(honest_egress.off_machine_scopes().count(), 1);

        // A malformed custom scope is inconsistent regardless of egress.
        let malformed = DataFlow {
            reads: true,
            writes: false,
            egress: true,
            egress_scopes: vec![EgressScope::Custom("notnamespaced".into())],
        };
        assert!(!malformed.scopes_consistent());
    }

    #[test]
    fn capabilities_roundtrip_with_defaults() {
        let caps = Capabilities {
            query: QueryCapability {
                kinds: vec!["snippet".into()],
                filters: vec![],
            },
            upsert: true,
            graph: false,
            embeddings_fingerprint: Some("bge-small-v1".into()),
            subscribe: false,
            verify: true,
            representations: vec![Representation::Full, Representation::Reference],
            resolve: true,
        };
        let json = serde_json::to_string(&caps).unwrap();
        let back: Capabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, caps);
    }

    #[test]
    fn representation_capability_defaults_to_full_only_and_is_wire_omitted() {
        // A provider that says nothing offers `full` only, and neither new
        // field disturbs the pre-representation wire form (resolve defaults
        // false and is a plain bool, representations omits when empty).
        let caps = Capabilities::default();
        assert_eq!(caps.offered_representations(), vec![Representation::Full]);
        assert!(caps.representations_consistent());
        let json = serde_json::to_string(&caps).unwrap();
        assert!(!json.contains("representations"));

        // Absent from the wire ⇒ still full-only, so pre-representation
        // providers keep working.
        let back: Capabilities = serde_json::from_str(r#"{"upsert":false}"#).unwrap();
        assert_eq!(back.offered_representations(), vec![Representation::Full]);
        assert!(!back.resolve);
    }

    #[test]
    fn reference_or_compact_without_resolve_is_inconsistent() {
        // Compact/reference hand the host a content_ref to rehydrate; a
        // provider that cannot resolve must not advertise them.
        let lying = Capabilities {
            representations: vec![Representation::Reference],
            resolve: false,
            ..Capabilities::default()
        };
        assert!(!lying.representations_consistent());

        let honest = Capabilities {
            representations: vec![Representation::Reference],
            resolve: true,
            ..Capabilities::default()
        };
        assert!(honest.representations_consistent());

        // Advertising only `full` never requires resolve.
        let full_only = Capabilities {
            representations: vec![Representation::Full],
            resolve: false,
            ..Capabilities::default()
        };
        assert!(full_only.representations_consistent());
    }
}
