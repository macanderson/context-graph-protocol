//! The examples, the JSON Schema, and the Rust types must agree.
//!
//! `schema/validate-examples.py` proves the bundled transcripts satisfy the
//! JSON Schema. That is only half the contract: the schema and the Rust types
//! are two independent descriptions of one wire, and nothing stopped them
//! drifting apart — a change to `Envelope` that never reached the schema (or
//! vice versa) would ship two divergent sources of truth, each internally
//! consistent.
//!
//! These tests close the loop by deserializing the same fixtures through
//! `contextgraph-host::wire::Envelope`, so a wire-type change that skips the
//! examples turns a PR red (issue #2).

use std::path::PathBuf;

use contextgraph_host::wire::Envelope;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("examples")
}

#[test]
fn every_reference_message_deserializes_through_the_rust_types() {
    let path = examples_dir().join("reference-messages.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    let messages: Vec<serde_json::Value> =
        serde_json::from_str(&raw).expect("reference-messages.json is a JSON array");

    assert!(
        !messages.is_empty(),
        "the reference transcript must not be empty — an empty fixture proves nothing"
    );

    for (index, message) in messages.iter().enumerate() {
        let envelope: Envelope = serde_json::from_value(message.clone()).unwrap_or_else(|e| {
            panic!(
                "reference-messages.json[{index}] does not deserialize into the Rust \
                 wire types: {e}\n  message: {message}\n\
                 The schema, the examples, and `Envelope` describe one wire — if you \
                 changed a wire type, update the examples in the same commit."
            )
        });

        // Round-trip: re-serializing must produce something the types still
        // accept, which catches an asymmetric Serialize/Deserialize impl.
        let reencoded = serde_json::to_value(&envelope).expect("envelope re-serializes");
        let back: Envelope =
            serde_json::from_value(reencoded).expect("re-serialized envelope re-parses");
        assert_eq!(
            back, envelope,
            "reference-messages.json[{index}] did not survive a serde round-trip"
        );
    }
}

#[test]
fn every_line_of_the_stdio_session_deserializes_through_the_rust_types() {
    let path = examples_dir().join("full-stdio-session.ndjson");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));

    let mut lines = 0;
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        serde_json::from_str::<Envelope>(line).unwrap_or_else(|e| {
            panic!(
                "full-stdio-session.ndjson line {} does not deserialize into the Rust \
                 wire types: {e}\n  line: {line}",
                index + 1
            )
        });
    }

    assert!(lines > 0, "the NDJSON transcript must not be empty");
}

#[test]
fn the_transcript_is_a_complete_session_not_an_arbitrary_bag_of_messages() {
    // The transcript doubles as documentation of the lifecycle, so it must
    // actually contain one: a handshake pair, an exchange, and a teardown. A
    // fixture that drifted into "three query messages" would still validate
    // against the schema while ceasing to demonstrate anything.
    let path = examples_dir().join("full-stdio-session.ndjson");
    let raw = std::fs::read_to_string(path).expect("stdio transcript readable");
    let kinds: Vec<String> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let value: serde_json::Value = serde_json::from_str(l).expect("valid JSON line");
            value["type"].as_str().expect("a type tag").to_string()
        })
        .collect();

    assert_eq!(kinds.first().map(String::as_str), Some("handshake"));
    assert!(kinds.iter().any(|k| k == "handshake_ack"));
    assert!(kinds.iter().any(|k| k == "query"));
    assert!(kinds.iter().any(|k| k == "frames"));
    assert_eq!(kinds.last().map(String::as_str), Some("shutdown"));
}

#[test]
fn a_correlated_query_in_the_transcript_is_answered_with_the_same_id() {
    // The transcript is the artifact a non-Rust implementer copies from, so it
    // must demonstrate correlation correctly rather than merely mention it.
    let path = examples_dir().join("full-stdio-session.ndjson");
    let raw = std::fs::read_to_string(path).expect("stdio transcript readable");

    let mut query_id = None;
    let mut frames_id = None;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Envelope>(line).expect("valid envelope") {
            Envelope::Query { id, .. } => query_id = id,
            Envelope::Frames { id, .. } => frames_id = id,
            _ => {}
        }
    }

    assert!(
        query_id.is_some(),
        "the example query should carry an id so the transcript documents correlation"
    );
    assert_eq!(
        query_id, frames_id,
        "the example `frames` reply must echo the `query` id (SPEC.md H4)"
    );
}
