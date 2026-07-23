//! Tests for prompt ingestion (ADR 0006). The load-bearing properties: every
//! emitted frame is honest (§B3) and structurally valid for its representation;
//! the same paste content-addresses to the same frame; and the provider plugs
//! into the ordinary `ContextProvider` contract for query and verify.

use super::*;
use contextgraph_types::FrameId;

// ---- SHA-256 known-answer vectors ----
//
// A wrong digest silently kills dedup, so the hasher is pinned to the standard
// vectors — including the padding boundaries (empty, one block, the two-block
// case) where a hand-rolled SHA-256 would break. We use `sha2`, but the KATs
// guard our hex encoding and the `sha256:` framing regardless.

#[test]
fn sha256_matches_the_standard_known_answer_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // 56 bytes: forces a second padding block (the classic FIPS-180 vector).
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // A multi-block input well over 64 bytes.
    let million_a = "a".repeat(1000);
    assert_eq!(
        sha256_hex(million_a.as_bytes()).len(),
        64,
        "hex must always be 64 lowercase chars"
    );
}

#[test]
fn a_digest_is_well_formed_and_lowercase() {
    let digest = sha256_digest("hello");
    assert!(contextgraph_types::is_well_formed_digest(&digest));
    assert_eq!(digest, digest.to_lowercase());
}

// ---- Segmentation ----

fn kinds(attachment: &str) -> Vec<SegmentKind> {
    split_blocks(attachment).iter().map(classify).collect()
}

#[test]
fn a_log_block_is_classified_as_a_log() {
    let log = "\
2026-07-20 18:00:01 INFO  starting retry loop
2026-07-20 18:00:02 WARN  attempt 1 failed, backing off
2026-07-20 18:00:05 WARN  attempt 2 failed, backing off
2026-07-20 18:00:11 ERROR giving up after 3 attempts";
    assert_eq!(kinds(log), vec![SegmentKind::Log]);
}

#[test]
fn a_pipe_table_is_classified_as_a_table() {
    let table = "\
| id | name  | active |
|----|-------|--------|
| 1  | alice | true   |
| 2  | bob   | false  |";
    assert_eq!(kinds(table), vec![SegmentKind::Table]);
}

#[test]
fn a_fenced_block_is_code_regardless_of_content() {
    let code = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
    assert_eq!(kinds(code), vec![SegmentKind::Code]);
}

#[test]
fn a_bare_path_is_a_path_reference_not_a_frame() {
    for path in [
        "./src/net",
        "../lib/mod.rs",
        "/repo/src",
        "net.rs",
        "file:///a/b",
    ] {
        assert_eq!(kinds(path), vec![SegmentKind::PathRef], "{path}");
    }
    // A URL is not a workspace anchor.
    assert_eq!(kinds("https://example.com/x"), vec![SegmentKind::Prose]);
    // A sentence with spaces is prose, even if it mentions a slash.
    assert_eq!(kinds("see the a/b split here"), vec![SegmentKind::Prose]);
}

#[test]
fn ordinary_prose_stays_prose() {
    let prose = "The retry loop gives up too early. I think the backoff is wrong \
                 and we exhaust attempts before the service recovers.";
    assert_eq!(kinds(prose), vec![SegmentKind::Prose]);
}

#[test]
fn blocks_are_split_on_blank_lines_and_fences() {
    let mixed = "\
first paragraph of prose

2026-07-20 18:00:01 ERROR boom
2026-07-20 18:00:02 ERROR boom again

```
raw code here
more code
```";
    assert_eq!(
        kinds(mixed),
        vec![SegmentKind::Prose, SegmentKind::Log, SegmentKind::Code]
    );
}

// ---- Honesty invariants: the whole point ----

fn ingest(intent: &str, attachments: Vec<&str>) -> IngestBundle {
    ingest_paste(
        PasteIngest {
            intent: intent.to_string(),
            anchors: vec![],
            attachments: attachments.into_iter().map(String::from).collect(),
        },
        IngestConfig::default(),
    )
}

/// Every frame the provider can emit, in every representation it advertises.
async fn all_served_frames(bundle: &IngestBundle) -> Vec<ContextFrame> {
    let mut out = Vec::new();
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
        out.extend(bundle.provider.query(&query).await.unwrap().frames);
    }
    out
}

#[tokio::test]
async fn every_emitted_frame_declares_an_honest_token_cost() {
    // §B3: `token_cost == ceil(utf8_len(content)/4)` for the exact bytes emitted
    // — checked across full, compact, AND reference, because each inlines
    // different content.
    let big_log = build_big_log(200);
    let bundle = ingest(
        "why does the retry loop give up",
        vec![
            &big_log,
            SAMPLE_TABLE,
            SAMPLE_CODE,
            "a short note about the bug",
        ],
    );
    let frames = all_served_frames(&bundle).await;
    assert!(!frames.is_empty());
    for frame in &frames {
        assert!(
            frame.declares_honest_token_cost(),
            "frame {} ({:?}) lied about cost: declared {}, canonical {}",
            frame.id,
            frame.representation,
            frame.token_cost,
            frame.expected_inline_token_cost(),
        );
    }
}

