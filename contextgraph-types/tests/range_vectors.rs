//! The §6.2.1 range grammar against the published reference vectors
//! (`tests/vectors/range-vectors.json`, `SPEC.md` F17).
//!
//! This half checks the *addressing*: that [`LineRange`] selects exactly the
//! bytes each vector publishes, and refuses exactly the ranges each vector
//! calls unverifiable, for the reason it gives. The *digest* half — that a host
//! re-reading a file hashes those bytes to the published `sha256:` value — is
//! `contextgraph-host/tests/range_vectors.rs`, because hashing lives behind this
//! crate's optional `attestation` feature and the host is the verifier F5 names.
//!
//! The vectors were computed independently of this crate, from the SPEC prose,
//! so this is a reconciliation against a second opinion rather than a snapshot
//! of the code's own output.

use contextgraph_types::{LineRange, LineRangeError, is_well_formed_line_range};

fn vectors() -> serde_json::Value {
    // Read at runtime rather than `include_str!`ed, so `cargo package` does not
    // have to resolve a path outside the crate directory.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/vectors/range-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is the published range fixture: {e}", path.display()));
    serde_json::from_str(&raw).expect("the fixture is JSON")
}

fn reason(error: &LineRangeError) -> &'static str {
    match error {
        LineRangeError::Unrecognised => "unrecognised",
        LineRangeError::Inverted => "inverted",
        LineRangeError::StartPastEnd { .. } => "start_past_end",
    }
}

#[test]
fn every_published_range_addresses_exactly_the_published_bytes() {
    let v = vectors();
    let resources = v["resources"].as_object().expect("resources is an object");
    let cases = v["cases"].as_array().expect("cases is an array");
    assert!(cases.len() >= 20, "the fixture lost cases: {}", cases.len());

    for case in cases {
        let name = case["resource"]
            .as_str()
            .expect("each case names a resource");
        let resource = resources[name]
            .as_str()
            .unwrap_or_else(|| panic!("unknown resource {name}"))
            .as_bytes();
        let Some(range) = case.get("range").and_then(|r| r.as_str()) else {
            // No range: the whole resource (§6.2). Nothing for LineRange to do,
            // but the fixture must still say so.
            assert_eq!(case["bytes"].as_str().map(str::as_bytes), Some(resource));
            continue;
        };

        let outcome = LineRange::parse(range).and_then(|r| r.byte_span(resource));
        match (case.get("bytes").and_then(|b| b.as_str()), outcome) {
            (Some(expected), Ok(span)) => assert_eq!(
                &resource[span],
                expected.as_bytes(),
                "{name} {range:?}: {}",
                case["note"]
            ),
            (None, Err(error)) => {
                let expected = case["unverifiable"].as_str().expect("a reason is given");
                assert_eq!(
                    reason(&error),
                    expected,
                    "{name} {range:?}: {}",
                    case["note"]
                );
                // The static half of F17 agrees: only a start past the last
                // line needs the resource to be read to be found out.
                assert_eq!(
                    is_well_formed_line_range(range),
                    expected == "start_past_end",
                    "{range:?}"
                );
            }
            (expected, actual) => panic!(
                "{name} {range:?}: fixture says {expected:?}, LineRange says {actual:?} ({})",
                case["note"]
            ),
        }
    }
}
