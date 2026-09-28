//! Attestation verification, end to end: a signing provider, a host that holds
//! its key, and the composition audit that says what was found
//! (`SPEC.md` §6.5, F8–F9; issues #88, #91, #130, #133 and #136;
//! [ADR 0016](../../docs/adr/0016-attestation-trust-roots.md)).
//!
//! The unit tests in `contextgraph_host::trust` cover the verifier's verdicts.
//! These cover the **wiring** the verdicts were useless without: that a fan-out
//! actually checks, that the check reaches
//! [`CompositionAudit`](contextgraph_host::CompositionAudit), and — the one
//! that matters most — that no outcome of the check can make evidence
//! disappear.

use async_trait::async_trait;
use contextgraph_host::{
    AttestationState, AttesterKey, ContextProvider, FrameAttestation, Host, HostError, PinOutcome,
    ProviderResult, RoundRobinByRank, TrustTier, TrustedKey,
};
use contextgraph_types::attest::{
    ProvenanceAttestation, inclusion_proof, merkle_root, public_key_for, result_set_commitments,
    sign_commitment, sign_frame_attestation,
};
use contextgraph_types::capability::QueryCapability;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, FrameId, FrameKind,
    Provenance, ProviderInfo,
};

/// A deterministic signing seed. Signs nothing outside this file.
const SEED: [u8; 32] = [11u8; 32];
/// A second seed, for the "signed by somebody else" case.
const IMPOSTOR_SEED: [u8; 32] = [12u8; 32];

const PROVIDER: &str = "docs";
const KEY_ID: &str = "docs-2026-08";

/// An in-process provider that serves frames and offers whatever attestations
/// the test hands it — on the result, which is the one place an attestation
/// rides (`SPEC.md` §6.5.5, ADR 0014).
struct SigningProvider {
    info: ProviderInfo,
    capabilities: Capabilities,
    frames: Vec<ContextFrame>,
    attestations: Vec<FrameAttestation>,
    /// The answer-level signature over the §6.5.3 Merkle root, if any.
    result_attestation: Option<ProvenanceAttestation>,
    /// The keys this provider publishes, as `handshake_ack.attester_keys`
    /// would carry them.
    published: Vec<AttesterKey>,
}