#[tokio::test]
async fn every_emitted_frame_satisfies_its_representation_invariants() {
    let big_log = build_big_log(200);
    let bundle = ingest("fix it", vec![&big_log, SAMPLE_TABLE, SAMPLE_CODE, "note"]);
    for frame in all_served_frames(&bundle).await {
        frame
            .representation_invariants()
            .unwrap_or_else(|e| panic!("frame {} invalid: {e}", frame.id));
        assert!(frame.has_valid_score());
        assert!(frame.has_valid_temporal_fields());
        // Pasted evidence must never masquerade as re-readable file provenance.
        assert!(frame.provenance_with_unusable_digests().is_empty());
        assert!(frame.provenance.iter().all(|p| p.kind == "derivation"));
    }
}

#[tokio::test]
async fn a_compact_frame_actually_shrinks_a_large_log_and_stays_rehydratable() {
    let big_log = build_big_log(300);
    let bundle = ingest("debug", vec![&big_log]);

    let compact = one_frame(&bundle, Representation::Compact).await;
    let full = one_frame(&bundle, Representation::Full).await;

    assert_eq!(compact.representation, Representation::Compact);
    assert!(
        compact.token_cost < full.token_cost,
        "compact ({}) must cost less than full ({})",
        compact.token_cost,
        full.token_cost
    );
    // The canonical hash pins the full source; the full frame's inline hash is
    // exactly that canonical hash — so a `[full]` re-query is verifiably the
    // rehydration of the same artifact.
    assert_eq!(
        compact.canonical_content_hash.as_deref(),
        full.content_digest.as_deref()
    );
    assert!(compact.canonical_token_cost.unwrap() == full.token_cost);
    assert_eq!(compact.content_fidelity, Some(ContentFidelity::Summarized));
    // The compact rendering keeps the ERROR line the log is about.
    assert!(compact.content.as_deref().unwrap().contains("ERROR"));
}

#[tokio::test]
async fn a_small_paste_is_served_verbatim_and_exact() {
    let bundle = ingest("ctx", vec!["one line, nothing to compact"]);
    let compact = one_frame(&bundle, Representation::Compact).await;
    // Nothing to distill: the "compact" rendering is the exact source.
    assert_eq!(compact.content_fidelity, Some(ContentFidelity::Exact));
    assert_eq!(
        compact.content.as_deref(),
        Some("one line, nothing to compact")
    );
    assert!(compact.declares_honest_token_cost());
    compact.representation_invariants().unwrap();
}

// ---- Intent and anchors ----

#[test]
fn intent_passes_through_verbatim_as_the_goal() {
    let intent = "why does the retry loop give up — DON'T paraphrase this: `foo->bar`";
    let bundle = ingest(intent, vec!["2026-01-01 ERROR x"]);
    assert_eq!(bundle.query.goal, intent, "intent must never be rewritten");
}

#[test]
fn a_directory_reference_becomes_an_anchor_with_no_frame() {
    let bundle = ingest_paste(
        PasteIngest {
            intent: "fix".into(),
            anchors: vec!["file:///repo/src/lib.rs".into()],
            attachments: vec!["./src/net".into()],
        },
        IngestConfig::default(),
    );
    assert!(bundle.query.anchors.contains(&"./src/net".to_string()));
    assert!(
        bundle
            .query
            .anchors
            .contains(&"file:///repo/src/lib.rs".to_string())
    );
    assert!(bundle.provider.is_empty(), "a path yields no frame");
    assert!(matches!(
        bundle.report[0].became,
        SegmentOutcome::Anchor { .. }
    ));
}

// ---- Content addressing / dedup ----

#[test]
fn the_same_paste_content_addresses_to_the_same_id_and_dedups() {
    let log = build_big_log(50);
    let once = ingest("g", vec![&log]);
    let twice = ingest("g", vec![&log, &log]);

    // Identical content ⇒ identical frame id across independent ingests.
    let id_once = &once.provider.artifacts[0].id;
    let id_twice = &twice.provider.artifacts[0].id;
    assert_eq!(id_once, id_twice);
    // The second identical attachment is collapsed, not duplicated.
    assert_eq!(twice.provider.len(), 1);
    assert!(
        twice
            .report
            .iter()
            .any(|r| matches!(r.became, SegmentOutcome::Duplicate { .. }))
    );
}

// ---- Provider contract: query budget & frame cap ----

#[tokio::test]
async fn query_respects_max_frames_and_max_tokens() {
    let bundle = ingest("g", vec![SAMPLE_TABLE, SAMPLE_CODE, "note one", "note two"]);
    assert!(bundle.provider.len() >= 3);

    // Cap to two frames.
    let capped = ContextQuery {
        max_frames: 2,
        max_tokens: u32::MAX,
        ..bundle.query.clone()
    };
    let result = bundle.provider.query(&capped).await.unwrap();
    assert!(result.respects_frame_limit(2));
    assert_eq!(result.frames.len(), 2);
    assert!(result.truncated);

    // Cap to a tiny token budget: the sum stays under it (§B1).
    let tight = ContextQuery {
        max_frames: u32::MAX,
        max_tokens: 3,
        ..bundle.query.clone()
    };
    let result = bundle.provider.query(&tight).await.unwrap();
    assert!(result.respects_budget(3));
    assert!(result.frames_with_dishonest_cost().is_empty());
}

