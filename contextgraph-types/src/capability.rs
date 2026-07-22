//! Handshake and capability negotiation types
//! (`SPEC.md` §3). `DataFlow` is
//! the security-critical field: hosts surface it at install/consent time,
//! and `egress: true` providers must never be auto-enabled (SPEC.md §4).

use serde::{Deserialize, Serialize};

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
    /// The provider durably persists data derived from what it receives —
    /// indexing query payloads, retaining request logs, and the like.
    ///
    /// This is a **consent-surface declaration**, not a capability flag: it
    /// does not imply any host-callable write method, and none exists (see
    /// [ADR 0004](../../docs/adr/0004-dead-capability-surface.md)). It is kept
    /// because "you may read my workspace" and "you may durably record things
    /// about me" are different grants, and a user deserves to be told about
    /// the second one.
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
///
/// Every field here is a capability a host can actually exercise. `upsert` and
/// `subscribe` were removed in the pre-freeze sweep because neither had a wire
/// method, a host API, a schema entry, or a conformance check — a provider
/// could declare a capability no host on earth could use. See
/// [ADR 0004](../../docs/adr/0004-dead-capability-surface.md), and the design
/// sketches under `docs/sketches/` that keep both doors open for a 1.x
/// additive minor.
///
/// Unknown fields are ignored on deserialization, so a provider still emitting
/// the removed flags handshakes successfully — the removal breaks the Rust API,
/// not the wire.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub query: QueryCapability,
    /// The provider echoes the `id` of a request on its reply, so a host may
    /// pipeline concurrent exchanges over one connection (`SPEC.md` §H4).
    ///
    /// Negotiated explicitly rather than discovered by observation. A host that
    /// sent an `id` speculatively could not tell a provider that does not
    /// implement correlation from one that implements it incorrectly, which
    /// makes the guarantee uncheckable — and an uncheckable guarantee is the
    /// thing this protocol exists to avoid. A provider that does not declare it
    /// is queried in lock-step and stays fully conformant.
    #[serde(default)]
    pub correlation: bool,
    /// The provider serves [`FrameKind::Graph`](crate::FrameKind::Graph) frames
    /// and populates [`Relation`](crate::Relation) edges. Gates the graph
    /// conformance checks (`SPEC.md` §G1–G3).
    #[serde(default)]
    pub graph: bool,
    /// Identifies the embedding space this provider indexes, so a host never
    /// sends it a vector from a different model.
    ///
    /// Format: `<model-id>/<dimensions>[/<normalization>]`, e.g.
    /// `bge-small-en-v1.5/384/l2`. Matching is exact — see
    /// [`embedding_fingerprints_match`](crate::embedding_fingerprints_match)
    /// and `SPEC.md` §E1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embeddings_fingerprint: Option<String>,
}

/// The retrieval surface a provider offers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryCapability {
    /// Frame kinds this provider serves, e.g. `["doc", "snippet"]`.
    #[serde(default)]
    pub kinds: Vec<String>,
}

/// The dimension count declared by an embedding fingerprint, if it is
/// well-formed.
///
/// A fingerprint is `<model-id>/<dimensions>[/<normalization>]`. Returning the
/// dimension separately is what lets a provider reject a vector whose length
/// contradicts its own declaration — the cheap check that catches a
/// misconfigured host before it gets silently garbage similarity scores.
pub fn fingerprint_dimensions(fingerprint: &str) -> Option<usize> {
    fingerprint.split('/').nth(1)?.parse().ok()
}

/// Whether a host may send its embeddings to a provider: exact string equality
/// of the two fingerprints (`SPEC.md` §E1).
///
/// Equality is required rather than, say, matching only the model id, because
/// dimension and normalization both change what a vector *means*. A host that
/// sent a 384-dimension unnormalized vector to a provider indexed on 384
/// L2-normalized vectors would get plausible-looking, meaningless scores —
/// precisely the class of silent wrongness the protocol exists to make loud.
pub fn embedding_fingerprints_match(host: &str, provider: &str) -> bool {
    !host.is_empty() && host == provider
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::EgressScope;

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
            },
            correlation: true,
            graph: true,
            embeddings_fingerprint: Some("bge-small-en-v1.5/384/l2".into()),
        };
        let json = serde_json::to_string(&caps).unwrap();
        let back: Capabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, caps);
    }

    #[test]
    fn a_provider_still_declaring_the_removed_capabilities_handshakes_successfully() {
        // ADR 0004 removes `upsert`, `subscribe`, and `filters` from the Rust
        // API but claims the *wire* stays compatible: an already-deployed
        // provider that still emits them is not rejected, the fields are just
        // ignored. That claim is load-bearing for the two live downstreams, so
        // it gets a test rather than a sentence.
        let legacy = r#"{
            "query": { "kinds": ["doc"], "filters": ["language"] },
            "upsert": true,
            "graph": false,
            "subscribe": true
        }"#;
        let caps: Capabilities = serde_json::from_str(legacy).expect("legacy ack must still parse");
        assert_eq!(caps.query.kinds, vec!["doc".to_string()]);
        assert!(!caps.graph);
    }

    #[test]
    fn a_well_formed_fingerprint_yields_its_dimension() {
        assert_eq!(
            fingerprint_dimensions("bge-small-en-v1.5/384/l2"),
            Some(384)
        );
        assert_eq!(
            fingerprint_dimensions("text-embedding-3-large/3072"),
            Some(3072)
        );
    }

    #[test]
    fn a_fingerprint_without_a_parseable_dimension_yields_none() {
        // Rather than defaulting to some guess: a host that cannot read the
        // dimension must not pretend it validated the vector length.
        assert_eq!(fingerprint_dimensions("bge-small-v1"), None);
        assert_eq!(fingerprint_dimensions("model/not-a-number"), None);
        assert_eq!(fingerprint_dimensions(""), None);
    }

    #[test]
    fn fingerprints_match_only_on_exact_equality() {
        let provider = "bge-small-en-v1.5/384/l2";
        assert!(embedding_fingerprints_match(provider, provider));

        // Same model and dimension, different normalization: NOT a match, and
        // this is the case worth pinning — the vectors are the same length, so
        // nothing downstream would notice the mismatch on its own.
        assert!(!embedding_fingerprints_match(
            "bge-small-en-v1.5/384",
            provider
        ));
        assert!(!embedding_fingerprints_match(
            "bge-small-en-v1.5/384/none",
            provider
        ));
        assert!(!embedding_fingerprints_match(
            "text-embedding-3-small/384/l2",
            provider
        ));
    }

    #[test]
    fn an_empty_fingerprint_never_matches_even_itself() {
        // Otherwise two providers that both declined to declare a fingerprint
        // would appear to agree on an embedding space.
        assert!(!embedding_fingerprints_match("", ""));
    }
}
