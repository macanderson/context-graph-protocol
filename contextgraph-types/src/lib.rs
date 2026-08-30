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
//! Protocol version: `contextgraph/1.0`.

pub mod attest;
pub mod attribution;
pub mod capability;
pub mod consent;
pub mod error_code;
pub mod frame;
pub mod identity;
pub mod query;
pub mod record;
pub mod record_attest;
pub mod scope;
pub mod token;
pub mod usage;
pub mod validate;
pub mod verify;

pub use attest::{
    ALGORITHM_ED25519, AttestationVerdict, FrameAttestation, InclusionProof, InclusionStep,
    ProvenanceAttestation, digest_string, encode_provenance_link,
};
#[cfg(feature = "attestation")]
pub use attest::{
    frame_commitment, inclusion_proof, merkle_root, provenance_chain_head, public_key_for,
    result_set_commitments, result_set_root, root_from_proof, sign_commitment,
    sign_frame_attestation, verify_commitment, verify_frame_attestation,
};
pub use attribution::{AttributionReport, ContextUse};
pub use capability::{
    Capabilities, DataFlow, ProviderInfo, QueryCapability, embedding_fingerprints_match,
    fingerprint_dimensions,
};
pub use consent::{ConsentReceipt, Grantor};
pub use error_code::{ErrorCode, HostReaction};
pub use frame::{
    ContentFidelity, ContentRef, ContextFrame, FrameEmbedding, FrameKind, InlineContentRequirement,
    Provenance, Relation, Representation, Transform, rel,
};
pub use identity::{FrameId, canonical_order};
pub use query::{ContextQuery, ContextQueryResult};
pub use record::{
    ConstraintEffect, ContextRecord, ContractRequirement, DirectiveKind, Enforcement,
    KnowledgeKind, LIFECYCLE_SCHEMA_VERSION, OriginClass, RecordAttestation, RecordBody,
    RecordLink, RecordProvenance, RecordScope, RecordStatus, RequirementResult, SharingScope,
    ValidationOutcome,
};
pub use record_attest::{
    RECORD_ATTESTATION_DOMAIN, RECORD_HASH_MEMBER, RecordHashError, record_attestation_message,
};
#[cfg(feature = "record-hash")]
pub use record_attest::{
    record_hash, record_hash_is_current, record_hash_of, record_hash_preimage,
};
#[cfg(feature = "record-attestation")]
pub use record_attest::{
    sign_record, sign_record_attestation, verify_record_attestation, verify_signed_record_hash,
};
pub use scope::EgressScope;
pub use token::{
    BYTES_PER_BUDGET_TOKEN, SUGGESTED_HOST_SAFETY_FACTOR, budget_from_model_tokens, budget_tokens,
};
pub use usage::{ProviderUsage, ServedFrame, UsageReport};
pub use validate::{
    DIGEST_ALGORITHMS, format_protocol_timestamp, is_protocol_timestamp, is_well_formed_digest,
};
pub use verify::{FrameVerdict, Verdict, VerifyRequest, VerifyResponse};

/// The stable protocol version string this crate implements (`SPEC.md` §3.1).
pub const PROTOCOL_VERSION: &str = "contextgraph/1.0";
