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
//!   4. **Canonical bytes.** `record-hash-vectors.json` pins the exact JCS
//!      preimage of every record fixture, recomputed here. A hash that
//!      disagrees between two implementations says nothing about *where* they
//!      diverged; a byte diff of the preimage says it immediately, which is why
//!      the vectors carry the text and not only the digest (profile LF1).
//!   5. **A verifiable attestation.** `record-attestation.json` carries a real
//!      detached Ed25519 signature, and this suite verifies it against the
//!      public key published beside it in `record-attestation-key.json`. It
//!      used to carry a placeholder no code could check — a shape example
//!      standing in for a vector.
//!
//! Every recomputation here calls the **library** —
//! [`contextgraph_types::record_attest`] — rather than a copy of the rule kept
//! in the test. A second implementation living in the suite that checks the
//! first is how a fixture set ends up agreeing with nothing that ships.
//!
//! `tests/fixtures/` is the **canonical home** for lifecycle-profile example
//! records (resolving the draft's open "which repo owns the vectors" question).
//!
//! Regenerating: `REGENERATE_LIFECYCLE_HASHES=1 cargo test -p
//! contextgraph-conformance --test lifecycle_profile_examples` rewrites each
//! fixture's `record_hash`, the attestation's `signed_record_hash` and
//! signature, and the whole vector file, then re-run without the env var to
//! verify.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use contextgraph_types::record_attest::{
    record_hash, record_hash_preimage, sign_record_attestation, verify_signed_record_hash,
};
use contextgraph_types::{
    AttestationVerdict, ContextRecord, LIFECYCLE_SCHEMA_VERSION, RecordAttestation,
};

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

/// The detached `RecordAttestation` example (profile LC3).
const ATTESTATION_FIXTURE: &str = "record-attestation.json";
/// The published test key the attestation example is signed under.
const ATTESTATION_KEY_FIXTURE: &str = "record-attestation-key.json";
/// The canonical JCS preimage and hash of every record fixture (profile LF1).
const HASH_VECTORS_FIXTURE: &str = "record-hash-vectors.json";

/// The fixtures in `tests/fixtures/` that are **not** lifecycle records.
///
/// An explicit list rather than a filename convention: every other file in the
/// directory is named for its `record_kind`, and a new non-record fixture must
/// be a deliberate entry here rather than something a glob quietly swallows.
const NON_RECORD_FIXTURES: [&str; 3] = [
    ATTESTATION_FIXTURE,
    ATTESTATION_KEY_FIXTURE,
    HASH_VECTORS_FIXTURE,
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("fixtures")
}

/// Every `*.json` fixture except the non-record ones — i.e. the record
/// fixtures, one per `record_kind`.
fn record_fixture_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("tests/fixtures is readable")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                !NON_RECORD_FIXTURES
                    .iter()
                    .any(|excluded| name == std::ffi::OsStr::new(excluded))
            })
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

/// The content-addressed `record_hash` — the library's rule, not a copy of it
/// (profile LH1, LF3).
fn compute_hash(value: &serde_json::Value) -> String {
    record_hash(value).expect("record canonicalizes under JCS")
}

/// Read and parse a fixture, naming it in the panic so a failure says which.
fn read_json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
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
        let expected = compute_hash(&value);

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

/// The canonical bytes every record fixture hashes over (profile LF1).
///
/// The digest alone is a poor interop artifact: when a third-party CEP computes
/// a different one, the digest cannot say whether the difference is in a number,
/// a member order, or an escape. The preimage text can, so it is what the
/// vectors carry.
#[test]
fn the_hash_vectors_pin_the_canonical_preimage_of_every_record_fixture() {
    let vectors_path = fixtures_dir().join(HASH_VECTORS_FIXTURE);
    let mut vectors = Vec::new();
    for path in record_fixture_paths() {
        let value = read_json(&path);
        let preimage = record_hash_preimage(&value).expect("record canonicalizes under JCS");
        let jcs = String::from_utf8(preimage).expect("JCS output is UTF-8 by definition");
        vectors.push(serde_json::json!({
            "record_file": path.file_name().unwrap().to_string_lossy(),
            "jcs_utf8": jcs,
            "record_hash": compute_hash(&value),
        }));
    }

    let rebuilt = serde_json::json!({
        "note": "Golden RFC 8785 (JCS) preimages and record_hash values for the \
                 lifecycle-profile record fixtures beside this file. Recomputed by \
                 contextgraph-conformance's lifecycle_profile_examples suite; \
                 regenerate with REGENERATE_LIFECYCLE_HASHES=1.",
        "rule": "record_hash = \"sha256:\" + hex(sha256(JCS(record with its top-level \
                 record_hash member removed)))",
        "vectors": vectors,
    });

    if regenerating() {
        let mut text = serde_json::to_string_pretty(&rebuilt).expect("vectors serialize");
        text.push('\n');
        std::fs::write(&vectors_path, text).expect("write vectors");
        eprintln!("regenerated {}", vectors_path.display());
        return;
    }

    let stored = read_json(&vectors_path);
    assert_eq!(
        stored["vectors"],
        rebuilt["vectors"],
        "{} no longer matches the canonicalization of the fixtures beside it \
         (run with REGENERATE_LIFECYCLE_HASHES=1 to refresh)",
        vectors_path.display()
    );
}

