//! Attestation verification, end to end: a signing provider, a host that holds
//! its key, and the composition audit that says what was found
//! (`SPEC.md` §6.5, F8–F9; issues #88 and #91;
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
    AttestationState, ContextProvider, FrameAttestation, Host, HostError, ProviderResult,
    RoundRobinByRank, TrustedKey,
};
use contextgraph_types::attest::{ProvenanceAttestation, public_key_for, sign_frame_attestation};
use contextgraph_types::capability::QueryCapability;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, FrameKind, Provenance,
    ProviderInfo,
};

/// A deterministic signing seed. Signs nothing outside this file.
const SEED: [u8; 32] = [11u8; 32];
/// A second seed, for the "signed by somebody else" case.
const IMPOSTOR_SEED: [u8; 32] = [12u8; 32];

const PROVIDER: &str = "docs";
const KEY_ID: &str = "docs-2026-08";

/// An in-process provider that serves frames and offers whatever attestations
/// the test hands it — the seam a signing provider implements
/// ([`ContextProvider::query_attested`]).
struct SigningProvider {
    info: ProviderInfo,
    capabilities: Capabilities,
    frames: Vec<ContextFrame>,
    attestations: Vec<FrameAttestation>,
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
        }
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
    async fn query(&self, _query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        Ok(ContextQueryResult {
            frames: self.frames.clone(),
            truncated: false,
            dropped_estimate: None,
            // This fixture serves its attestations through `query_attested`
            // below, not on the result envelope (#138). Reconciling the two
            // paths is tracked separately.
            frame_attestations: Vec::new(),
            result_attestation: None,
        })
    }
    async fn query_attested(
        &self,
        query: &ContextQuery,
    ) -> Result<contextgraph_host::AttestedQueryResult, HostError> {
        Ok(contextgraph_host::AttestedQueryResult::with_attestations(
            self.query(query).await?,
            self.attestations.clone(),
        ))
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
        vec![FrameAttestation::new("frm_signed", sign(&attested, &SEED))],
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
        vec![FrameAttestation::new("frm_poisoned", garbage)],
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
        vec![FrameAttestation::new("frm_1", sign(&subject, &SEED))],
    ));
    assert!(state_after(&host).await.is_attested());

    // 2. No key known for this provider — a configuration gap, and the host
    //    must not present it as a finding about the signature.
    let mut untrusting = Host::new();
    untrusting.register(Box::new(SigningProvider::new(
        vec![subject.clone()],
        vec![FrameAttestation::new("frm_1", sign(&subject, &SEED))],
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
        vec![FrameAttestation::new(
            "frm_1",
            sign(&subject, &IMPOSTOR_SEED),
        )],
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
        vec![FrameAttestation::new("frm_1", malformed)],
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
        vec![FrameAttestation::new("frm_1", future_scheme)],
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
        vec![FrameAttestation::new("frm_1", sign(&one, &SEED))],
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
        vec![FrameAttestation::new(
            "frm_signed",
            sign(&signed_frame, &SEED),
        )],
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
        vec![FrameAttestation::new("frm_low", sign(&low, &SEED))],
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

/// The single-provider door verifies too, and reports the same states — a host
/// that reaches for `query_provider_attested` is not on a path where
/// verification quietly does not happen.
#[tokio::test]
async fn the_single_provider_door_reports_the_same_outcomes() {
    let one = frame("frm_1", "content");
    let host = host_trusting(SigningProvider::new(
        vec![one.clone()],
        vec![FrameAttestation::new("frm_1", sign(&one, &SEED))],
    ));

    let (attested, outcomes) = host
        .query_provider_attested(PROVIDER, &query())
        .await
        .expect("the provider answers");
    assert_eq!(attested.result.frames.len(), 1);
    assert_eq!(outcomes.len(), 1, "one outcome per frame, always");
    assert!(outcomes[0].state.is_attested());
    assert_eq!(outcomes[0].frame, one.identity(PROVIDER));

    // The un-attested door still works and still returns the same frames.
    let plain = host
        .query_provider(PROVIDER, &query())
        .await
        .expect("the provider answers");
    assert_eq!(plain.frames, attested.result.frames);
}