#[tokio::test]
async fn the_default_bundle_query_returns_every_frame_within_budget() {
    let bundle = ingest("g", vec![SAMPLE_TABLE, SAMPLE_CODE, "a note"]);
    let result = bundle.provider.query(&bundle.query).await.unwrap();
    assert_eq!(result.frames.len(), bundle.provider.len());
    assert!(result.respects_budget(bundle.query.max_tokens));
    assert!(!result.truncated);
}

// ---- Provider contract: verify ----

#[tokio::test]
async fn verify_vouches_for_a_held_frame_and_rejects_a_tampered_or_unknown_one() {
    let log = build_big_log(120);
    let bundle = ingest("g", vec![&log]);
    let provider = &bundle.provider;

    // Take the compact frame the host would actually hold.
    let compact = one_frame(&bundle, Representation::Compact).await;
    let held = compact.identity(provider.id());
    assert!(held.is_verifiable());

    let request = VerifyRequest::new(vec![held.clone()]);
    let response = provider.verify(&request).await.unwrap();
    assert_eq!(response.verdict_for(&held), Some(&Verdict::Valid));

    // A wrong digest on a known id ⇒ stale, carrying the current digest.
    let tampered = FrameId::new(provider.id(), &compact.id, Some("sha256:dead".into()));
    let response = provider
        .verify(&VerifyRequest::new(vec![tampered.clone()]))
        .await
        .unwrap();
    assert!(matches!(
        response.verdict_for(&tampered),
        Some(Verdict::Stale { .. })
    ));

    // An unknown id ⇒ gone (the store is authoritative-complete).
    let ghost = FrameId::new(provider.id(), "frm_notours", Some("sha256:beef".into()));
    let response = provider
        .verify(&VerifyRequest::new(vec![ghost.clone()]))
        .await
        .unwrap();
    assert_eq!(response.verdict_for(&ghost), Some(&Verdict::Gone));
}

// ---- End-to-end honesty of the realistic example ----

#[tokio::test]
async fn the_motivating_example_produces_a_clean_bundle() {
    // The exact scenario from the ADR: a log, a table, a directory, and intent.
    let log = build_big_log(75);
    let bundle = ingest_paste(
        PasteIngest {
            intent: "figure out why the retry loop gives up".into(),
            anchors: vec![],
            attachments: vec![log.clone(), SAMPLE_TABLE.into(), "./src/net".into()],
        },
        IngestConfig::default(),
    );

    // Intent verbatim, directory became an anchor, two evidence frames.
    assert_eq!(bundle.query.goal, "figure out why the retry loop gives up");
    assert!(bundle.query.anchors.contains(&"./src/net".to_string()));
    assert_eq!(bundle.provider.len(), 2);

    // The provider is local-only and needs no consent.
    assert!(!bundle.provider.info().data_flow.egress);
    assert!(bundle.provider.info().data_flow.scopes_consistent());
    assert!(bundle.provider.capabilities().representations_consistent());

    // Everything it serves is honest and composes.
    let result = bundle.provider.query(&bundle.query).await.unwrap();
    assert!(result.respects_budget(bundle.query.max_tokens));
    assert!(result.frames_with_dishonest_cost().is_empty());
    let composed =
        crate::compose::compose_context(result.frames.iter().map(|f| (bundle.provider.id(), f)));
    assert!(composed.contains("<frame"));
}

// ---- fixtures ----

const SAMPLE_TABLE: &str = "\
| id | name  | score | active |
|----|-------|-------|--------|
| 1  | alice | 0.91  | true   |
| 2  | bob   | 0.42  | false  |
| 3  | carol | 0.77  | true   |";

const SAMPLE_CODE: &str =
    "```rust\nfn retry() {\n    for _ in 0..3 {\n        attempt();\n    }\n}\n```";

fn build_big_log(lines: usize) -> String {
    let mut out = String::new();
    for i in 0..lines {
        let level = if i == lines / 2 {
            "ERROR"
        } else if i % 7 == 0 {
            "WARN "
        } else {
            "INFO "
        };
        out.push_str(&format!(
            "2026-07-20T18:{:02}:{:02}Z {level} attempt {i} of the retry loop\n",
            i / 60 % 60,
            i % 60
        ));
    }
    out.trim_end().to_string()
}

async fn one_frame(bundle: &IngestBundle, representation: Representation) -> ContextFrame {
    let query = ContextQuery {
        representation_preferences: vec![representation],
        max_frames: 1,
        max_tokens: u32::MAX,
        ..bundle.query.clone()
    };
    bundle
        .provider
        .query(&query)
        .await
        .unwrap()
        .frames
        .into_iter()
        .next()
        .expect("at least one frame")
}
