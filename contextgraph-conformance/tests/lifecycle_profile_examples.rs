//! The lifecycle-profile fixtures, the JSON Schema, and the Rust record types
//! must agree — the record-layer analogue of `examples_roundtrip.rs`.
//!
//! `schema/validate-examples.py` proves each `tests/fixtures/*.json` record
//! satisfies `schema/contextgraph-lifecycle-record.schema.json`. That is only
//! part of the contract. This suite closes the loop three ways:
//!
//!   1. **Round-trip.** Every fixture deserializes through
//!      [`contextgraph_types::ContextRecord`] and survives a serde round-trip,
//!      so a wire-type change that skips the fixtures turns a PR red (the record
//!      analogue of issue #2).
//!   2. **Envelope invariants.** Each record passes
//!      [`ContextRecord::envelope_invariants`] — schema_version, the record_hash
//!      grammar, the confidence range, the origin→derivation matrix, and the
//!      "a constraint directive states its effect" rule (reconciliation rows
//!      B3/B5/C5/E3).
//!   3. **Content-addressed hash.** `record_hash` is recomputed as
//!      `sha256:<hex>` over the RFC 8785 (JCS) canonicalization of the record
//!      with its own `record_hash` member removed, and must match the stored
//!      value. This is what makes the fixtures a golden vector for the hashing
//!      rule rather than a hash a fixture merely asserts about itself.
//!
//! `tests/fixtures/` is the **canonical home** for lifecycle-profile example
//! records (resolving the draft's open "which repo owns the vectors" question).
//!
//! Regenerating the hashes: `REGENERATE_LIFECYCLE_HASHES=1 cargo test -p
//! contextgraph-conformance --test lifecycle_profile_examples` rewrites each
//! fixture's `record_hash` (and the attestation's `signed_record_hash`) to the
//! recomputed value, preserving the file's field order.

use std::collections::BTreeSet;
use std::path::PathBuf;

use contextgraph_types::{ContextRecord, LIFECYCLE_SCHEMA_VERSION, RecordAttestation};
use sha2::{Digest, Sha256};

/// The 12 portable record kinds the profile defines (reconciliation row D1).
const EXPECTED_KINDS: [&str; 12] = [
    "observation",
    "knowledge",
    "memory",
    "directive",
    "record_proposal",
    "evidence",
    "artifact_contract",
    "contract_validation",
    "outcome_assessment",
    "promotion_event",
    "context_use",
    "context_use_feedback",
];

const ATTESTATION_FIXTURE: &str = "record-attestation.json";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("fixtures")
}

/// Every `*.json` fixture except the detached attestation — i.e. the record
/// fixtures, one per `record_kind`.
fn record_fixture_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("tests/fixtures is readable")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != ATTESTATION_FIXTURE)
        })
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no lifecycle record fixtures found under {}",
        fixtures_dir().display()
    );
    paths
}

