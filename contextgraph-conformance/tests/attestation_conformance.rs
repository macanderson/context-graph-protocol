//! Wire-level witnesses for the provenance-attestation check (`SPEC.md` §6.5,
//! F6–F9).
//!
//! `conformance_suite.rs` asserts that each `--misbehave` mode trips
//! `attestation` and names the right verdict. That is the outcome. These tests
//! assert the **preconditions** the outcomes depend on, by reading the fixture's
//! own wire rather than the suite's summary of it — because a mode that fails
//! for a reason other than the one it claims is a test that will sit quietly
//! through a real regression.
//!
//! The one that matters is `lift-signature`. Its whole claim is that the §6.5.2
//! identity binding, and nothing else, distinguishes the two frames. If the
//! fixture built them carelessly — a different backing file, a different
//! `content_digest`, a different provenance range — the mode would still go red
//! and would still say `CommitmentMismatch`, while proving nothing about the
//! binding at all.

use contextgraph_host::wire::FrameAttestation;
use contextgraph_host::{Envelope, RawStdioConnection};
use contextgraph_types::{
    AttestationVerdict, ContextFrame, frame_commitment, provenance_chain_head,
    verify_frame_attestation,
};

use contextgraph_conformance::sample_query;

fn fixture() -> String {
    env!("CARGO_BIN_EXE_contextgraph-example-docs").to_string()
}

/// Drive the fixture over the raw wire and return everything the attestation
/// probe reads: the provider's declared name (the `provider_id` §6.5.2 binds
/// into a commitment), its published keys, the frames, and the detached
/// attestations.
async fn wire_exchange(
    misbehave: Option<&str>,
) -> (
    String,
    Vec<contextgraph_host::AttesterKey>,
    Vec<ContextFrame>,
    Vec<FrameAttestation>,
) {
    let args: Vec<String> = match misbehave {
        Some(mode) => vec!["--misbehave".into(), mode.into()],
        None => vec![],
    };
    let mut conn = RawStdioConnection::spawn(&fixture(), &args)
        .await
        .expect("fixture spawns");
    let (info, _) = conn.handshake().await.expect("fixture handshakes");
    let keys = conn.attester_keys().to_vec();
    conn.send(&Envelope::Query {
        id: None,
        query: sample_query(),
    })
    .await
    .expect("query is accepted");
    let (frames, attestations) = match conn.recv().await.expect("fixture answers") {
        Envelope::Frames {
            result,
            attestations,
            ..
        } => (result.frames, attestations),
        other => panic!(
            "expected frames, got {}",
            contextgraph_host::envelope_kind(&other)
        ),
    };
    let _ = conn.shutdown().await;
    (info.name, keys, frames, attestations)
}

fn public_key(keys: &[contextgraph_host::AttesterKey], key_id: &str) -> Vec<u8> {
    let key = keys
        .iter()
        .find(|key| key.key_id == key_id)
        .unwrap_or_else(|| panic!("handshake published no key `{key_id}`"));
    (0..key.public_key.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&key.public_key[i..i + 2], 16).expect("key is lowercase hex"))
        .collect()
}

#[tokio::test]
async fn the_honest_fixture_publishes_a_key_and_signs_every_frame_it_serves() {
    let (provider_id, keys, frames, attestations) = wire_exchange(None).await;
    assert_eq!(keys.len(), 1, "one published attester key");
    assert_eq!(
        attestations.len(),
        frames.len(),
        "every served frame carries a detached attestation"
    );
    for entry in &attestations {
        let frame = frames
            .iter()
            .find(|frame| frame.id == entry.frame_id)
            .expect("an attestation names a frame in the same answer");
        let key = public_key(&keys, &entry.attestation.key_id);
        assert_eq!(
            verify_frame_attestation(&provider_id, frame, &entry.attestation, &key),
            AttestationVerdict::Valid,
            "frame `{}` must verify against the published key",
            frame.id
        );
    }
}