impl SigningProvider {
    fn new(frames: Vec<ContextFrame>, attestations: Vec<FrameAttestation>) -> Self {
        Self {
            info: ProviderInfo {
                name: PROVIDER.into(),
                version: "0.0.1".into(),
                data_flow: DataFlow {
                    reads: true,
                    writes: false,
                    egress: false,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: QueryCapability {
                    kinds: vec!["doc".into()],
                },
                ..Capabilities::default()
            },
            frames,
            attestations,
            result_attestation: None,
            published: vec![],
        }
    }

    /// The same provider, publishing `seed`'s public key under [`KEY_ID`].
    fn publishing(mut self, seed: &[u8; 32]) -> Self {
        self.published = vec![AttesterKey {
            key_id: KEY_ID.into(),
            algorithm: "ed25519".into(),
            public_key: public_key_for(seed)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }];
        self
    }

    /// The same provider, signing its answer once over the result-set root.
    fn with_result_attestation(mut self, root: ProvenanceAttestation) -> Self {
        self.result_attestation = Some(root);
        self
    }
}

#[async_trait]
impl ContextProvider for SigningProvider {
    fn id(&self) -> &str {
        PROVIDER
    }
    fn info(&self) -> &ProviderInfo {
        &self.info
    }
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    fn attester_keys(&self) -> &[AttesterKey] {
        &self.published
    }
    async fn query(&self, _query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        Ok(ContextQueryResult {
            frame_attestations: self.attestations.clone(),
            result_attestation: self.result_attestation.clone(),
            ..ContextQueryResult::unattested(self.frames.clone(), false, None)
        })
    }
}

fn frame(id: &str, content: &str) -> ContextFrame {
    let mut frame = ContextFrame::full(id, FrameKind::Doc, id, content, 0.9, 4);
    // A distinct digest per frame: cross-provider dedup collapses frames that
    // claim the same content, and two test frames sharing one digest *are* the
    // same evidence as far as the composer is concerned.
    frame.content_digest = Some(digest_of(id));
    frame.provenance = vec![Provenance {
        kind: "file".into(),
        uri: Some(format!("file:///repo/{id}.md")),
        range: None,
        digest: Some(format!("sha256:{}", "ab".repeat(32))),
        method: None,
        by: None,
    }];
    frame
}

/// Bind an attestation to the frame it covers.
///
/// The canonical `FrameAttestation` names the whole
/// `(provider_id, frame_id, content_digest)` triple rather than a bare id
/// (#161), so evidence cannot be lent from one frame to another that happens to
/// share an id. Every frame here comes from [`frame`], whose digest is
/// [`digest_of`] its id.
fn entry(frame_id: &str, attestation: ProvenanceAttestation) -> FrameAttestation {
    FrameAttestation::signed(
        FrameId {
            provider_id: PROVIDER.into(),
            frame_id: frame_id.into(),
            content_digest: Some(digest_of(frame_id)),
        },
        attestation,
    )
}

/// A well-formed, frame-distinct `sha256:` digest built from the frame id, so
/// no two test frames look to the composer like the same evidence.
fn digest_of(id: &str) -> String {
    let mut hex = String::new();
    for byte in id.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(64);
    while hex.len() < 64 {
        hex.push('0');
    }
    format!("sha256:{hex}")
}

fn sign(frame: &ContextFrame, seed: &[u8; 32]) -> ProvenanceAttestation {
    sign_frame_attestation(
        PROVIDER,
        frame,
        seed,
        KEY_ID,
        "docs-provider",
        "2026-08-29T00:00:00Z",
    )
}

fn query() -> ContextQuery {
    ContextQuery {
        goal: "explain the protocol".into(),
        query_text: None,
        embedding: None,
        kinds: vec![FrameKind::Doc],
        anchors: vec![],
        max_frames: 8,
        max_tokens: 1_000,
        as_of: None,
        representation_preferences: vec![],
    }
}

/// A host with the signing key trusted and the provider registered.
fn host_trusting(provider: SigningProvider) -> Host {
    let mut host = Host::new();
    host.trust_key(
        PROVIDER,
        TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)),
    );
    host.register(Box::new(provider));
    host
}

/// The witness. Before this change nothing in `contextgraph-host` consumed
/// `contextgraph_types::attest`, so a frame arriving with a valid attestation
/// was indistinguishable from one arriving with none, and a composed prompt's
/// audit could not tell a reader which evidence was signed. This asserts the
/// distinction now exists and travels all the way to the audit.
#[tokio::test]
async fn a_composed_prompts_audit_distinguishes_attested_from_unattested_evidence() {
    let attested = frame("frm_signed", "the signed paragraph");
    let bare = frame("frm_bare", "an unsigned paragraph");
    let host = host_trusting(SigningProvider::new(
        vec![attested.clone(), bare.clone()],
        vec![entry("frm_signed", sign(&attested, &SEED))],
    ));

    let fanout = host.query_all(&query()).await;
    let composed = fanout.compose_for_prompt(1_000);

    let signed_entry = composed
        .audit
        .entries
        .iter()
        .find(|entry| entry.frame.frame_id == "frm_signed")
        .expect("the signed frame is in the audit");
    assert_eq!(
        signed_entry.attestation,
        AttestationState::Attested {
            key_id: KEY_ID.to_string(),
            attester_id: "docs-provider".to_string(),
            covers_content: true,
        },
        "a frame the host verified reads as attested in the audit"
    );

    let bare_entry = composed
        .audit
        .entries
        .iter()
        .find(|entry| entry.frame.frame_id == "frm_bare")
        .expect("the unsigned frame is in the audit");
    assert_eq!(
        bare_entry.attestation,
        AttestationState::Unattested,
        "a frame the provider signed nothing for reads as unattested, not unchecked"
    );

    assert_eq!(composed.audit.attested().count(), 1);
    assert!(fanout.any_attested());

    // Both are still quoted evidence — attestation annotates, it never selects.
    assert!(composed.prompt.contains("the signed paragraph"));
    assert!(composed.prompt.contains("an unsigned paragraph"));
}

