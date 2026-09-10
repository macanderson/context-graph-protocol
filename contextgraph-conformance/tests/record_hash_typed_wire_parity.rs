//! Hashing a record as a **value** and hashing it as a **type** must produce
//! the same digest, for every lifecycle fixture — issue #118.
//!
//! `record_hash(&Value)` is the normative primitive (profile LH1): it hashes
//! exactly the bytes it is handed. `record_hash_of(&ContextRecord)` is the
//! convenience a host actually reaches for once it has parsed a record, and it
//! hashes what the reference types *re-serialize*. Those are the same digest
//! only for as long as `ContextRecord` can hold everything the wire carries.
//! When it cannot, the two disagree and neither call reports a problem — a
//! wrong content address returned as a success, which is the worst shape a
//! defect can take in a content-addressed protocol.
//!
//! So the agreement is checked here rather than documented:
//!
//!   1. **Every fixture, both ways.** `record_hash(wire) ==
//!      record_hash_of(typed)` for each of the twelve profile fixtures, and the
//!      digest equals the `record_hash` the fixture stores — so this suite
//!      cannot drift into comparing two equally-wrong values.
//!   2. **Every fixture, extended.** The same equality with a namespaced member
//!      (`SPEC.md` §13 U3) injected that no reference type models — the exact
//!      record shape that used to lose the member and hash as though it were
//!      never there.
//!   3. **The member is content.** An extended record must hash *differently*
//!      from its unextended self, or the parity above would be satisfiable by
//!      dropping the member on both sides.
//!
//! The injection is in-memory only. `schema/contextgraph-lifecycle-record.schema.json`
//! is deliberately authoring-strict (`unevaluatedProperties: false`, see its
//! `$comment`) so a typo in a committed fixture is caught; that strictness is a
//! lint on what this repository authors, not the interop contract, which
//! `SPEC.md` §13 U1 states the other way round — a receiver **MUST** ignore
//! members it does not recognise. Writing the member to disk would fail the
//! schema check for no gain.

use std::path::PathBuf;

use contextgraph_types::ContextRecord;
use contextgraph_types::record_attest::{record_hash, record_hash_of};
use serde_json::{Value, json};

/// The fixtures in `tests/fixtures/` that are **not** lifecycle records. Kept
/// in step with `lifecycle_profile_examples.rs`, which owns the same list for
/// the same directory.
const NON_RECORD_FIXTURES: [&str; 3] = [
    "record-attestation.json",
    "record-attestation-key.json",
    "record-hash-vectors.json",
];

/// The 12 portable record kinds — asserted, so a fixture that disappears makes
/// this suite fail rather than quietly check fewer records.
const EXPECTED_FIXTURE_COUNT: usize = 12;

fn record_fixture_paths() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()))
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
    assert_eq!(
        paths.len(),
        EXPECTED_FIXTURE_COUNT,
        "expected one fixture per portable record_kind under {}",
        dir.display()
    );
    paths
}

fn read_fixture(path: &PathBuf) -> Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

/// A namespaced member no reference type models (`SPEC.md` §13 U3), spanning
/// every JSON shape — an integer and a float in particular, because the two
/// print differently and a value model that widened `2` to `2.0` would change
/// the digest without changing the record.
fn extend(wire: &Value) -> Value {
    let mut extended = wire.clone();
    extended.as_object_mut().expect("a record is an object").insert(
        "acme:relay".to_string(),
        json!({
            "hops": 2,
            "latency_ms": 12.5,
            "verified": true,
            "route": ["provider_a", "provider_b"],
            "note": null
        }),
    );
    extended
}

#[test]
fn every_fixture_hashes_the_same_as_a_value_and_as_a_type() {
    for path in record_fixture_paths() {
        let name = path.file_name().expect("named").to_string_lossy().to_string();
        let wire = read_fixture(&path);

        let record: ContextRecord = serde_json::from_value(wire.clone())
            .unwrap_or_else(|e| panic!("{name} does not parse as a ContextRecord: {e}"));

        let from_value = record_hash(&wire).unwrap_or_else(|e| panic!("{name}: {e}"));
        let from_type = record_hash_of(&record).unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(
            from_type, from_value,
            "{name}: hashing the parsed record disagrees with hashing its wire bytes"
        );

        let stored = wire
            .get("record_hash")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name} has no record_hash"));
        assert_eq!(
            from_value, stored,
            "{name}: the recomputed hash is not the one the fixture stores — the \
             parity above would otherwise be two equally-wrong values agreeing"
        );
    }
}

#[test]
fn every_fixture_still_agrees_carrying_a_member_no_reference_type_models() {
    for path in record_fixture_paths() {
        let name = path.file_name().expect("named").to_string_lossy().to_string();
        let extended = extend(&read_fixture(&path));

        let record: ContextRecord = serde_json::from_value(extended.clone())
            .unwrap_or_else(|e| panic!("extended {name} does not parse: {e}"));

        assert!(
            record.extra.contains_key("acme:relay"),
            "extended {name}: the unmodelled member was dropped on the way in"
        );
        assert_eq!(
            serde_json::to_value(&record).expect("re-serializes"),
            extended,
            "extended {name}: the record does not re-serialize to the bytes it was parsed from"
        );
        assert_eq!(
            record_hash_of(&record).unwrap_or_else(|e| panic!("{name}: {e}")),
            record_hash(&extended).unwrap_or_else(|e| panic!("{name}: {e}")),
            "extended {name}: a relayed extension member changes the typed digest \
             but not the wire digest — the defect in issue #118"
        );
    }
}

#[test]
fn an_extension_member_is_content_and_moves_the_content_address() {
    for path in record_fixture_paths() {
        let name = path.file_name().expect("named").to_string_lossy().to_string();
        let wire = read_fixture(&path);
        assert_ne!(
            record_hash(&extend(&wire)).unwrap_or_else(|e| panic!("{name}: {e}")),
            record_hash(&wire).unwrap_or_else(|e| panic!("{name}: {e}")),
            "{name}: an extension member must be part of the preimage"
        );
    }
}