#[test]
fn every_hash_vector_names_a_fixture_that_exists() {
    let stored = read_json(&fixtures_dir().join(HASH_VECTORS_FIXTURE));
    let named: BTreeSet<String> = stored["vectors"]
        .as_array()
        .expect("vectors is an array")
        .iter()
        .map(|vector| {
            vector["record_file"]
                .as_str()
                .expect("record_file is a string")
                .to_string()
        })
        .collect();
    let present: BTreeSet<String> = record_fixture_paths()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        named, present,
        "the vector file and the fixture directory must cover the same records"
    );
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
    assert!(attestation.uses_known_algorithm());
    assert!(attestation.has_well_formed_signed_record_hash());
    assert!(attestation.has_well_formed_issued_at());

    // It signs the observation record's hash — a coherent, cross-linked fixture
    // set. The attestation is detached: it is validated on its own, never as a
    // member of a ContextRecord.
    // Compute the observation hash directly (not by reading its stored field) so
    // this test never races the fixture that rewrites observation.json.
    let observation_hash = compute_hash(&read_json(&fixtures_dir().join("observation.json")));

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

/// The published signing key for the attestation example.
///
/// A test key, and labelled as one everywhere it appears: it is committed to a
/// public repository, so anything it signs is forgeable by anyone. Publishing it
/// is the whole point — a vector nobody can reproduce is a shape example, which
/// is exactly what this fixture used to be.
fn attestation_key() -> (serde_json::Value, [u8; 32]) {
    let key = read_json(&fixtures_dir().join(ATTESTATION_KEY_FIXTURE));
    let hex = key["signing_key_seed"]
        .as_str()
        .expect("signing_key_seed is a string");
    let mut seed = [0u8; 32];
    for (slot, pair) in seed.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let hi = (pair[0] as char).to_digit(16).expect("hex");
        let lo = (pair[1] as char).to_digit(16).expect("hex");
        *slot = (hi * 16 + lo) as u8;
    }
    (key, seed)
}

/// The attestation example is a **verifiable vector**, not a shape example.
///
/// Before this suite could check it, the fixture carried 49 bytes of
/// DER-shaped filler where an Ed25519 signature belongs — a value no
/// implementation could have reproduced or refuted, sitting in the directory the
/// profile calls the canonical home for its vectors.
#[test]
fn the_attestation_example_verifies_under_its_published_key() {
    let attestation_path = fixtures_dir().join(ATTESTATION_FIXTURE);
    let attestation: RecordAttestation = serde_json::from_value(read_json(&attestation_path))
        .expect("attestation deserializes through RecordAttestation");
    let observation_hash = compute_hash(&read_json(&fixtures_dir().join("observation.json")));
    let (key, seed) = attestation_key();

    if regenerating() {
        let regenerated = sign_record_attestation(
            &observation_hash,
            &seed,
            attestation.key_id.clone(),
            attestation.attester_id.clone(),
            attestation.issued_at.clone(),
        )
        .expect("the observation hash is a well-formed digest");
        let mut text = serde_json::to_string_pretty(&regenerated).expect("serializes");
        text.push('\n');
        std::fs::write(&attestation_path, text).expect("rewrite attestation");
        eprintln!("regenerated signature for {}", attestation_path.display());
        return;
    }

    let public_key_hex = key["public_key"].as_str().expect("public_key is a string");
    let mut public_key = [0u8; 32];
    for (slot, pair) in public_key
        .iter_mut()
        .zip(public_key_hex.as_bytes().chunks_exact(2))
    {
        let hi = (pair[0] as char).to_digit(16).expect("hex");
        let lo = (pair[1] as char).to_digit(16).expect("hex");
        *slot = (hi * 16 + lo) as u8;
    }

    assert_eq!(
        verify_signed_record_hash(&observation_hash, &attestation, &public_key),
        AttestationVerdict::Valid,
        "the published attestation must verify against the published key \
         (run with REGENERATE_LIFECYCLE_HASHES=1 to re-sign)"
    );

    // And it must fail for the right reason once the record moves under it.
    let mut edited = read_json(&fixtures_dir().join("observation.json"));
    edited["statement"] = serde_json::json!("an edit made after the record was signed");
    let verdict = verify_signed_record_hash(&compute_hash(&edited), &attestation, &public_key);
    assert!(
        matches!(verdict, AttestationVerdict::CommitmentMismatch { .. }),
        "editing the signed record must be caught as a mismatch, got {verdict:?}"
    );
}

/// The key fixture must describe the key it publishes, or it is decoration.
#[test]
fn the_published_key_fixture_agrees_with_its_own_seed() {
    let (key, seed) = attestation_key();
    let derived = contextgraph_types::attest::public_key_for(&seed);
    let derived_hex: String = derived.iter().map(|byte| format!("{byte:02x}")).collect();

    if regenerating() {
        let observation_hash = compute_hash(&read_json(&fixtures_dir().join("observation.json")));
        let message = contextgraph_types::record_attestation_message(&observation_hash)
            .expect("the observation hash is a well-formed digest");
        let message_hex: String = message.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut rebuilt = key.clone();
        rebuilt["public_key"] = serde_json::json!(derived_hex);
        rebuilt["signed_message_hex"] = serde_json::json!(message_hex);
        let mut text = serde_json::to_string_pretty(&rebuilt).expect("serializes");
        text.push('\n');
        std::fs::write(fixtures_dir().join(ATTESTATION_KEY_FIXTURE), text).expect("rewrite key");
        eprintln!("regenerated {ATTESTATION_KEY_FIXTURE}");
        return;
    }

    assert_eq!(
        key["public_key"].as_str(),
        Some(derived_hex.as_str()),
        "the published public key is not the one this seed produces"
    );

    let observation_hash = compute_hash(&read_json(&fixtures_dir().join("observation.json")));
    let message = contextgraph_types::record_attestation_message(&observation_hash)
        .expect("the observation hash is a well-formed digest");
    let message_hex: String = message.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(
        key["signed_message_hex"].as_str(),
        Some(message_hex.as_str()),
        "the published signed message is not the domain tag followed by the \
         observation record's hash"
    );
}