/// **F9.** The security-critical requirement: an attestation the host cannot
/// verify degrades its frame to unattested and never disqualifies it. A host
/// that dropped such frames would hand any peer a denial-of-service primitive —
/// attach a malformed attestation to a rival's evidence and watch it vanish
/// from the prompt.
#[tokio::test]
async fn a_frame_carrying_a_garbage_attestation_is_still_served_marked_unattested() {
    let poisoned = frame("frm_poisoned", "evidence a peer wants suppressed");
    let mut garbage = sign(&poisoned, &SEED);
    garbage.signature = "\u{0}not a signature at all\u{7f}".into();
    garbage.signed_commitment = "definitely not sha256".into();

    let host = host_trusting(SigningProvider::new(
        vec![poisoned.clone()],
        vec![entry("frm_poisoned", garbage)],
    ));

    let fanout = host.query_all(&query()).await;

    // The leg is accepted whole: garbage cryptography is not a budget lie, a
    // consent failure, or a transport error.
    assert!(matches!(
        fanout.outcomes[0].result,
        ProviderResult::Frames(_)
    ));
    assert_eq!(
        fanout.accepted_frames().count(),
        1,
        "F9: the frame survives an attestation the host cannot verify"
    );

    let composed = fanout.compose_for_prompt(1_000);
    assert!(
        composed.prompt.contains("evidence a peer wants suppressed"),
        "F9: the evidence reaches the prompt"
    );

    let entry = &composed.audit.entries[0];
    assert!(
        matches!(
            entry.disposition,
            contextgraph_host::FrameDisposition::Included { .. }
        ),
        "F9: included, never excluded"
    );
    assert!(
        !entry.attestation.is_attested(),
        "and honestly marked: 'I could not check it' is never 'it is good'"
    );
    assert!(
        matches!(entry.attestation, AttestationState::Invalid { .. }),
        "the audit names what went wrong, got {:?}",
        entry.attestation
    );
    assert_eq!(composed.audit.attested().count(), 0);
    assert!(!fanout.any_attested());
}

/// The four outcomes a host may reasonably treat differently, each reached
/// through the real fan-out rather than by calling the verifier directly.
#[tokio::test]
async fn every_verification_outcome_reaches_the_audit_with_its_own_name() {
    let subject = frame("frm_1", "the paragraph in question");

    // 1. Verified.
    let host = host_trusting(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_1", sign(&subject, &SEED))],
    ));
    assert!(state_after(&host).await.is_attested());

    // 2. No key known for this provider — a configuration gap, and the host
    //    must not present it as a finding about the signature.
    let mut untrusting = Host::new();
    untrusting.register(Box::new(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_1", sign(&subject, &SEED))],
    )));
    assert_eq!(
        state_after(&untrusting).await,
        AttestationState::NoTrustedKey {
            key_id: KEY_ID.to_string()
        }
    );

    // 3. Key known, signature bad — signed by an impostor under the same id.
    let host = host_trusting(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_1", sign(&subject, &IMPOSTOR_SEED))],
    ));
    assert_eq!(
        state_after(&host).await,
        AttestationState::Invalid {
            verdict: contextgraph_types::AttestationVerdict::BadSignature
        }
    );

    // 4. Malformed — well-formed enough to route, not well-formed enough to
    //    check. Named as malformed, never as forged.
    let mut malformed = sign(&subject, &SEED);
    malformed.signature = "0123".into();
    let host = host_trusting(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_1", malformed)],
    ));
    assert_eq!(
        state_after(&host).await,
        AttestationState::Invalid {
            verdict: contextgraph_types::AttestationVerdict::MalformedSignature
        }
    );

    // 5. An algorithm this build cannot check (F8) — uncheckable, not invalid.
    let mut future_scheme = sign(&subject, &SEED);
    future_scheme.algorithm = "ml-dsa-65".into();
    let host = host_trusting(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_1", future_scheme)],
    ));
    assert_eq!(
        state_after(&host).await,
        AttestationState::UnknownAlgorithm {
            algorithm: "ml-dsa-65".to_string()
        }
    );

    // Every one of the five served its frame.
    async fn state_after(host: &Host) -> AttestationState {
        let fanout = host.query_all(&query()).await;
        assert_eq!(
            fanout.accepted_frames().count(),
            1,
            "F9: no verification outcome removes a frame"
        );
        fanout.compose_for_prompt(1_000).audit.entries[0]
            .attestation
            .clone()
    }
}

