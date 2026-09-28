//! The trust-store file, end to end (#134;
//! [ADR 0029](../../docs/adr/0029-the-trust-store-file.md)): a store written to
//! a path the operator names, read back, and used to verify a frame — so an
//! operator's trusted keys survive a restart — and every way a file can be
//! wrong named rather than turned into an empty store.

use std::path::PathBuf;

use contextgraph_host::{
    AttestationState, Host, TrustFileError, TrustFileProblem, TrustStore, TrustedKey,
};
use contextgraph_types::attest::{public_key_for, sign_frame_attestation};
use contextgraph_types::{
    ContextFrame, ContextQueryResult, FrameAttestation, FrameKind, KeyValidity, Provenance,
};

/// Signs nothing outside this file.
const SEED: [u8; 32] = [21u8; 32];
const PROVIDER: &str = "docs";
const KEY_ID: &str = "docs-2026-09";

/// A path under the system temp directory that no other test, and no other run
/// of this one, will use. No `tempfile` dependency: the standard library is
/// enough for one file.
fn scratch_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "contextgraph-trust-{name}-{}-{nanos}.json",
        std::process::id()
    ))
}

fn signed_frame() -> (ContextFrame, ContextQueryResult) {
    let mut frame = ContextFrame::full("frm_1", FrameKind::Doc, "Title", "content", 0.9, 4);
    frame.content_digest = Some(format!("sha256:{}", "cd".repeat(32)));
    frame.provenance = vec![Provenance {
        kind: "file".into(),
        uri: Some("file:///repo/README.md".into()),
        range: None,
        digest: Some(format!("sha256:{}", "ab".repeat(32))),
        method: None,
        by: None,
    }];
    let attestation = sign_frame_attestation(
        PROVIDER,
        &frame,
        &SEED,
        KEY_ID,
        PROVIDER,
        "2026-09-28T00:00:00Z",
    );
    let result = ContextQueryResult {
        frame_attestations: vec![FrameAttestation::signed(
            frame.identity(PROVIDER),
            attestation,
        )],
        ..ContextQueryResult::unattested(vec![frame.clone()], false, None)
    };
    (frame, result)
}

/// The DoD witness: write a store, read it back from the file, and verify a
/// frame against a key that came from the file — through a `Host`, the way a
/// restarted host would.
#[test]
fn a_store_written_to_a_file_verifies_frames_after_it_is_read_back() {
    let mut store = TrustStore::new();
    store.trust(
        PROVIDER,
        TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED))
            .with_validity(KeyValidity::new(Some("2026-01-01T00:00:00Z".into()), None).unwrap()),
    );
    let path = scratch_path("round-trip");
    store.save(&path).expect("the store is written");

    // A fresh host — the restart — learns its keys from the file alone.
    let loaded = TrustStore::load(&path).expect("the file is read back");
    std::fs::remove_file(&path).ok();
    assert_eq!(loaded, store, "the round trip is lossless");

    let mut host = Host::new();
    host.set_trust_store(loaded);
    let (_, result) = signed_frame();
    let outcomes =
        host.trust()
            .check_result_signed_as_at(PROVIDER, PROVIDER, &result, "2026-09-28T00:00:01Z");
    assert_eq!(outcomes.len(), 1);
    assert!(
        outcomes[0].state.is_attested(),
        "the key came from the file and verified the frame: {:?}",
        outcomes[0].state
    );

    // The window came back too: before `not_before`, the same key is not in
    // service.
    let early =
        host.trust()
            .check_result_signed_as_at(PROVIDER, PROVIDER, &result, "2025-06-01T00:00:00Z");
    assert!(matches!(
        early[0].state,
        AttestationState::KeyNotInService { .. }
    ));
}

/// The fingerprint a host shows beside the consent prompt (ADR 0016 §2) is in
/// the file, beside the key, and is the same string `TrustedKey::fingerprint`
/// reports for the loaded key — so "I consent to this provider" and "I trust
/// this key" stay one decision after a restart.
#[test]
fn the_file_records_the_fingerprint_the_consent_prompt_shows() {
    let key = TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED));
    let fingerprint = key.fingerprint().expect("a well-formed key has one");
    let mut store = TrustStore::new();
    store.trust(PROVIDER, key);

    let text = store.to_trust_file_json().expect("serializable");
    assert!(text.contains(&fingerprint), "{text}");

    let loaded = TrustStore::from_trust_file_json(&text).unwrap();
    let shown: Vec<(String, String)> = loaded
        .providers()
        .flat_map(|provider| {
            loaded
                .keys_for(provider)
                .map(|key| (key.key_id.clone(), key.fingerprint().unwrap_or_default()))
        })
        .collect();
    assert_eq!(shown, vec![(KEY_ID.to_string(), fingerprint)]);
}

#[test]
fn a_file_that_does_not_exist_is_a_named_error_never_an_empty_store() {
    let path = scratch_path("missing");
    match TrustStore::load(&path) {
        Err(TrustFileError::Unreadable {
            path: named,
            source,
        }) => {
            assert_eq!(named, path);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn a_malformed_file_is_a_named_error_carrying_its_path() {
    let path = scratch_path("malformed");
    std::fs::write(
        &path,
        "{ \"format\": \"contextgraph-trust/1\", \"providers\": [",
    )
    .unwrap();
    let error = TrustStore::load(&path).expect_err("refused");
    std::fs::remove_file(&path).ok();
    match &error {
        TrustFileError::Invalid {
            path: Some(named), ..
        } => assert_eq!(named, &path),
        other => panic!("expected Invalid with the path, got {other:?}"),
    }
    assert!(
        matches!(error.problem(), Some(TrustFileProblem::NotJson(_))),
        "{error:?}"
    );
    assert!(error.to_string().contains("trust file"), "{error}");
}

#[test]
fn saving_replaces_the_previous_file_whole() {
    let path = scratch_path("replace");
    let mut first = TrustStore::new();
    first.trust(
        PROVIDER,
        TrustedKey::ed25519_bytes("old", &public_key_for(&SEED)),
    );
    first.save(&path).unwrap();

    let mut second = TrustStore::new();
    second.trust(
        PROVIDER,
        TrustedKey::ed25519_bytes("new", &public_key_for(&SEED)),
    );
    second.save(&path).unwrap();

    let loaded = TrustStore::load(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(loaded, second);
    assert!(loaded.key(PROVIDER, "old").is_none());
}
