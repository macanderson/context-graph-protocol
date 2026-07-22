//! Witness test for ContextFrame representation support (CGEP lifecycle work,
//! phase 2: frame representations).
//!
//! Normative requirements exercised here (from the lifecycle build prompt):
//! - A `reference` representation frame carries NO inline `content`; it
//!   requires `content_ref` and `canonical_content_hash`.
//! - "Never encode a reference as `content: \"\"`."
//! - "If the existing Rust `ContextFrame` requires `content: String`, change
//!   it to a proper optional or tagged body representation so references can
//!   omit content. Preserve legacy wire deserialization."
//! - `representation` absent means `full` for legacy frames (legacy frames
//!   keep deserializing unchanged).
//!
//! This test operates at the wire (JSON) level so it does not depend on the
//! exact Rust API the implementer chooses. It FAILS on the current code
//! because `ContextFrame.content` is a required `String`, so a reference
//! frame without inline content cannot deserialize at all.

use contextgraph_types::ContextFrame;

/// A canonical reference-representation frame: no inline `content`,
/// `content_ref` + `canonical_content_hash` present, no inline
/// `content_hash`, no `transform`.
const REFERENCE_FRAME_JSON: &str = r#"{
  "id": "frm_ref_1",
  "kind": "doc",
  "title": "Deployment runbook",
  "uri": "file:///repo/docs/runbook.md",
  "representation": "reference",
  "canonical_content_hash": "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
  "content_ref": {
    "provider_id": "provider_example",
    "uri": "context://provider_example/records/doc_runbook_v1"
  },
  "score": 0.9,
  "token_cost": 0
}"#;

/// A legacy full frame exactly as pre-lifecycle providers emit it:
/// no `representation` field, inline `content` required.
const LEGACY_FULL_FRAME_JSON: &str = r#"{
  "id": "frm_legacy_1",
  "kind": "snippet",
  "title": "workspace.ts L120-160",
  "content": "export interface Workspace { }",
  "score": 0.83,
  "token_cost": 412
}"#;

#[test]
fn reference_frame_without_inline_content_deserializes() {
    // Fails on current code: `content` is a required String field, so serde
    // rejects a reference frame that (correctly) omits inline content.
    let frame: ContextFrame = serde_json::from_str(REFERENCE_FRAME_JSON)
        .expect("a reference-representation frame must deserialize without inline content");

    // Round-trip back to the wire and check the normative invariants on the
    // serialized form, so this test does not depend on the chosen Rust API.
    let value = serde_json::to_value(&frame).expect("frame must serialize");
    let obj = value
        .as_object()
        .expect("frame serializes as a JSON object");

    // Never encode a reference as `content: ""` (or any inline content).
    match obj.get("content") {
        None => {}
        Some(serde_json::Value::Null) => {}
        Some(other) => {
            panic!("reference frame must not carry inline content on the wire, got: {other}")
        }
    }

    // Reference frames must state their representation and keep the
    // resolver handle and canonical hash for honest, verifiable rehydration.
    assert_eq!(
        obj.get("representation").and_then(|v| v.as_str()),
        Some("reference"),
        "reference frame must round-trip its representation"
    );
    assert_eq!(
        obj.get("canonical_content_hash").and_then(|v| v.as_str()),
        Some("sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
        "reference frame must round-trip canonical_content_hash"
    );
    let content_ref = obj
        .get("content_ref")
        .and_then(|v| v.as_object())
        .expect("reference frame must round-trip content_ref");
    assert_eq!(
        content_ref.get("provider_id").and_then(|v| v.as_str()),
        Some("provider_example"),
        "content_ref must carry the exact provider_id that returned it"
    );
    assert_eq!(
        content_ref.get("uri").and_then(|v| v.as_str()),
        Some("context://provider_example/records/doc_runbook_v1"),
        "content_ref must round-trip its opaque resolver uri"
    );
}

#[test]
fn legacy_full_frame_still_deserializes_without_representation() {
    // Compatibility guard: legacy full frames (no `representation`, inline
    // `content`) must keep deserializing after the migration.
    let frame: ContextFrame = serde_json::from_str(LEGACY_FULL_FRAME_JSON)
        .expect("legacy full ContextFrames must continue to deserialize");

    let value = serde_json::to_value(&frame).expect("frame must serialize");
    let obj = value
        .as_object()
        .expect("frame serializes as a JSON object");
    assert_eq!(
        obj.get("content").and_then(|v| v.as_str()),
        Some("export interface Workspace { }"),
        "legacy full frame must keep its inline content"
    );
}