/// A host that trusts nobody is exactly the host that existed before this
/// change: it checks nothing, learns nothing, and loses nothing. The audit says
/// so rather than reporting the frames as unsigned.
#[tokio::test]
async fn a_host_with_an_empty_trust_store_serves_everything_and_claims_nothing() {
    let one = frame("frm_1", "content");
    let mut host = Host::new();
    host.register(Box::new(SigningProvider::new(
        vec![one.clone()],
        vec![entry("frm_1", sign(&one, &SEED))],
    )));

    let fanout = host.query_all(&query()).await;
    assert_eq!(fanout.accepted_frames().count(), 1);
    assert!(!fanout.any_attested());
    let composed = fanout.compose_for_prompt(1_000);
    assert!(matches!(
        composed.audit.entries[0].attestation,
        AttestationState::NoTrustedKey { .. }
    ));
}

/// Composing without a ledger reports `NotChecked`, not `Unattested`. "I did
/// not look" and "there was nothing to find" are different claims, and the
/// standalone composer may only make the first.
#[tokio::test]
async fn the_ledgerless_composer_says_not_checked_rather_than_unattested() {
    let one = frame("frm_1", "content");
    let composed = contextgraph_host::compose_for_prompt([(PROVIDER, &one)], 1_000);
    assert_eq!(
        composed.audit.entries[0].attestation,
        AttestationState::NotChecked
    );
    assert_eq!(composed.audit.attested().count(), 0);
}

/// The ranking seam (#95) and the attestation seam are orthogonal, and the
/// audit must carry both facts under any policy. A non-default strategy decides
/// *which* frames are packed and in what order; the ledger still describes each
/// one, and still decides nothing.
#[tokio::test]
async fn a_non_default_ranking_policy_still_carries_every_attestation_state() {
    let signed_frame = frame("frm_signed", "the signed paragraph");
    let bare = frame("frm_bare", "an unsigned paragraph");
    let host = host_trusting(SigningProvider::new(
        vec![signed_frame.clone(), bare.clone()],
        vec![entry("frm_signed", sign(&signed_frame, &SEED))],
    ));

    let fanout = host.query_all(&query()).await;
    let composed = fanout.compose_for_prompt_with(1_000, &RoundRobinByRank);

    assert_eq!(composed.audit.entries.len(), 2);
    assert_eq!(composed.audit.attested().count(), 1);
    let states: Vec<_> = composed
        .audit
        .entries
        .iter()
        .map(|entry| (entry.frame.frame_id.as_str(), entry.attestation.clone()))
        .collect();
    assert!(
        states
            .iter()
            .any(|(id, state)| *id == "frm_signed" && state.is_attested())
    );
    assert!(
        states
            .iter()
            .any(|(id, state)| *id == "frm_bare" && *state == AttestationState::Unattested)
    );

    // And the policy, not the ledger, is what chose the frames: the same
    // strategy over the same frames with no trust store composes to the same
    // bytes.
    let mut untrusting = Host::new();
    untrusting.register(Box::new(SigningProvider::new(
        vec![signed_frame, bare],
        vec![],
    )));
    let without = untrusting
        .query_all(&query())
        .await
        .compose_for_prompt_with(1_000, &RoundRobinByRank);
    assert_eq!(without.prompt, composed.prompt);
}

