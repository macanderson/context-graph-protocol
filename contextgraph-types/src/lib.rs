//! `contextgraph-types` — the Context Graph Protocol wire types.
//!
//! This crate is the industry-facing artifact: **MIT licensed, zero
//! dependencies beyond `serde`**, publishable to crates.io on its own so a
//! third party can implement an Context Graph Protocol host or provider without pulling in any
//! Stella code. See `docs/specs/stella-rust-cli/06-context-protocol.md` §3
//! for the normative shape this crate binds to Rust types.
//!
//! Protocol version: `contextgraph/1.0-draft`.

pub mod capability;
pub mod consent;
pub mod frame;
pub mod identity;
pub mod query;
pub mod scope;
pub mod usage;
pub mod verify;

pub use capability::{Capabilities, DataFlow, ProviderInfo};
pub use consent::{ConsentReceipt, Grantor};
pub use frame::{ContextFrame, FrameKind, Provenance, Relation};
pub use identity::{FrameId, canonical_order};
pub use query::{ContextQuery, ContextQueryResult};
pub use scope::EgressScope;
pub use usage::{ProviderUsage, ServedFrame, UsageReport};
pub use verify::{FrameVerdict, Verdict, VerifyRequest, VerifyResponse};

/// The protocol version string this crate implements. Frozen to `contextgraph/1.0`
/// only at the public v1.0 release (`06-context-protocol.md` §3).
pub const PROTOCOL_VERSION: &str = "contextgraph/1.0-draft";
