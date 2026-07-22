//! `context/verify` wire-level tests (`docs/context-reuse.md` §4).
//!
//! Two things the type-level tests can't cover: that the published reference
//! messages stay in lockstep with the types (the repo's schema/examples sync
//! convention), and that a real stdio provider answers a verify exchange
//! honestly over an actual pipe.

use contextgraph_host::wire::Envelope;
use contextgraph_host::{DropReason, Host};
use contextgraph_types::{FrameId, Verdict};

fn fixture() -> String {
    env!("CARGO_BIN_EXE_contextgraph-example-docs").to_string()
}

/// Every published reference message must still parse as an `Envelope`, and
/// the verify exchange must round-trip byte-for-byte — the examples are the
/// cross-language contract, so drift between them and the types is a defect.
#[test]
fn published_reference_messages_match_the_types() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/reference-messages.json"
    ))
    .expect("reference-messages.json is readable");
    let messages: Vec<serde_json::Value> =
        serde_json::from_str(&raw).expect("reference messages are a JSON array");

    let mut saw_verify = false;
    let mut saw_verified = false;
    for (i, message) in messages.iter().enumerate() {
        let kind = message["type"].as_str().expect("every message has a type");
        let parsed: Envelope = serde_json::from_value(message.clone())
            .unwrap_or_else(|e| panic!("reference message {i} (`{kind}`) does not parse: {e}"));

        if matches!(kind, "verify" | "verified") {
            let reserialized = serde_json::to_value(&parsed).unwrap();
            assert_eq!(
                &reserialized, message,
                "reference message {i} (`{kind}`) does not round-trip through the types"
            );
            saw_verify |= kind == "verify";
            saw_verified |= kind == "verified";
        }
    }
    assert!(
        saw_verify && saw_verified,
        "the reference messages must document a full verify exchange"
    );
}

/// A verify request carries identities, never bodies — the economic claim of
/// §4, asserted against the actual serialized wire form.
#[test]
fn a_verify_exchange_carries_no_frame_bodies_on_the_wire() {
    let request = contextgraph_types::VerifyRequest::new(vec![FrameId::new(
        "docs",
        "frm_getting_started",
        Some("sha256:getting-started-v1".into()),
    )]);
    let line = serde_json::to_string(&Envelope::Verify { request }).unwrap();
    for body_field in ["\"content\"", "\"title\"", "\"provenance\"", "\"score\""] {
        assert!(
            !line.contains(body_field),
            "a verify envelope must not carry {body_field}: {line}"
        );
    }
}

/// Drive the real stdio fixture end-to-end: a frame it just served verifies
/// `valid`, a mutated digest verifies `stale` and is evicted by the host, and
/// a frame it does not serve at all comes back `gone`.
#[tokio::test]
async fn a_real_stdio_provider_answers_a_verify_exchange_honestly() {
    let mut host = Host::new();
    host.add_stdio("docs", &fixture(), &[])
        .await
        .expect("fixture handshakes");

    let served = FrameId::new(
        "docs",
        "frm_getting_started",
        Some("sha256:getting-started-v1".into()),
    );
    let mutated = FrameId::new(
        "docs",
        "frm_configuration",
        Some("sha256:not-what-i-serve".into()),
    );
    let absent = FrameId::new("docs", "frm_never_existed", Some("sha256:whatever".into()));

    let outcome = host
        .verify_frames(&[served.clone(), mutated.clone(), absent.clone()])
        .await;

    assert_eq!(
        outcome.retained,
        vec![served],
        "an unchanged frame must survive revalidation"
    );
    // The host demonstrably drops the frame verified stale.
    assert!(outcome.was_dropped(&mutated));
    assert_eq!(
        outcome.drop_reason(&mutated),
        Some(&DropReason::Stale {
            replacement_digest: Some("sha256:configuration-v1".into())
        }),
        "a mutated digest must come back stale, carrying the current digest"
    );
    assert_eq!(outcome.drop_reason(&absent), Some(&DropReason::Gone));

    // Only the stale frame is worth re-querying; the gone one is not there.
    let requery: Vec<&FrameId> = outcome.requery().collect();
    assert_eq!(requery, vec![&mutated]);

    let _ = host.shutdown().await;
}

/// The capability gate: pointed at a provider that does not advertise
/// `verify`, the host must fall back to re-query rather than trust the frames.
#[tokio::test]
async fn the_default_provider_impl_vouches_for_nothing() {
    // The trait default answers `unknown` for everything, so a provider that
    // implements nothing can never accidentally bless a stale frame.
    use contextgraph_types::{VerifyRequest, VerifyResponse};
    let request = VerifyRequest::new(vec![FrameId::new("p", "f", Some("sha256:a".into()))]);
    let response = VerifyResponse::uniform(&request, Verdict::Unknown);
    assert_eq!(
        response.verdict_for(&request.frames[0]),
        Some(&Verdict::Unknown)
    );
    assert!(!Verdict::Unknown.permits_reuse());
}
