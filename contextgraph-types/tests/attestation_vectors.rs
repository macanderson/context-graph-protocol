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
    AttestationVerdict, ProvenanceAttestation, digest_string, encode_provenance_link,
    frame_commitment, inclusion_proof, merkle_root, provenance_chain_head, public_key_for,
    root_from_proof, sign_commitment, verify_commitment,
};
use contextgraph_types::{ContextFrame, FrameKind, Provenance};

/// The empty chain — "this frame claims no provenance" as a signed assertion
/// rather than a gap.
const EMPTY_CHAIN_HEAD: &str =
    "sha256:2ab226c884ffeb5ac95d300a8d8ee726cb4488fb59a19047d21da7170e9b63d6";

/// The signing seed the published signature vector uses. It signs nothing
/// outside this file and the SDK suites that reconcile against it — a vector
/// needs a *fixed* key or the signature is not reproducible.
const VECTOR_SEED: [u8; 32] = [7u8; 32];

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

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

/// A link whose every present field is multi-byte UTF-8, ending in an
/// astral-plane character.
///
/// The length prefix counts **UTF-8 bytes**. A port that reaches for its
/// language's native string length gets a different number in three different
/// ways: JavaScript and Java count UTF-16 code units (so `𝄞` is 2, not 4),
/// Python 3 counts code points (so `é` is 1, not 2), and Go counts bytes only
/// because its strings already are bytes. Every one of those ports passes an
/// ASCII-only vector suite. This link is the one that separates them.
///
/// Written with escapes rather than literal characters so no editor, terminal
/// or copy-paste step can silently normalize the input out from under the
/// vector.
fn unicode_link() -> Provenance {
    Provenance {
        // "docs/naïve/日本語.md" — 24 UTF-8 bytes, counted by
        // the assertion below rather than by hand.
        kind: "file".into(),
        uri: Some("docs/na\u{ef}ve/\u{65e5}\u{672c}\u{8a9e}.md".into()),
        range: Some("L1-L2".into()),
        digest: None,
        // "résumé"
        method: Some("r\u{e9}sum\u{e9}".into()),
        // "𝄞-agent" — U+1D11E is outside the BMP: one code point, four UTF-8
        // bytes, and a surrogate *pair* in UTF-16.
        by: Some("\u{1d11e}-agent".into()),
    }
}

/// The leaves the Merkle vectors are taken over: `f0`, `f1`, … each with the
/// same content digest and no provenance, so a port reproduces them from the
/// frame commitment rule alone.
fn merkle_leaves(n: usize) -> Vec<[u8; 32]> {
    (0..n)
        .map(|i| {
            let mut frame =
                ContextFrame::full(format!("f{i}"), FrameKind::Doc, "T", "body", 0.5, 1);
            frame.content_digest = Some(format!("sha256:{}", "0e".repeat(32)));
            frame_commitment("repo-graph", &frame)
        })
        .collect()
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
    assert_eq!(
        digest_string(&merkle_root(&[])),
        "sha256:1d7896160c99d216069525861650366067ae242de9ee5e86730a36c80e57bcc2"
    );
    assert_eq!(
        digest_string(&merkle_root(&merkle_leaves(4))),
        "sha256:a29d4042e68ee59dd8713ec5361031add15e4814353da0bfca7a5bf59a96c869"
    );
}

// ---------------------------------------------------------------------------
// The vectors below were added by #93, when three SDK ports had to reconcile
// against this file and found it could not tell a correct port from an
// incorrect one. Each closes a specific blind spot; none changes a value above.
// ---------------------------------------------------------------------------