/// The content-addressed `record_hash`: `sha256:<hex>` over the RFC 8785 (JCS)
/// canonicalization of the record with `record_hash` (or, for the detached
/// attestation, `signed_record_hash`) omitted from the preimage.
fn compute_hash(value: &serde_json::Value, hash_member: &str) -> String {
    let mut preimage = value.clone();
    preimage
        .as_object_mut()
        .expect("a record is a JSON object")
        .remove(hash_member);
    let canonical =
        serde_json_canonicalizer::to_vec(&preimage).expect("record canonicalizes under JCS");
    let hex: String = Sha256::digest(&canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn regenerating() -> bool {
    std::env::var_os("REGENERATE_LIFECYCLE_HASHES").is_some()
}

/// Replace the single `"<member>": "sha256:…"` value in `text` with `new`,
/// preserving the file's field order and formatting.
fn rewrite_hash(text: &str, old: &str, new: &str) -> String {
    assert!(
        text.matches(old).count() == 1,
        "expected exactly one occurrence of {old} to rewrite"
    );
    text.replacen(old, new, 1)
}

#[test]
fn every_record_kind_has_exactly_one_fixture() {
    let kinds: BTreeSet<String> = record_fixture_paths()
        .iter()
        .map(|path| {
            let raw = std::fs::read_to_string(path).expect("fixture readable");
            let value: serde_json::Value =
                serde_json::from_str(&raw).expect("fixture is valid JSON");
            value["record_kind"]
                .as_str()
                .unwrap_or_else(|| panic!("{} has no record_kind", path.display()))
                .to_string()
        })
        .collect();

    let expected: BTreeSet<String> = EXPECTED_KINDS.iter().map(|k| k.to_string()).collect();
    assert_eq!(
        kinds, expected,
        "tests/fixtures must hold exactly one fixture per record_kind"
    );
}

#[test]
fn every_fixture_round_trips_and_satisfies_its_envelope_invariants() {
    for path in record_fixture_paths() {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));

        let record: ContextRecord = serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!(
                "{} does not deserialize into ContextRecord: {e}\n\
                 The schema, the fixtures, and the Rust types describe one record \
                 layer — if you changed a record type, update the fixtures in the \
                 same commit.",
                path.display()
            )
        });

        assert_eq!(record.schema_version, LIFECYCLE_SCHEMA_VERSION);
        record.envelope_invariants().unwrap_or_else(|e| {
            panic!(
                "{} violates the profile envelope invariants: {e}",
                path.display()
            )
        });

        // The filename stem is the record_kind, so the fixture set is
        // self-documenting.
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(
            record.record_kind(),
            stem,
            "{} should carry record_kind == its filename",
            path.display()
        );

        // Round-trip: re-serializing must produce something the types still
        // accept, catching an asymmetric Serialize/Deserialize impl.
        let reencoded = serde_json::to_value(&record).expect("record re-serializes");
        let back: ContextRecord =
            serde_json::from_value(reencoded).expect("re-serialized record re-parses");
        assert_eq!(
            back,
            record,
            "{} did not survive a serde round-trip",
            path.display()
        );
    }
}

#[test]
fn record_hash_is_the_jcs_sha256_of_the_hashless_record() {
    let regenerate = regenerating();
    for path in record_fixture_paths() {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
        let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        let stored = value["record_hash"]
            .as_str()
            .expect("record_hash present")
            .to_string();
        let expected = compute_hash(&value, "record_hash");

        if regenerate {
            if stored != expected {
                let rewritten = rewrite_hash(&raw, &stored, &expected);
                std::fs::write(&path, rewritten).expect("rewrite fixture");
                eprintln!("regenerated record_hash for {}", path.display());
            }
        } else {
            assert_eq!(
                stored,
                expected,
                "{} carries a record_hash that is not the JCS-sha256 of its hashless \
                 form (run with REGENERATE_LIFECYCLE_HASHES=1 to refresh)",
                path.display()
            );
        }
    }
}

#[test]
fn the_detached_attestation_round_trips_and_signs_the_observation_record() {
    let attestation_path = fixtures_dir().join(ATTESTATION_FIXTURE);
    let raw = std::fs::read_to_string(&attestation_path).expect("attestation readable");

    // It deserializes through the dedicated detached type.
    let attestation: RecordAttestation =
        serde_json::from_str(&raw).expect("attestation deserializes through RecordAttestation");
    let reencoded = serde_json::to_value(&attestation).expect("re-serializes");
    let back: RecordAttestation = serde_json::from_value(reencoded).expect("re-parses");
    assert_eq!(back, attestation);

    // It signs the observation record's hash — a coherent, cross-linked fixture
    // set. The attestation is detached: it is validated on its own, never as a
    // member of a ContextRecord.
    // Compute the observation hash directly (not by reading its stored field) so
    // this test never races the fixture that rewrites observation.json.
    let observation: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures_dir().join("observation.json")).expect("readable"),
    )
    .expect("valid JSON");
    let observation_hash = compute_hash(&observation, "record_hash");

    if regenerating() {
        if attestation.signed_record_hash != observation_hash {
            let rewritten = rewrite_hash(&raw, &attestation.signed_record_hash, &observation_hash);
            std::fs::write(&attestation_path, rewritten).expect("rewrite attestation");
            eprintln!(
                "regenerated signed_record_hash for {}",
                attestation_path.display()
            );
        }
    } else {
        assert_eq!(
            attestation.signed_record_hash, observation_hash,
            "the example attestation should sign the observation fixture's record_hash"
        );
    }
}