/// Verification annotates; it must not rank. Two frames, one signed and one
/// not, compose in exactly the order they would have without any trust store —
/// acting on the state is a host's policy decision, taken above this layer.
#[tokio::test]
async fn verification_changes_neither_selection_nor_order() {
    let high = frame("frm_high", "the higher scored frame");
    let mut low = frame("frm_low", "the lower scored frame");
    low.score = 0.1;

    let unattested_host = {
        let mut host = Host::new();
        host.register(Box::new(SigningProvider::new(
            vec![high.clone(), low.clone()],
            vec![],
        )));
        host
    };
    // The same two frames, but the *low* scored one is the signed one — the
    // arrangement most likely to tempt a reranker.
    let attested_host = host_trusting(SigningProvider::new(
        vec![high.clone(), low.clone()],
        vec![entry("frm_low", sign(&low, &SEED))],
    ));

    let without = unattested_host
        .query_all(&query())
        .await
        .compose_for_prompt(1_000);
    let with = attested_host
        .query_all(&query())
        .await
        .compose_for_prompt(1_000);

    assert_eq!(
        without.prompt, with.prompt,
        "an attestation must not move a frame, drop one, or change the bytes"
    );
    assert_eq!(
        without
            .audit
            .entries
            .iter()
            .map(|entry| entry.frame.clone())
            .collect::<Vec<_>>(),
        with.audit
            .entries
            .iter()
            .map(|entry| entry.frame.clone())
            .collect::<Vec<_>>(),
        "and must not reorder the audit either"
    );
}

/// **F9 under key rotation (#136).** A key whose validity window closed before
/// the answer arrived reads as `KeyNotInService` — its own name, distinct from
/// "no key" and from "forged" — and the frame is still served, still quoted,
/// and still included. The host records the instant it received the answer, and
/// replaying the check at an instant inside the window attests the same
/// evidence: the auditor-replay case (ADR 0028).
#[tokio::test]
async fn a_lapsed_key_degrades_its_frame_to_unattested_and_never_removes_it() {
    let subject = frame("frm_rotated", "evidence signed under a retired key");
    let retired = contextgraph_types::KeyValidity::new(
        Some("2019-01-01T00:00:00Z".into()),
        Some("2019-12-31T23:59:59Z".into()),
    )
    .expect("a well-formed window");

    let mut host = Host::new();
    host.trust_key(
        PROVIDER,
        TrustedKey::ed25519_bytes(KEY_ID, &public_key_for(&SEED)).with_validity(retired),
    );
    host.register(Box::new(SigningProvider::new(
        vec![subject.clone()],
        vec![entry("frm_rotated", sign(&subject, &SEED))],
    )));

    let fanout = host.query_all(&query()).await;
    assert_eq!(
        fanout.accepted_frames().count(),
        1,
        "F9: an out-of-service key never removes a frame"
    );
    let received_at = fanout.outcomes[0]
        .received_at
        .clone()
        .expect("a leg that served frames records when it received them");
    assert!(contextgraph_types::is_protocol_timestamp(&received_at));

    let composed = fanout.compose_for_prompt(1_000);
    assert!(
        composed
            .prompt
            .contains("evidence signed under a retired key")
    );
    let audit = &composed.audit.entries[0];
    assert!(matches!(
        audit.disposition,
        contextgraph_host::FrameDisposition::Included { .. }
    ));
    match &audit.attestation {
        AttestationState::KeyNotInService {
            key_id,
            received_at: evaluated_at,
            position,
            ..
        } => {
            assert_eq!(key_id, KEY_ID);
            assert_eq!(evaluated_at, &received_at);
            assert_eq!(*position, contextgraph_types::WindowPosition::Expired);
        }
        other => panic!("expected KeyNotInService, got {other:?}"),
    }
    assert!(!audit.attestation.is_attested());
    assert_eq!(composed.audit.attested().count(), 0);

    // Replay: the same answer, checked at an instant the operator's records
    // place inside the window, is attested — the key vouches for what it
    // signed while in service.
    let ProviderResult::Frames(result) = &fanout.outcomes[0].result else {
        panic!("the leg served frames");
    };
    let replayed =
        host.trust()
            .check_result_signed_as_at(PROVIDER, PROVIDER, result, "2019-06-01T00:00:00Z");
    assert!(replayed[0].state.is_attested(), "{replayed:?}");
}