#[test]
fn the_length_prefix_counts_utf8_bytes_not_code_units() {
    let link = unicode_link();
    let encoded = encode_provenance_link(&link);

    // Stated rather than derived: a port that computes the same wrong length
    // twice would agree with a derived expectation.
    assert_eq!(
        hex(&encoded),
        concat!(
            // kind = "file" (4 bytes)
            "00000004",
            "66696c65",
            // uri present, 0x18 = 24 UTF-8 bytes ("docs/na" 7 + ï 2 + "ve/" 3
            // + 日本語 9 + ".md" 3) — but only 17 code points and 17 UTF-16
            // code units, which is the divergence this vector exists to catch.
            "01",
            "00000018",
            "646f63732f6e61c3af76652fe697a5e69cace8aa9e2e6d64",
            // range present, "L1-L2"
            "01",
            "00000005",
            "4c312d4c32",
            // digest absent
            "00",
            // method present, "résumé" — 8 UTF-8 bytes, 6 code points.
            "01",
            "00000008",
            "72c3a973756dc3a9",
            // by present, "𝄞-agent" — 10 UTF-8 bytes, 7 code points,
            // 8 UTF-16 code units.
            "01",
            "0000000a",
            "f09d849e2d6167656e74",
        ),
        "the §6.5.1 length prefix is a UTF-8 byte count"
    );

    // The three wrong answers, named, so a failing port can localize itself.
    let uri = link.uri.as_deref().expect("uri is present");
    assert_eq!(uri.len(), 24, "UTF-8 bytes");
    assert_eq!(uri.chars().count(), 17, "code points — Python's len()");
    assert_eq!(
        uri.encode_utf16().count(),
        17,
        "UTF-16 code units — JavaScript's .length"
    );
    let by = link.by.as_deref().expect("by is present");
    assert_eq!(by.len(), 10, "UTF-8 bytes");
    assert_eq!(by.chars().count(), 7, "code points");
    assert_eq!(
        by.encode_utf16().count(),
        8,
        "UTF-16 code units — the astral character is a surrogate pair"
    );
}

#[test]
fn the_published_unicode_chain_vector_holds() {
    assert_eq!(
        digest_string(&provenance_chain_head(&[unicode_link()])),
        "sha256:af8acb1dd4bd884f03cfcb21983b680b906f92733d905c3f22949fe90c8de065"
    );
}

#[test]
fn the_published_odd_leaf_merkle_vectors_hold() {
    // Four leaves is a power of two, where RFC 6962's split and the common
    // "duplicate the last leaf" shortcut agree — so the vector above cannot
    // tell them apart. Three and seven can.
    assert_eq!(
        digest_string(&merkle_root(&merkle_leaves(1))),
        "sha256:ca097914b0d358813bf20c3916fe1fcbc92695bf26cc94cae2df6bfc049072ae"
    );
    assert_eq!(
        digest_string(&merkle_root(&merkle_leaves(3))),
        "sha256:bb7a2c794a3f8de323372acf692a4cf95ea4015487caf761ff964d41e4eb15b2"
    );
    assert_eq!(
        digest_string(&merkle_root(&merkle_leaves(7))),
        "sha256:4b421f5b2831079accda2aea30aaa989e5dfb3cb01a01fdc3855f7d1d1ff1cb3"
    );
}

#[test]
fn the_published_inclusion_proof_vector_holds() {
    let leaves = merkle_leaves(7);
    let proof = inclusion_proof(&leaves, 3).expect("index 3 of 7 is in range");

    assert_eq!(proof.leaf_index, 3);
    assert_eq!(proof.leaf_count, 7);
    let path: Vec<(&str, bool)> = proof
        .path
        .iter()
        .map(|s| (s.sibling.as_str(), s.sibling_is_left))
        .collect();
    assert_eq!(
        path,
        vec![
            (
                "sha256:dc1ae14ddfe6bd0fdc1b030179837fe1b8154201fe1060d4511bc07099536629",
                true
            ),
            (
                "sha256:4162de7a1474da412cf29c4ff2f687122feb66d1cb554aaf743fdbe762f58d4b",
                true
            ),
            (
                "sha256:761a1b093e5a4c6f5f1a2eb2d5378377e6b62291ec11ec0a9b99822121659cc3",
                false
            ),
        ],
        "a seven-leaf tree is unbalanced; the sides are not derivable from parity"
    );
    assert_eq!(
        root_from_proof(&leaves[3], &proof).map(|r| digest_string(&r)),
        Some(digest_string(&merkle_root(&leaves)))
    );
}

