//! `contextgraph-types` — the Context Graph Protocol (CGP) wire types.
//!
//! This crate is the industry-facing artifact: **MIT licensed, zero
//! dependencies beyond `serde`**, publishable to crates.io on its own so a
//! third party can implement a CGP host or provider without pulling in any
//! other code. The normative shape these types bind to is [`SPEC.md`] at the
//! repository root; every doc comment below cites a section of it.
//!
//! [`SPEC.md`]: https://github.com/macanderson/context-graph-protocol/blob/main/SPEC.md
//!
//! Protocol version: `contextgraph/1.0-draft`.

pub mod capability;
pub mod consent;
pub mod error_code;
pub mod frame;
pub mod identity;
pub mod query;
pub mod scope;
pub mod token;
pub mod usage;
pub mod validate;

pub use capability::{
    Capabilities, DataFlow, ProviderInfo, QueryCapability, embedding_fingerprints_match,
    fingerprint_dimensions,
};
pub use consent::{ConsentReceipt, Grantor};
pub use error_code::{ErrorCode, HostReaction};
pub use frame::{ContextFrame, FrameEmbedding, FrameKind, Provenance, Relation, rel};
pub use identity::{FrameId, canonical_order};
pub use query::{ContextQuery, ContextQueryResult};
pub use scope::EgressScope;
pub use token::{
    BYTES_PER_BUDGET_TOKEN, SUGGESTED_HOST_SAFETY_FACTOR, budget_from_model_tokens, budget_tokens,
};
pub use usage::{ProviderUsage, ServedFrame, UsageReport};
pub use validate::{DIGEST_ALGORITHMS, is_protocol_timestamp, is_well_formed_digest};

/// The protocol version string this crate implements. Frozen to `contextgraph/1.0`
/// only at the public v1.0 release (`SPEC.md` §Version strings).
pub const PROTOCOL_VERSION: &str = "contextgraph/1.0-draft";
