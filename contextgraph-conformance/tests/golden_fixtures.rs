use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use contextgraph_conformance::check_frames;
use contextgraph_types::{
    ContextFrame, ContextQuery, ContextQueryResult, PROTOCOL_VERSION, Representation,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const PROFILE: &str = "contextgraph-1.0-draft";
const PROFILE_VERSION: &str = "1.1.0";
const GENERATION_COMMAND: &str = "cargo test -p contextgraph-conformance --test golden_fixtures";
const FIXTURE_FILES: [&str; 7] = [
    "context-frame.compact.valid.json",
    "context-frame.missing-citation.invalid.json",
    "context-frame.reference.valid.json",
    "context-frame.valid.json",
    "context-query.valid.json",
    "normalization-vectors.json",
    "strict-validation.invalid.json",
];
// The pinned `contextgraph-1.0-draft` strict frame profile. `content_digest` is
// intentionally excluded here as it was before the representation work; the nine
// representation/cost fields below are additive and default-absent.
const FRAME_FIELDS: [&str; 23] = [
    "id",
    "kind",
    "title",
    "content",
    "uri",
    "representation",
    "content_fidelity",
    "canonical_content_hash",
    "content_ref",
    "transform",
    "minimum_content_fidelity",
    "inline_content_requirement",
    "score",
    "token_cost",
    "canonical_token_cost",
    "tokenizer_ref",
    "valid_from",
    "valid_to",
    "recorded_at",
    "provenance",
    "citation_label",
    "embedding",
    "relations",
];
const PROVENANCE_FIELDS: [&str; 6] = ["type", "uri", "range", "digest", "method", "by"];
const EMBEDDING_FIELDS: [&str; 2] = ["fingerprint", "vector"];
const RELATION_FIELDS: [&str; 3] = ["rel", "target_uri", "display_name"];
const QUERY_FIELDS: [&str; 8] = [
    "goal",
    "query_text",
    "embedding",
    "kinds",
    "anchors",
    "max_frames",
    "max_tokens",
    "as_of",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol_version: String,
    fixture_profile_version: String,
    generation_command: String,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DigestFixtures {
    cases: Vec<DigestCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DigestCase {
    name: String,
    source: Value,
    expected_normalized: Value,
    expected_jcs_utf8: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CitationFixtures {
    missing: Value,
    blank: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictValidationFixtures {
    cases: Vec<StrictValidationCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictValidationCase {
    name: String,
    target: StrictTarget,
    input: Value,
    unknown_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StrictTarget {
    Frame,
    Query,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizationFixtures {
    vectors: Vec<NormalizationVector>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizationVector {
    name: String,
    input_json: String,
    expected_normalized: Value,
    expected_jcs_utf8: String,
    sha256: String,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(PROFILE)
}

fn read_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixture_dir().join(name);
    let bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

fn assert_lowercase_sha256(digest: &str) {
    let hex = digest
        .strip_prefix("sha256:")
        .unwrap_or_else(|| panic!("digest lacks sha256: prefix: {digest}"));
    assert_eq!(hex.len(), 64, "digest must contain 32 bytes: {digest}");
    assert!(
        hex.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "digest must use lowercase hexadecimal: {digest}"
    );
}

fn fixture_file_names(directory: &Path) -> BTreeSet<String> {
    fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .map(|entry| entry.expect("fixture directory entry must be readable"))
        .filter(|entry| {
            entry
                .file_type()
                .expect("fixture file type must be readable")
                .is_file()
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{path} must be an object"))
}

fn reject_unknown_fields(value: &Value, allowed: &[&str], path: &str) -> Result<(), String> {
    for key in object(value, path)?.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown field {path}.{key}"));
        }
    }
    Ok(())
}

fn strict_frame(value: &Value) -> Result<ContextFrame, String> {
    reject_unknown_fields(value, &FRAME_FIELDS, "frame")?;
    let frame = object(value, "frame")?;

    if let Some(provenance) = frame.get("provenance") {
        let entries = provenance
            .as_array()
            .ok_or_else(|| "frame.provenance must be an array".to_owned())?;
        for (index, entry) in entries.iter().enumerate() {
            reject_unknown_fields(
                entry,
                &PROVENANCE_FIELDS,
                &format!("frame.provenance[{index}]"),
            )?;
        }
    }
    if let Some(embedding) = frame.get("embedding") {
        reject_unknown_fields(embedding, &EMBEDDING_FIELDS, "frame.embedding")?;
    }
    if let Some(relations) = frame.get("relations") {
        let entries = relations
            .as_array()
            .ok_or_else(|| "frame.relations must be an array".to_owned())?;
        for (index, entry) in entries.iter().enumerate() {
            reject_unknown_fields(
                entry,
                &RELATION_FIELDS,
                &format!("frame.relations[{index}]"),
            )?;
        }
    }

    serde_json::from_value(value.clone()).map_err(|error| format!("invalid frame: {error}"))
}

fn strict_query(value: &Value) -> Result<ContextQuery, String> {
    reject_unknown_fields(value, &QUERY_FIELDS, "query")?;
    serde_json::from_value(value.clone()).map_err(|error| format!("invalid query: {error}"))
}

fn normalize_frame(value: &Value) -> Result<Value, String> {
    strict_frame(value)?;
    let mut normalized = value.clone();
    let frame = normalized
        .as_object_mut()
        .expect("strict frame validation requires an object");
    frame
        .entry("provenance")
        .or_insert_with(|| Value::Array(Vec::new()));
    frame
        .entry("relations")
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(normalized)
}

fn normalize_query(value: &Value) -> Result<Value, String> {
    strict_query(value)?;
    let mut normalized = value.clone();
    let query = normalized
        .as_object_mut()
        .expect("strict query validation requires an object");
    query
        .entry("kinds")
        .or_insert_with(|| Value::Array(Vec::new()));
    query
        .entry("anchors")
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(normalized)
}

fn assert_digest_case(case: &DigestCase, normalized: &Value) {
    assert_eq!(normalized, &case.expected_normalized, "{} value", case.name);
    let canonical = serde_json_canonicalizer::to_string(normalized)
        .unwrap_or_else(|error| panic!("{} could not be canonicalized: {error}", case.name));
    assert_eq!(canonical, case.expected_jcs_utf8, "{} JCS text", case.name);
    assert_lowercase_sha256(&case.sha256);
    assert_eq!(
        sha256_digest(case.expected_jcs_utf8.as_bytes()),
        case.sha256,
        "{} JCS digest",
        case.name
    );
}

fn assert_rejected_citation(frame: ContextFrame, label: &str) {
    let result = ContextQueryResult {
        frames: vec![frame],
        truncated: false,
        dropped_estimate: None,
    };
    let (passed, evidence) = check_frames(&result);
    assert!(!passed, "{label} citation unexpectedly passed conformance");
    assert!(
        evidence.contains("citation_label"),
        "unexpected evidence for {label} citation: {evidence}"
    );
}

#[test]
fn manifest_has_strict_coverage_and_correct_file_hashes() {
    let directory = fixture_dir();
    let manifest: Manifest = read_fixture("manifest.json");

    assert_eq!(manifest.protocol_version, PROTOCOL_VERSION);
    assert_eq!(manifest.fixture_profile_version, PROFILE_VERSION);
    assert_eq!(manifest.generation_command, GENERATION_COMMAND);

    let declared: BTreeSet<_> = manifest.files.keys().cloned().collect();
    let expected: BTreeSet<_> = FIXTURE_FILES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(
        declared, expected,
        "manifest must declare every profile fixture"
    );

    let mut on_disk = fixture_file_names(&directory);
    assert!(on_disk.remove("manifest.json"));
    assert_eq!(
        on_disk, declared,
        "manifest coverage must exactly match disk"
    );

    for (name, expected_digest) in manifest.files {
        assert_lowercase_sha256(&expected_digest);
        let bytes = fs::read(directory.join(&name))
            .unwrap_or_else(|error| panic!("failed to read {name}: {error}"));
        assert_eq!(sha256_digest(&bytes), expected_digest, "hash for {name}");
    }
}

#[test]
fn frame_digest_cases_pin_default_materialization_jcs_and_array_order() {
    let fixtures: DigestFixtures = read_fixture("context-frame.valid.json");
    let names: BTreeSet<_> = fixtures
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["fully_populated_frame", "minimal_frame"])
    );

    let mut frames = Vec::new();
    for case in &fixtures.cases {
        let frame = strict_frame(&case.source)
            .unwrap_or_else(|error| panic!("{} source was rejected: {error}", case.name));
        assert_eq!(serde_json::to_value(&frame).unwrap(), case.source);
        let normalized = normalize_frame(&case.source).unwrap();
        assert_digest_case(case, &normalized);
        frames.push(frame);
    }

    let full = fixtures
        .cases
        .iter()
        .find(|case| case.name == "fully_populated_frame")
        .unwrap();
    for field in ["provenance", "relations"] {
        assert_eq!(
            full.source.get(field),
            full.expected_normalized.get(field),
            "{field} order changed"
        );
    }
    assert_eq!(
        full.source.pointer("/embedding/vector"),
        full.expected_normalized.pointer("/embedding/vector"),
        "embedding vector order changed"
    );

    let minimal = fixtures
        .cases
        .iter()
        .find(|case| case.name == "minimal_frame")
        .unwrap();
    assert!(minimal.source.get("provenance").is_none());
    assert!(minimal.source.get("relations").is_none());
    assert_eq!(
        minimal.expected_normalized["provenance"],
        Value::Array(vec![])
    );
    assert_eq!(
        minimal.expected_normalized["relations"],
        Value::Array(vec![])
    );
    for optional in ["uri", "valid_from", "valid_to", "recorded_at", "embedding"] {
        assert!(
            minimal.expected_normalized.get(optional).is_none(),
            "optional scalar {optional} must remain absent"
        );
    }

    let result = ContextQueryResult {
        frames,
        truncated: false,
        dropped_estimate: None,
    };
    let (passed, evidence) = check_frames(&result);
    assert!(passed, "valid golden frames were rejected: {evidence}");
}

#[test]
fn representation_vectors_are_honest_and_structurally_valid() {
    // The compact/reference goldens added for issue #52. They pin the §6.4
    // representation surface (P1–P5) and the §B3 canonical cost that the
    // pre-representation `context-frame.valid.json` predated — the exact gap
    // downstreams were papering over with local, unattested vectors.
    //
    // These are parsed as real `ContextFrame`s rather than through
    // `strict_frame`: a `compact` frame carries `content_digest`, which the
    // frozen `contextgraph-1.0-draft` field allow-list deliberately omits (it
    // predates the representation work). Their conformance is proven below by
    // `representation_invariants`, §B3 honesty, and `check_frames`.
    let compact: ContextFrame = read_fixture("context-frame.compact.valid.json");
    assert_eq!(compact.representation, Representation::Compact);
    compact
        .representation_invariants()
        .expect("compact vector must satisfy its representation invariants (§6.4 P1–P3)");
    assert!(
        compact.declares_honest_token_cost(),
        "compact token_cost must equal the canonical count of its inline content (§B3)"
    );
    // The inline hash must be the real SHA-256 of the inline bytes it carries —
    // a golden that lied here would teach downstreams to lie too.
    let inline = compact
        .content
        .as_deref()
        .expect("a compact frame carries inline content");
    assert_eq!(
        compact.content_digest.as_deref().unwrap(),
        sha256_digest(inline.as_bytes()),
        "compact content_digest must be SHA-256 of its inline content bytes"
    );
    assert_lowercase_sha256(compact.canonical_content_hash.as_deref().unwrap());

    let reference: ContextFrame = read_fixture("context-frame.reference.valid.json");
    assert_eq!(reference.representation, Representation::Reference);
    reference
        .representation_invariants()
        .expect("reference vector must satisfy its representation invariants (§6.4 P1–P5)");
    assert!(
        reference.content.is_none(),
        "a reference frame must not inline content (§P4)"
    );
    assert_eq!(
        reference.token_cost, 0,
        "a reference inlines nothing, so its inline cost is 0 (§P4)"
    );
    assert!(
        reference.declares_honest_token_cost(),
        "reference inline cost 0 is the canonical count of no content (§B3, §P4)"
    );
    assert_lowercase_sha256(reference.canonical_content_hash.as_deref().unwrap());

    // Both must also clear the full frame conformance the suite enforces:
    // score in [0,1], a title, a citation label, an honest representation,
    // and §F4 timestamps.
    let result = ContextQueryResult {
        frames: vec![compact, reference],
        truncated: false,
        dropped_estimate: None,
    };
    let (passed, evidence) = check_frames(&result);
    assert!(
        passed,
        "representation golden vectors were rejected by frame conformance: {evidence}"
    );
}

#[test]
fn query_digest_case_pins_default_materialization_and_jcs() {
    let fixtures: DigestFixtures = read_fixture("context-query.valid.json");
    assert_eq!(fixtures.cases.len(), 1);
    let case = &fixtures.cases[0];
    assert_eq!(case.name, "minimal_query");

    let query = strict_query(&case.source).expect("minimal query source must deserialize");
    assert!(query.kinds.is_empty());
    assert!(query.anchors.is_empty());
    assert_eq!(serde_json::to_value(&query).unwrap(), case.source);

    let normalized = normalize_query(&case.source).unwrap();
    assert_digest_case(case, &normalized);
    assert!(case.source.get("kinds").is_none());
    assert!(case.source.get("anchors").is_none());
    assert_eq!(case.expected_normalized["kinds"], Value::Array(vec![]));
    assert_eq!(case.expected_normalized["anchors"], Value::Array(vec![]));
    for optional in ["query_text", "embedding", "as_of"] {
        assert!(
            case.expected_normalized.get(optional).is_none(),
            "optional scalar {optional} must remain absent"
        );
    }
}

#[test]
fn pinned_profile_rejects_all_published_unknown_field_cases() {
    let fixtures: StrictValidationFixtures = read_fixture("strict-validation.invalid.json");
    let names: BTreeSet<_> = fixtures
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "frame_embedding_unknown",
            "frame_provenance_unknown",
            "frame_relation_unknown",
            "frame_top_level_unknown",
            "query_top_level_unknown",
        ])
    );

    for case in fixtures.cases {
        let error = match case.target {
            StrictTarget::Frame => {
                serde_json::from_value::<ContextFrame>(case.input.clone()).unwrap_or_else(
                    |error| {
                        panic!(
                            "{} must remain valid for the general protocol: {error}",
                            case.name
                        )
                    },
                );
                strict_frame(&case.input).expect_err("pinned frame profile accepted unknown field")
            }
            StrictTarget::Query => {
                serde_json::from_value::<ContextQuery>(case.input.clone()).unwrap_or_else(
                    |error| {
                        panic!(
                            "{} must remain valid for the general protocol: {error}",
                            case.name
                        )
                    },
                );
                strict_query(&case.input).expect_err("pinned query profile accepted unknown field")
            }
        };
        assert_eq!(error, format!("unknown field {}", case.unknown_path));
    }
}

#[test]
fn missing_citation_fixture_is_rejected_by_frame_conformance() {
    let fixtures: CitationFixtures = read_fixture("context-frame.missing-citation.invalid.json");
    let frame = strict_frame(&fixtures.missing).expect("missing citation fixture must match shape");
    assert!(frame.citation_label.is_none());
    assert_rejected_citation(frame, "missing");
}

#[test]
fn blank_citation_fixture_is_rejected_by_frame_conformance() {
    let fixtures: CitationFixtures = read_fixture("context-frame.missing-citation.invalid.json");
    let frame = strict_frame(&fixtures.blank).expect("blank citation fixture must match shape");
    assert!(frame.citation_label.as_deref().unwrap().trim().is_empty());
    assert_rejected_citation(frame, "blank");
}

#[test]
fn normalization_vectors_match_rfc_8785_text_and_sha256() {
    let fixtures: NormalizationFixtures = read_fixture("normalization-vectors.json");
    let names: BTreeSet<_> = fixtures
        .vectors
        .iter()
        .map(|vector| vector.name.as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["escaping", "number_boundaries", "numbers", "unicode"])
    );

    for vector in fixtures.vectors {
        let input: Value = serde_json::from_str(&vector.input_json)
            .unwrap_or_else(|error| panic!("{} input is invalid JSON: {error}", vector.name));
        assert_eq!(
            input, vector.expected_normalized,
            "{} normalized JSON value",
            vector.name
        );

        // JCS is computed by an RFC 8785 implementation. Ordinary serde JSON
        // serialization is used above only for typed wire-shape assertions.
        let canonical = serde_json_canonicalizer::to_string(&input)
            .unwrap_or_else(|error| panic!("{} could not be canonicalized: {error}", vector.name));
        assert_eq!(
            canonical, vector.expected_jcs_utf8,
            "{} JCS text",
            vector.name
        );

        assert_lowercase_sha256(&vector.sha256);
        assert_eq!(
            sha256_digest(vector.expected_jcs_utf8.as_bytes()),
            vector.sha256,
            "{} JCS digest",
            vector.name
        );
    }
}