#[test]
fn the_published_signature_vector_holds() {
    // A fixed key and a fixed commitment, so an SDK verifier can be checked
    // against a signature it did not produce. Ed25519 is deterministic, so
    // this signature is reproducible by anyone holding the seed.
    let public_key = public_key_for(&VECTOR_SEED);
    assert_eq!(
        hex(&public_key),
        "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
    );

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
    let commitment = frame_commitment("repo-graph", &frame);

    let attestation = sign_commitment(
        &commitment,
        &VECTOR_SEED,
        "key-1",
        "oxagen",
        "2026-08-27T00:00:00Z",
    );
    assert_eq!(
        attestation.signature,
        "2f8af919196862e7f946a0117d65522da36592344cf46ffe00ab8e6ec0a6b5cd\
         b132800b71b5cd161609378d8ebd789a0adc971cdb8856cd195c9154fe2b2d04"
    );
    assert_eq!(
        verify_commitment(&commitment, &attestation, &public_key),
        AttestationVerdict::Valid
    );

    // The negative direction, so the vector proves a verifier rather than a
    // constant: one flipped signature byte must not verify.
    let mut forged = attestation.clone();
    forged.signature = {
        let mut bytes = attestation.signature.clone().into_bytes();
        bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
        String::from_utf8(bytes).expect("hex stays ASCII")
    };
    assert_eq!(
        verify_commitment(&commitment, &forged, &public_key),
        AttestationVerdict::BadSignature
    );
}

#[test]
fn the_published_verdict_vocabulary_is_reproducible_offline() {
    // A verifier that only ever answers Valid is not a verifier. Every named
    // outcome an SDK port has to reproduce, produced here from the same fixed
    // inputs the SDK suites use.
    let public_key = public_key_for(&VECTOR_SEED);
    let commitment = merkle_leaves(1)[0];
    let good = sign_commitment(
        &commitment,
        &VECTOR_SEED,
        "key-1",
        "oxagen",
        "2026-08-27T00:00:00Z",
    );

    let with = |f: fn(&mut ProvenanceAttestation)| {
        let mut a = good.clone();
        f(&mut a);
        a
    };

    assert!(matches!(
        verify_commitment(&merkle_root(&[]), &good, &public_key),
        AttestationVerdict::CommitmentMismatch { .. }
    ));
    assert_eq!(
        verify_commitment(&commitment, &good, &[0u8; 5]),
        AttestationVerdict::MalformedKey
    );
    assert_eq!(
        verify_commitment(
            &commitment,
            &with(|a| a.algorithm = "dilithium3".into()),
            &public_key
        ),
        AttestationVerdict::UnknownAlgorithm("dilithium3".into())
    );
    assert_eq!(
        verify_commitment(
            &commitment,
            &with(|a| a.signature = "abcd".into()),
            &public_key
        ),
        AttestationVerdict::MalformedSignature
    );
    assert_eq!(
        verify_commitment(
            &commitment,
            &with(|a| a.signed_commitment = "not-a-digest".into()),
            &public_key
        ),
        AttestationVerdict::MalformedCommitment
    );
}