/// **The pinned tier, end to end (#130, ADR 0030).** A host that opts in pins
/// the key a provider published at its handshake; the provider's signed frames
/// then read as `Pinned` — verified, and labelled as continuity rather than
/// identity, so the audit never counts them as operator-attested. After a
/// restart the provider publishes a *different* key under the same `key_id`:
/// the host is told loudly, the pin is not replaced, the new signatures do not
/// verify, and every frame is still served (F9).
#[tokio::test]
async fn a_pinned_key_is_its_own_tier_and_a_changed_key_is_reported_not_re_pinned() {
    let subject = frame("frm_1", "evidence from a provider nobody configured");

    // First contact.
    let mut host = Host::new();
    host.register(Box::new(
        SigningProvider::new(
            vec![subject.clone()],
            vec![entry("frm_1", sign(&subject, &SEED))],
        )
        .publishing(&SEED),
    ));
    let outcomes = host.pin_attester_keys(PROVIDER).expect("registered");
    assert!(
        matches!(outcomes.as_slice(), [PinOutcome::Pinned { .. }]),
        "{outcomes:?}"
    );

    let fanout = host.query_all(&query()).await;
    let composed = fanout.compose_for_prompt(1_000);
    let state = &composed.audit.entries[0].attestation;
    assert!(
        matches!(state, AttestationState::Pinned { .. }),
        "{state:?}"
    );
    assert_eq!(state.trust_tier(), Some(TrustTier::Pinned));
    assert!(state.signature_verified());
    assert!(
        !state.is_attested(),
        "a pin is never presented as a configured key"
    );
    assert_eq!(composed.audit.attested().count(), 0);
    assert!(!fanout.any_attested());
    assert!(
        composed
            .prompt
            .contains("evidence from a provider nobody configured")
    );

    // Restart: the store is restored, and the provider now publishes and signs
    // with a different key under the same key_id.
    let mut restarted = Host::new();
    restarted.set_trust_store(host.trust().clone());
    restarted.register(Box::new(
        SigningProvider::new(
            vec![subject.clone()],
            vec![entry("frm_1", sign(&subject, &IMPOSTOR_SEED))],
        )
        .publishing(&IMPOSTOR_SEED),
    ));
    let outcomes = restarted.pin_attester_keys(PROVIDER).expect("registered");
    assert!(
        matches!(outcomes.as_slice(), [PinOutcome::KeyChanged { .. }]),
        "a changed key is an alarm: {outcomes:?}"
    );
    assert!(outcomes[0].is_alarm());
    assert_eq!(
        restarted.trust().key(PROVIDER, KEY_ID),
        host.trust().key(PROVIDER, KEY_ID),
        "the pin stands; it is never silently replaced"
    );

    let fanout = restarted.query_all(&query()).await;
    assert_eq!(fanout.accepted_frames().count(), 1, "F9");
    let composed = fanout.compose_for_prompt(1_000);
    assert_eq!(
        composed.audit.entries[0].attestation,
        AttestationState::Invalid {
            verdict: contextgraph_types::AttestationVerdict::BadSignature
        }
    );
    assert!(
        composed
            .prompt
            .contains("evidence from a provider nobody configured")
    );
}

#[tokio::test]
async fn pinning_an_unknown_provider_is_an_error() {
    let mut host = Host::new();
    assert!(matches!(
        host.pin_attester_keys("nobody"),
        Err(HostError::UnknownProvider(_))
    ));
}

