//! Egress-scope vocabulary — a closed, extensible classification of *where*
//! a provider's content goes (`docs/context-reuse.md` §3).
//!
//! `DataFlow.egress` answers a yes/no question: does anything leave the machine?
//! That boolean is enough to gate consent, but not enough to *record* it: an
//! auditor asking "what left, and to whom?" months later needs the class of
//! destination, not just the fact of departure. [`EgressScope`] is that class.
//!
//! Four **normative base scopes** form the closed core every host and provider
//! agree on:
//!
//! - [`LocalOnly`](EgressScope::LocalOnly) — nothing leaves the machine.
//! - [`OrgTenant`](EgressScope::OrgTenant) — leaves the machine but stays inside
//!   the organization's own infrastructure.
//! - [`ThirdPartyIndex`](EgressScope::ThirdPartyIndex) — content sent to an
//!   external index / embedding service.
//! - [`ThirdPartyModel`](EgressScope::ThirdPartyModel) — content sent to an
//!   external model API.
//!
//! The vocabulary is **extensible** by [`Custom`](EgressScope::Custom) scopes,
//! which MUST be **namespaced** (`vendor:scope-name`) so a custom scope can
//! never collide with — or be mistaken for — a base class. Everything other
//! than `local-only` is treated as [off-machine](EgressScope::is_off_machine):
//! an unrecognized custom scope is conservatively assumed to leave.
//!
//! A provider declares its scopes in [`DataFlow::egress_scopes`](crate::DataFlow::egress_scopes);
//! a scope governs **every frame that provider serves** (there is no per-frame
//! scope — the serving provider's declaration is the frame's egress class).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The wire strings of the four normative base scopes.
const LOCAL_ONLY: &str = "local-only";
const ORG_TENANT: &str = "org-tenant";
const THIRD_PARTY_INDEX: &str = "third-party-index";
const THIRD_PARTY_MODEL: &str = "third-party-model";

/// Where a provider's served content may go. A closed base vocabulary of four
/// classes plus namespaced custom extensions (`docs/context-reuse.md` §3).
///
/// Serializes to a flat string — the base classes to their canonical kebab-case
/// names, a [`Custom`](Self::Custom) to its namespaced string — so the wire form
/// is a plain enum-of-strings any language can produce.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EgressScope {
    /// Nothing leaves the machine. The only scope compatible with
    /// `data_flow.egress == false`.
    LocalOnly,
    /// Leaves the machine but stays inside the organization's own
    /// infrastructure.
    OrgTenant,
    /// Content sent to an external index / embedding service.
    ThirdPartyIndex,
    /// Content sent to an external model API.
    ThirdPartyModel,
    /// A namespaced custom scope, e.g. `acme:vector-store`. MUST contain a
    /// namespace separator `:` with non-empty sides so it can never collide
    /// with the base vocabulary — see [`is_valid`](Self::is_valid).
    Custom(String),
}

impl EgressScope {
    /// The canonical wire string of this scope.
    pub fn as_str(&self) -> &str {
        match self {
            Self::LocalOnly => LOCAL_ONLY,
            Self::OrgTenant => ORG_TENANT,
            Self::ThirdPartyIndex => THIRD_PARTY_INDEX,
            Self::ThirdPartyModel => THIRD_PARTY_MODEL,
            Self::Custom(scope) => scope,
        }
    }

    /// Parse a wire string: a known base name maps to its variant, anything
    /// else to [`Custom`](Self::Custom). Never fails — validity of a custom
    /// scope is a separate, checkable property ([`is_valid`](Self::is_valid)),
    /// so an unknown-but-well-formed scope round-trips rather than erroring.
    pub fn from_wire(scope: impl Into<String>) -> Self {
        let scope = scope.into();
        match scope.as_str() {
            LOCAL_ONLY => Self::LocalOnly,
            ORG_TENANT => Self::OrgTenant,
            THIRD_PARTY_INDEX => Self::ThirdPartyIndex,
            THIRD_PARTY_MODEL => Self::ThirdPartyModel,
            _ => Self::Custom(scope),
        }
    }

    /// Whether this scope is one of the four normative base classes.
    pub fn is_base(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }

    /// Whether content under this scope leaves the machine. Everything except
    /// [`LocalOnly`](Self::LocalOnly) is off-machine, including any custom
    /// scope — the conservative default is that an unrecognized destination
    /// leaves, so a host never under-gates a custom scope.
    pub fn is_off_machine(&self) -> bool {
        !matches!(self, Self::LocalOnly)
    }

    /// Whether this scope is well-formed. Base classes are always valid; a
    /// [`Custom`](Self::Custom) scope MUST be namespaced — exactly one purpose
    /// of the `:` separator is guaranteeing it cannot be spelled as a bare base
    /// name. Valid iff it contains a `:` with a non-empty namespace and a
    /// non-empty name.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Custom(scope) => match scope.split_once(':') {
                Some((namespace, name)) => !namespace.is_empty() && !name.is_empty(),
                None => false,
            },
            _ => true,
        }
    }
}

impl fmt::Display for EgressScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for EgressScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EgressScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let scope = String::deserialize(deserializer)?;
        Ok(Self::from_wire(scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_scopes_round_trip_through_their_canonical_strings() {
        for (scope, wire) in [
            (EgressScope::LocalOnly, "local-only"),
            (EgressScope::OrgTenant, "org-tenant"),
            (EgressScope::ThirdPartyIndex, "third-party-index"),
            (EgressScope::ThirdPartyModel, "third-party-model"),
        ] {
            assert_eq!(scope.as_str(), wire);
            let json = serde_json::to_string(&scope).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            let back: EgressScope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
            assert!(scope.is_base() && scope.is_valid());
        }
    }

    #[test]
    fn a_custom_scope_round_trips_as_a_flat_string() {
        let scope = EgressScope::Custom("acme:vector-store".into());
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(json, "\"acme:vector-store\"");
        let back: EgressScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scope);
        assert!(!scope.is_base());
        assert!(scope.is_valid());
    }

    #[test]
    fn an_unknown_base_like_string_deserializes_to_custom_not_a_base_class() {
        // A string that isn't one of the four base names is Custom — the
        // vocabulary is closed at the base level, open by namespacing.
        let back: EgressScope = serde_json::from_str("\"acme:special\"").unwrap();
        assert_eq!(back, EgressScope::Custom("acme:special".into()));
    }

    #[test]
    fn only_local_only_is_on_machine() {
        assert!(!EgressScope::LocalOnly.is_off_machine());
        assert!(EgressScope::OrgTenant.is_off_machine());
        assert!(EgressScope::ThirdPartyIndex.is_off_machine());
        assert!(EgressScope::ThirdPartyModel.is_off_machine());
        // A custom scope is conservatively off-machine.
        assert!(EgressScope::Custom("acme:sink".into()).is_off_machine());
    }

    #[test]
    fn a_non_namespaced_custom_scope_is_invalid() {
        assert!(!EgressScope::Custom("notnamespaced".into()).is_valid());
        assert!(!EgressScope::Custom(":no-namespace".into()).is_valid());
        assert!(!EgressScope::Custom("no-name:".into()).is_valid());
        assert!(EgressScope::Custom("ns:name".into()).is_valid());
    }
}
