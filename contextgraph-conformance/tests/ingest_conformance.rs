//! Wire conformance for prompt ingestion (ADR 0006).
//!
//! An ingested paste is only "just another provider" if its frames survive the
//! *same* checks every provider's do — not merely the Rust invariants the host
//! crate's unit tests assert, but the SPEC §6 frame-validity check, the §B
//! budget check, the `ContextFrame` representation invariants, and the JSON
//! Schema's own required-key contract, in every representation it serves.
//!
//! This is the check that catches a Rust-serialized frame drifting from the
//! schema. It is not hypothetical: writing it surfaced that `relations` (and
//! `provenance`) were listed as globally `required` in the schema while the
//! reference `ContextFrame` serializer omits them when empty — so an ingest
//! frame with no graph edges failed schema validation until the schema was
//! corrected to require only what the serializer always emits.

use std::path::PathBuf;

use contextgraph_conformance::{check_budget, check_frames};
use contextgraph_host::wire::{Envelope, decode_line, encode_line};
use contextgraph_host::{ContextProvider, IngestConfig, PasteIngest, ingest_paste};
use contextgraph_types::{ContextQuery, Representation};

/// The exact `required` key set the JSON Schema declares for a `ContextFrame`,
/// read from the schema in the repo so the test tracks the schema rather than a
/// hard-coded copy of it.
fn schema_required_frame_keys() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("schema")
        .join("contextgraph-envelope.schema.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let schema: serde_json::Value = serde_json::from_str(&raw).expect("schema is valid JSON");
    schema["$defs"]["ContextFrame"]["required"]
        .as_array()
        .expect("$defs.ContextFrame.required is an array")
        .iter()
        .map(|v| v.as_str().expect("a required key is a string").to_string())
        .collect()
}

/// The ADR's motivating paste: a noisy log, a data table, an attached note, and
/// a directory reference — every evidence kind plus an anchor.
fn motivating_paste() -> PasteIngest {
    let log: String = (0..80)
        .map(|i| {
            let level = if i == 40 {
                "ERROR"
            } else if i % 9 == 0 {
                "WARN "
            } else {
                "INFO "
            };
            format!(
                "2026-07-20T18:{:02}:{:02}Z {level} retry attempt {i}\n",
                i / 60,
                i % 60
            )
        })
        .collect();
    let table = "\
| endpoint | calls | p99_ms | errors |
|----------|-------|--------|--------|
| /query   | 1841  | 210    | 3      |
| /resolve | 92    | 1204   | 41     |";
    PasteIngest {
        intent: "figure out why the retry loop gives up".to_string(),
        anchors: vec![],
        attachments: vec![
            log,
            table.to_string(),
            "a note: backoff may be too aggressive".to_string(),
            "./src/net".to_string(),
        ],
    }
}

#[tokio::test]
async fn ingested_frames_pass_frame_budget_and_schema_conformance_in_every_representation() {
    let required = schema_required_frame_keys();
    // Sanity: the schema really does list core keys we expect to always emit.
    assert!(required.iter().any(|k| k == "id"));
    assert!(required.iter().any(|k| k == "token_cost"));

    let bundle = ingest_paste(motivating_paste(), IngestConfig::default());
    assert!(bundle.provider.len() >= 3, "log, table, and note frames");

    for representation in [
        Representation::Full,
        Representation::Compact,
        Representation::Reference,
    ] {
        let query = ContextQuery {
            representation_preferences: vec![representation],
            max_frames: u32::MAX,
            max_tokens: u32::MAX,
            ..bundle.query.clone()
        };
        let result = bundle.provider.query(&query).await.unwrap();
        assert!(
            !result.frames.is_empty(),
            "{representation:?} served no frames"
        );

        // SPEC §6 frame validity and §B1/§B3/§B4 budget — the real conformance
        // checks every provider is held to.
        let (frames_ok, evidence) = check_frames(&result);
        assert!(frames_ok, "{representation:?} frame-validity: {evidence}");
        let (budget_ok, evidence) = check_budget(&result, &query);
        assert!(budget_ok, "{representation:?} budget: {evidence}");

        for frame in &result.frames {
            // The per-representation structural invariants (mirrors the schema's
            // compact/reference `allOf` conditionals).
            frame
                .representation_invariants()
                .unwrap_or_else(|e| panic!("{representation:?} frame {}: {e}", frame.id));

            // Every schema-required `ContextFrame` key is present in the
            // serialized frame. This is the assertion that catches a
            // Rust-serialized frame diverging from the wire schema.
            let value = serde_json::to_value(frame).expect("frame serializes");
            let object = value.as_object().expect("a frame serializes to an object");
            for key in &required {
                assert!(
                    object.contains_key(key),
                    "{representation:?} frame {} omits schema-required key `{key}`",
                    frame.id
                );
            }
        }

        // The whole result round-trips through the real NDJSON `frames` envelope.
        let envelope = Envelope::Frames { id: None, result };
        let line = encode_line(&envelope).expect("frames envelope encodes");
        assert!(matches!(
            decode_line(&line).expect("frames envelope decodes"),
            Envelope::Frames { .. }
        ));
    }
}
