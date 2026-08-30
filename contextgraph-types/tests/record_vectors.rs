//! Cross-language reference vectors for record content addressing and record
//! attestation (lifecycle profile §3 and §7, ADR 0012).
//!
//! The sibling of [`attestation_vectors`](./attestation_vectors.rs) one layer
//! down. `record_hash` and the record attestation preimage are **normative**
//! rules, and a normative rule with no published vectors is something two
//! implementations can both believe they follow while computing different
//! hashes. These are the values an implementation in any language reconciles
//! against; if your digest matches these, your canonicalization matches the
//! profile.
//!
//! Every value here is a *fixture*, not an assertion about the current code:
//! changing the rule changes these digests, and a diff in this file is a
//! **wire-breaking change**. That is exactly why they are written out rather
//! than recomputed. What proves the rule is right rather than merely stable
//! lives elsewhere — `record_attest`'s unit tests check the canonicalizer
//! against RFC 8785's own published vectors, and
//! `contextgraph-conformance`'s `lifecycle_profile_examples` recomputes the
//! twelve profile fixtures.

#![cfg(feature = "record-attestation")]

use contextgraph_types::record_attest::{
    RECORD_ATTESTATION_DOMAIN, record_attestation_message, record_hash, record_hash_preimage,
    sign_record, verify_record_attestation, verify_signed_record_hash,
};
use contextgraph_types::{AttestationVerdict, RecordAttestation, attest::public_key_for};
use serde_json::{Value, json};

/// The published test seed — the ASCII bytes of
/// `contextgraph-lifecycle-test-key!`, the same one `tests/fixtures/`
/// publishes. It signs nothing real and is forgeable by anyone reading this.
const SEED: [u8; 32] = *b"contextgraph-lifecycle-test-key!";

/// The reference record: the profile's `observation.json` fixture, inline so
/// this vector travels inside the published crate rather than depending on a
/// file at the repository root.
fn reference_record() -> Value {
    json!({
        "schema_version": "contextgraph/lifecycle/1.0-draft",
        "record_id": "rec_obs_0001",
        "lineage_id": "lin_obs_0001",
        "record_status": "active",
        "scope": {
            "repository_id": "repo_stella",
            "workspace_id": "ws_main",
            "session_id": "sess_412"
        },
        "sharing_scope": "repository",
        "observed_at": "2026-07-29T14:00:00Z",
        "origin": "observed",
        "record_hash": "sha256:b45eebfdfe7e6e5056bf25d84864cf9acd731eef120a1f6de129fb788c3b34dc",
        "provenance": {
            "origin_provider_id": "provider_example",
            "producer_kind": "agent",
            "origin_authority_id": "authority_acme",
            "producer_ref": "agent://trace-miner"
        },
        "sensitivity": "internal",
        "confidence": 0.82,
        "record_kind": "observation",
        "statement": "the api handler retries three times before surfacing a 502",
        "subject_ref": "trace_run_991"
    })
}

/// The RFC 8785 canonicalization of the reference record with its own
/// `record_hash` member removed — the exact bytes the digest is taken over.
const REFERENCE_PREIMAGE: &str = concat!(
    r#"{"confidence":0.82,"lineage_id":"lin_obs_0001","observed_at":"2026-07-29T14:00:00Z","#,
    r#""origin":"observed","provenance":{"origin_authority_id":"authority_acme","#,
    r#""origin_provider_id":"provider_example","producer_kind":"agent","#,
    r#""producer_ref":"agent://trace-miner"},"record_id":"rec_obs_0001","#,
    r#""record_kind":"observation","record_status":"active","#,
    r#""schema_version":"contextgraph/lifecycle/1.0-draft","#,
    r#""scope":{"repository_id":"repo_stella","session_id":"sess_412","workspace_id":"ws_main"},"#,
    r#""sensitivity":"internal","sharing_scope":"repository","#,
    r#""statement":"the api handler retries three times before surfacing a 502","#,
    r#""subject_ref":"trace_run_991"}"#,
);

/// The reference record's content-addressed identity.
const REFERENCE_RECORD_HASH: &str =
    "sha256:b45eebfdfe7e6e5056bf25d84864cf9acd731eef120a1f6de129fb788c3b34dc";

/// The Ed25519 public key [`SEED`] produces, lowercase hex.
const REFERENCE_PUBLIC_KEY: &str =
    "495b4a0a4a16c5444d8626a7ae0bc6eca613676b51fb947238cb8238baa9fde5";

/// The detached signature over [`REFERENCE_RECORD_HASH`] under [`SEED`].
/// Ed25519 is deterministic (RFC 8032), so this is reproducible everywhere.
const REFERENCE_SIGNATURE: &str = concat!(
    "8cce3f453510c50d88821eb57dd1767827ba7ab5e29d072b1fadf29583313a04",
    "635521228a62b3015399ca8676394087a2bb861a4f893ff4912dea43fdccb905",
);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn the_canonical_preimage_is_these_exact_bytes() {
    let preimage = record_hash_preimage(&reference_record()).expect("canonicalizes");
    assert_eq!(
        String::from_utf8(preimage).expect("JCS output is UTF-8"),
        REFERENCE_PREIMAGE
    );
}

#[test]
fn the_record_hash_is_this_exact_digest() {
    assert_eq!(
        record_hash(&reference_record()).expect("canonicalizes"),
        REFERENCE_RECORD_HASH
    );
}

#[test]
fn the_signed_message_is_the_domain_tag_then_the_digest() {
    let message = record_attestation_message(REFERENCE_RECORD_HASH).expect("well-formed digest");
    assert_eq!(
        hex(&message),
        format!(
            "{}{}",
            hex(RECORD_ATTESTATION_DOMAIN),
            REFERENCE_RECORD_HASH
                .strip_prefix("sha256:")
                .expect("the digest names its algorithm")
        )
    );
}

#[test]
fn the_published_key_and_signature_are_these_exact_values() {
    assert_eq!(hex(&public_key_for(&SEED)), REFERENCE_PUBLIC_KEY);
    let attestation = sign_record(
        &reference_record(),
        &SEED,
        "cep-signing-key-2026-07",
        "provider_example",
        "2026-07-29T14:00:05Z",
    )
    .expect("the reference record hashes");
    assert_eq!(attestation.signature, REFERENCE_SIGNATURE);
    assert_eq!(attestation.signed_record_hash, REFERENCE_RECORD_HASH);
}

#[test]
fn the_published_signature_verifies_and_only_over_this_record() {
    let attestation = RecordAttestation::new(
        REFERENCE_RECORD_HASH,
        "cep-signing-key-2026-07",
        "ed25519",
        "provider_example",
        REFERENCE_SIGNATURE,
        "2026-07-29T14:00:05Z",
    );
    let key = public_key_for(&SEED);
    assert_eq!(
        verify_record_attestation(&reference_record(), &attestation, &key).expect("hashes"),
        AttestationVerdict::Valid
    );
    assert_eq!(
        verify_signed_record_hash(REFERENCE_RECORD_HASH, &attestation, &key),
        AttestationVerdict::Valid
    );

    let mut edited = reference_record();
    edited["statement"] = json!("the api handler never retries");
    assert!(matches!(
        verify_record_attestation(&edited, &attestation, &key).expect("hashes"),
        AttestationVerdict::CommitmentMismatch { .. }
    ));
}