/// Sign `frames` the cheap way (`SPEC.md` §6.5.3): one signature over the
/// result-set Merkle root, and one inclusion proof per frame with no per-frame
/// signature at all. Entries come back in canonical `FrameId` order, which is
/// the order the leaves were hashed in.
fn root_signed(
    frames: &[ContextFrame],
    seed: &[u8; 32],
) -> (Vec<FrameAttestation>, ProvenanceAttestation) {
    let ordered = result_set_commitments(PROVIDER, frames);
    let commitments: Vec<[u8; 32]> = ordered.iter().map(|(_, commitment)| *commitment).collect();
    let entries = ordered
        .iter()
        .enumerate()
        .map(|(index, (id, _))| {
            FrameAttestation::proven(
                id.clone(),
                inclusion_proof(&commitments, index).expect("index is in range"),
            )
        })
        .collect();
    let root = sign_commitment(
        &merkle_root(&commitments),
        seed,
        KEY_ID,
        "docs-provider",
        "2026-08-29T00:00:00Z",
    );
    (entries, root)
}

fn three_frames() -> Vec<ContextFrame> {
    vec![
        frame("frm_a", "the first proven paragraph"),
        frame("frm_b", "the second proven paragraph"),
        frame("frm_c", "the third proven paragraph"),
    ]
}

/// #133's first witness: a provider that signs its whole answer once gets
/// credit for every frame it proves, through the same `AttestationState` and
/// the same audit field as a per-frame signature — no second vocabulary.
#[tokio::test]
async fn every_frame_proven_under_a_signed_root_is_attested_in_the_audit() {
    let frames = three_frames();
    let (entries, root) = root_signed(&frames, &SEED);
    let host =
        host_trusting(SigningProvider::new(frames.clone(), entries).with_result_attestation(root));

    let fanout = host.query_all(&query()).await;
    let composed = fanout.compose_for_prompt(1_000);
    assert_eq!(composed.audit.entries.len(), 3);
    for entry in &composed.audit.entries {
        assert_eq!(
            entry.attestation,
            AttestationState::Attested {
                key_id: KEY_ID.to_string(),
                attester_id: "docs-provider".to_string(),
                covers_content: true,
            },
            "{} is attested through its inclusion proof",
            entry.frame.frame_id
        );
    }
    assert_eq!(composed.audit.attested().count(), 3);
    assert!(fanout.any_attested());
}

/// #133's second witness: `leaf_count` is part of the proof so a verifier
/// cannot be shown a proof from a differently-shaped tree, and the host honors
/// it. A provider that hands one frame a proof taken from a four-leaf tree —
/// the candidate set before it truncated one away — gets that frame reported
/// as malformed evidence, not attested, while its correctly-proven siblings
/// stay attested. Every frame is served.
#[tokio::test]
async fn a_proof_from_a_differently_shaped_tree_is_not_attested() {
    let frames = three_frames();
    let (mut entries, root) = root_signed(&frames, &SEED);

    let mut candidates = frames.clone();
    candidates.push(frame("frm_d", "a candidate the provider truncated away"));
    let (wider_entries, _) = root_signed(&candidates, &SEED);
    let wider_proof = wider_entries[0].inclusion_proof.clone().expect("a proof");
    assert_eq!(wider_proof.leaf_count, 4);
    entries[0].inclusion_proof = Some(wider_proof);

    let host =
        host_trusting(SigningProvider::new(frames.clone(), entries).with_result_attestation(root));
    let fanout = host.query_all(&query()).await;
    assert_eq!(fanout.accepted_frames().count(), 3, "F9");
    let composed = fanout.compose_for_prompt(1_000);
    let state_of = |id: &str| {
        composed
            .audit
            .entries
            .iter()
            .find(|entry| entry.frame.frame_id == id)
            .expect("in the audit")
            .attestation
            .clone()
    };
    assert_eq!(
        state_of("frm_a"),
        AttestationState::Invalid {
            verdict: contextgraph_types::AttestationVerdict::MalformedCommitment
        }
    );
    assert!(state_of("frm_b").is_attested());
    assert!(state_of("frm_c").is_attested());
    assert!(composed.prompt.contains("the first proven paragraph"));
}