#[test]
fn the_shared_fixture_publishes_exactly_these_values() {
    // The TypeScript, Python and Go suites reconcile against
    // `tests/vectors/attestation-vectors.json` rather than against this file,
    // because a digest transcribed into four languages is four things that can
    // drift. This test is what makes the fixture a mirror rather than a fifth
    // opinion: correcting a value here without correcting it there fails.
    //
    // Read at runtime rather than `include_str!`ed, so `cargo package` does not
    // have to resolve a path outside the crate directory.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/vectors/attestation-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is the fixture the SDKs read: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&raw).expect("the fixture is JSON");

    let s = |pointer: &str| -> String {
        v.pointer(pointer)
            .and_then(|x| x.as_str())
            .unwrap_or_else(|| panic!("the fixture has no string at {pointer}"))
            .to_owned()
    };

    // The inputs, so a port building the wrong link cannot match by accident.
    let fixture_link = |key: &str| -> Provenance {
        let obj = v
            .pointer(&format!("/links/{key}"))
            .unwrap_or_else(|| panic!("the fixture has no link {key}"));
        let field = |name: &str| obj.get(name).and_then(|x| x.as_str()).map(str::to_owned);
        Provenance {
            kind: field("type").expect("every link states its type"),
            uri: field("uri"),
            range: field("range"),
            digest: field("digest"),
            method: field("method"),
            by: field("by"),
        }
    };
    assert_eq!(fixture_link("file"), file_link());
    assert_eq!(fixture_link("derivation"), derivation_link());
    assert_eq!(fixture_link("unicode"), unicode_link());

    assert_eq!(
        s("/link_encodings_hex/unicode"),
        hex(&encode_provenance_link(&unicode_link()))
    );
    let n = |pointer: &str| -> u64 {
        v.pointer(pointer)
            .and_then(|x| x.as_u64())
            .unwrap_or_else(|| panic!("the fixture has no number at {pointer}"))
    };
    let uri = unicode_link().uri.expect("uri is present");
    assert_eq!(n("/unicode_length_trap/uri/utf8_bytes"), uri.len() as u64);
    assert_eq!(
        n("/unicode_length_trap/uri/code_points"),
        uri.chars().count() as u64
    );
    assert_eq!(
        n("/unicode_length_trap/uri/utf16_code_units"),
        uri.encode_utf16().count() as u64
    );
    let by = unicode_link().by.expect("by is present");
    assert_eq!(n("/unicode_length_trap/by/utf8_bytes"), by.len() as u64);
    assert_eq!(
        n("/unicode_length_trap/by/code_points"),
        by.chars().count() as u64
    );
    assert_eq!(
        n("/unicode_length_trap/by/utf16_code_units"),
        by.encode_utf16().count() as u64
    );

    assert_eq!(s("/chain_heads/empty"), EMPTY_CHAIN_HEAD);
    assert_eq!(
        s("/chain_heads/unicode"),
        digest_string(&provenance_chain_head(&[unicode_link()]))
    );
    assert_eq!(
        s("/chain_heads/file_then_derivation"),
        digest_string(&provenance_chain_head(&[file_link(), derivation_link()]))
    );
    assert_eq!(
        s("/frame_commitment/commitment"),
        "sha256:cba5cb083177e61ce370ab586dbf7096508e5224ad18d213c902e014796c88ea"
    );
    for n in [0usize, 1, 3, 4, 7] {
        assert_eq!(
            s(&format!("/merkle/roots_by_leaf_count/{n}")),
            digest_string(&merkle_root(&merkle_leaves(n))),
            "the fixture's {n}-leaf root"
        );
    }
    let proof = inclusion_proof(&merkle_leaves(7), 3).expect("index 3 of 7 is in range");
    for (i, step) in proof.path.iter().enumerate() {
        assert_eq!(
            s(&format!("/merkle/inclusion_proof/path/{i}/sibling")),
            step.sibling
        );
        assert_eq!(
            v.pointer(&format!("/merkle/inclusion_proof/path/{i}/sibling_is_left"))
                .and_then(|x| x.as_bool()),
            Some(step.sibling_is_left)
        );
    }
    assert_eq!(
        s("/signature/public_key_hex"),
        hex(&public_key_for(&VECTOR_SEED))
    );
    assert_eq!(s("/signature/signing_key_seed_hex"), hex(&VECTOR_SEED));
    assert_eq!(
        s("/signature/attestation/signature"),
        "2f8af919196862e7f946a0117d65522da36592344cf46ffe00ab8e6ec0a6b5cd\
         b132800b71b5cd161609378d8ebd789a0adc971cdb8856cd195c9154fe2b2d04"
    );
}
