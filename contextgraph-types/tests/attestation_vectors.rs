//! Cross-language reference vectors for provenance attestation
//! (`SPEC.md` §6.5).
//!
//! §6.5.1 defines a **normative** byte encoding, and a normative encoding with
//! no published vectors is a rule two implementations can both believe they
//! follow while computing different hashes. These are the byte vectors an
//! implementation in any language reconciles against; if your chain head
//! matches these, your encoding matches the spec.
//!
//! Every value here is a *fixture*, not an assertion about the current code:
//! changing the encoding changes these digests, and a diff in this file is a
//! **wire-breaking change** that requires a new major family (`SPEC.md` §15).
//! That is exactly why they are written out rather than recomputed.

#![cfg(feature = "attestation")]

use contextgraph_types::attest::{
    digest_string, encode_provenance_link, frame_commitment, merkle_root, provenance_chain_head,
};
use contextgraph_types::{ContextFrame, FrameKind, Provenance};

/// The empty chain — "this frame claims no provenance" as a signed assertion
/// rather than a gap.
const EMPTY_CHAIN_HEAD: &str =
    "sha256:2ab226c884ffeb5ac95d300a8d8ee726cb4488fb59a19047d21da7170e9b63d6";

fn file_link() -> Provenance {
    Provenance {
        kind: "file".into(),
        uri: Some("src/retry.rs".into()),
        range: Some("L10-L20".into()),
        digest: Some(format!("sha256:{}", "ab".repeat(32))),
        method: None,
        by: None,
    }
}

fn derivation_link() -> Provenance {
    Provenance {
        kind: "derivation".into(),
        uri: None,
        range: None,
        digest: None,
        method: Some("extractive_summary".into()),
        by: Some("refprov/1.0".into()),
    }
}

#[test]
fn the_link_encoding_is_length_prefixed_exactly_as_specified() {
    let link = Provenance {
        kind: "file".into(),
        uri: Some("a".into()),
        range: None,
        digest: None,
        method: None,
        by: None,
    };
    // kind="file"      -> 00 00 00 04 'f' 'i' 'l' 'e'
    // uri=Some("a")    -> 01 00 00 00 01 'a'
    // range..by = None -> 00 00 00 00
    let expected: Vec<u8> = vec![
        0x00, 0x00, 0x00, 0x04, b'f', b'i', b'l', b'e', //
        0x01, 0x00, 0x00, 0x00, 0x01, b'a', //
        0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        encode_provenance_link(&link),
        expected,
        "the §6.5.1 encoding is normative — a mismatch here is a wire break"
    );
}

#[test]
fn an_empty_provenance_chain_hashes_to_the_published_genesis() {
    assert_eq!(digest_string(&provenance_chain_head(&[])), EMPTY_CHAIN_HEAD);
}

#[test]
fn the_published_chain_vectors_hold() {
    assert_eq!(
        digest_string(&provenance_chain_head(&[file_link()])),
        "sha256:ac5418d723088033179a2671d17cd08d3e082eefa783e3eeb1a5145f83592178"
    );
    assert_eq!(
        digest_string(&provenance_chain_head(&[file_link(), derivation_link()])),
        "sha256:2245cb46807d55e9e89479b2948d7887fbba95430dd315d63972744a47a18410"
    );
}

#[test]
fn the_published_frame_commitment_vector_holds() {
    let mut frame = ContextFrame::full(
        "retry-policy",
        FrameKind::Doc,
        "Retry policy",
        "Retries use exponential backoff.",
        0.87,
        8,
    );
    frame.content_digest = Some(format!("sha256:{}", "cd".repeat(32)));
    frame.provenance = vec![file_link()];

    assert_eq!(
        digest_string(&frame_commitment("repo-graph", &frame)),
        "sha256:cba5cb083177e61ce370ab586dbf7096508e5224ad18d213c902e014796c88ea"
    );
}

#[test]
fn the_published_merkle_vectors_hold() {
    let leaves: Vec<[u8; 32]> = (0..4)
        .map(|i| {
            let mut frame =
                ContextFrame::full(format!("f{i}"), FrameKind::Doc, "T", "body", 0.5, 1);
            frame.content_digest = Some(format!("sha256:{}", "0e".repeat(32)));
            frame_commitment("repo-graph", &frame)
        })
        .collect();

    assert_eq!(
        digest_string(&merkle_root(&[])),
        "sha256:1d7896160c99d216069525861650366067ae242de9ee5e86730a36c80e57bcc2"
    );
    assert_eq!(
        digest_string(&merkle_root(&leaves)),
        "sha256:a29d4042e68ee59dd8713ec5361031add15e4814353da0bfca7a5bf59a96c869"
    );
}