/// #133's F9 witness: a malformed root, a root signed by somebody else, and a
/// malformed proof each leave every frame served, included and quoted —
/// degraded to not-attested, never removed.
#[tokio::test]
async fn a_malformed_root_or_proof_leaves_every_frame_served() {
    let frames = three_frames();

    // 1. A root that is garbage from end to end.
    let (entries, mut garbage_root) = root_signed(&frames, &SEED);
    garbage_root.signed_commitment = "not a digest".into();
    garbage_root.signature = "\u{0}definitely not hex".into();
    assert_all_served_and_none_attested(
        SigningProvider::new(frames.clone(), entries).with_result_attestation(garbage_root),
    )
    .await;

    // 2. A well-formed root signed by an impostor under the trusted key_id.
    let (entries, forged_root) = root_signed(&frames, &IMPOSTOR_SEED);
    assert_all_served_and_none_attested(
        SigningProvider::new(frames.clone(), entries).with_result_attestation(forged_root),
    )
    .await;

    // 3. Honest root, every proof's first sibling replaced with garbage.
    let (mut entries, root) = root_signed(&frames, &SEED);
    for entry in &mut entries {
        if let Some(proof) = entry.inclusion_proof.as_mut() {
            proof.path[0].sibling = "sha256:zz".into();
        }
    }
    assert_all_served_and_none_attested(
        SigningProvider::new(frames.clone(), entries).with_result_attestation(root),
    )
    .await;

    async fn assert_all_served_and_none_attested(provider: SigningProvider) {
        let host = host_trusting(provider);
        let fanout = host.query_all(&query()).await;
        assert!(matches!(
            fanout.outcomes[0].result,
            ProviderResult::Frames(_)
        ));
        assert_eq!(fanout.accepted_frames().count(), 3, "F9: nothing removed");
        let composed = fanout.compose_for_prompt(1_000);
        assert_eq!(composed.audit.entries.len(), 3);
        for entry in &composed.audit.entries {
            assert!(
                matches!(
                    entry.disposition,
                    contextgraph_host::FrameDisposition::Included { .. }
                ),
                "F9: {} is included",
                entry.frame.frame_id
            );
            assert!(
                matches!(entry.attestation, AttestationState::Invalid { .. }),
                "named, not silently unattested: {:?}",
                entry.attestation
            );
        }
        assert_eq!(composed.audit.attested().count(), 0);
        for content in [
            "the first proven paragraph",
            "the second proven paragraph",
            "the third proven paragraph",
        ] {
            assert!(composed.prompt.contains(content), "F9: {content} is quoted");
        }
    }
}

/// The single-provider door verifies too, and reports the same states — a host
/// that reaches for `query_provider_attested` is not on a path where
/// verification quietly does not happen.
#[tokio::test]
async fn the_single_provider_door_reports_the_same_outcomes() {
    let one = frame("frm_1", "content");
    let host = host_trusting(SigningProvider::new(
        vec![one.clone()],
        vec![entry("frm_1", sign(&one, &SEED))],
    ));

    let (attested, outcomes) = host
        .query_provider_attested(PROVIDER, &query())
        .await
        .expect("the provider answers");
    assert_eq!(attested.frames.len(), 1);
    assert_eq!(outcomes.len(), 1, "one outcome per frame, always");
    assert!(outcomes[0].state.is_attested());
    assert_eq!(outcomes[0].frame, one.identity(PROVIDER));

    // The un-attested door still works and still returns the same frames.
    let plain = host
        .query_provider(PROVIDER, &query())
        .await
        .expect("the provider answers");
    assert_eq!(plain.frames, attested.frames);
}