#[tokio::test]
async fn an_attestation_is_detached_and_never_rides_inside_the_frame_it_signs() {
    // F6. Serialize the served frame and look for the signature in it: an
    // attestation folded into the frame would perturb the frame's
    // content-addressed identity every time the key rotated.
    let (_, _, frames, attestations) = wire_exchange(None).await;
    let signature = &attestations
        .first()
        .expect("the fixture attests its frames")
        .attestation
        .signature;
    for frame in &frames {
        let json = serde_json::to_string(frame).expect("a frame serializes");
        assert!(
            !json.contains(signature.as_str()),
            "frame `{}` carries its own attestation inline, which F6 forbids",
            frame.id
        );
    }
}

#[tokio::test]
async fn an_attestation_lift_differs_only_in_the_frame_id() {
    // The precondition the `lift-signature` mode's whole meaning rests on.
    //
    // A frame commitment is SHA256 over (provider id, frame id,
    // content_digest, provenance chain head). Three of those four are asserted
    // equal here, so the mismatch the mode produces is attributable to the
    // frame id — the identity binding — and to nothing incidental.
    let (provider_id, keys, frames, attestations) = wire_exchange(Some("lift-signature")).await;
    assert_eq!(frames.len(), 2, "the lift needs two frames to work with");
    let (honest, forged) = (&frames[0], &frames[1]);

    assert_ne!(honest.id, forged.id, "the frames must be distinct frames");
    assert_eq!(
        provenance_chain_head(&honest.provenance),
        provenance_chain_head(&forged.provenance),
        "identical provenance is the point: without it the chain head alone would separate them"
    );
    assert_eq!(
        honest.content_digest, forged.content_digest,
        "a differing content_digest would produce the same mismatch for a reason the mode does not claim"
    );
    assert_ne!(
        frame_commitment(&provider_id, honest),
        frame_commitment(&provider_id, forged),
        "the identity binding is what must separate two otherwise-identical commitments"
    );

    // Both entries carry the SAME signature — one genuine attestation, served
    // twice — and it is genuinely valid for the frame it was issued over.
    let lifted = &attestations
        .iter()
        .find(|entry| entry.frame_id == forged.id)
        .expect("the forged frame carries an attestation")
        .attestation;
    let key = public_key(&keys, &lifted.key_id);
    assert_eq!(
        verify_frame_attestation(&provider_id, honest, lifted, &key),
        AttestationVerdict::Valid,
        "the stapled attestation must be a genuine one, or the mode proves nothing"
    );
    assert!(
        matches!(
            verify_frame_attestation(&provider_id, forged, lifted, &key),
            AttestationVerdict::CommitmentMismatch { .. }
        ),
        "a genuine signature must not validate the frame it was lifted onto"
    );
}

#[tokio::test]
async fn a_forged_signature_leaves_the_commitment_intact() {
    // §6.5.4 orders the two comparisons so an operator is sent to the right
    // problem. This asserts the ordering is observable on the wire: under
    // `forge-signature` the recomputed commitment still equals the signed one,
    // so the finding is about the key and not about tampering.
    let (provider_id, keys, frames, attestations) = wire_exchange(Some("forge-signature")).await;
    for entry in &attestations {
        let frame = frames
            .iter()
            .find(|frame| frame.id == entry.frame_id)
            .expect("an attestation names a frame in the same answer");
        assert_eq!(
            contextgraph_types::digest_string(&frame_commitment(&provider_id, frame)),
            entry.attestation.signed_commitment,
            "the commitment must be honest, or the mode would report a mismatch instead"
        );
        let key = public_key(&keys, &entry.attestation.key_id);
        assert_eq!(
            verify_frame_attestation(&provider_id, frame, &entry.attestation, &key),
            AttestationVerdict::BadSignature
        );
    }
}

#[tokio::test]
async fn truncation_hides_a_derivation_link_the_signature_still_covers() {
    // The served chain must be the *shorter* one: if the fixture served the
    // full chain the mode would be a no-op that happened to go red for some
    // other reason.
    let (_, _, honest, _) = wire_exchange(None).await;
    let (_, _, truncated, _) = wire_exchange(Some("truncate-chain")).await;
    assert_eq!(
        truncated[0].provenance.len(),
        honest[0].provenance.len(),
        "the served frame is byte-identical to the honest one — the hidden link was never served"
    );
    assert!(
        truncated[0]
            .provenance
            .iter()
            .all(|link| link.kind != "derivation"),
        "the served chain must not admit the summarisation the signature covers"
    );
}
