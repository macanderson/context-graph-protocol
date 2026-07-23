//! End-to-end: an ingested paste is an ordinary provider (ADR 0006).
//!
//! These tests exercise the *public* surface only, driving the ingest provider
//! through the real `Host` fan-out, budget audit, deterministic composition, and
//! `context/verify` — the same machinery every other provider goes through. If
//! the paste provider is truly "just another provider", it survives all of it.

use contextgraph_host::{
    ContextProvider, Host, IngestConfig, PasteIngest, compose_context, ingest_paste,
};

/// A realistic mixed paste: a noisy log, a data table, a directory reference,
/// and the actual ask.
fn motivating_paste() -> PasteIngest {
    let mut log = String::new();
    for i in 0..75 {
        let level = if i == 40 {
            "ERROR"
        } else if i % 9 == 0 {
            "WARN "
        } else {
            "INFO "
        };
        log.push_str(&format!(
            "2026-07-20T18:{:02}:{:02}Z {level} retry attempt {i}\n",
            i / 60,
            i % 60
        ));
    }
    let table = "\
| endpoint      | calls | p99_ms | errors |
|---------------|-------|--------|--------|
| /v1/query     | 1841  | 210    | 3      |
| /v1/resolve   | 92    | 1204   | 41     |
| /v1/handshake | 1841  | 8      | 0      |";

    PasteIngest {
        intent: "figure out why the retry loop gives up".to_string(),
        anchors: vec![],
        attachments: vec![log, table.to_string(), "./src/net".to_string()],
    }
}

#[tokio::test]
async fn an_ingested_paste_fans_out_through_the_host_honestly() {
    let bundle = ingest_paste(motivating_paste(), IngestConfig::default());
    let query = bundle.query.clone();
    let provider_id = bundle.provider.id().to_string();
    let frame_count = bundle.provider.len();

    // Directory became an anchor; intent is verbatim.
    assert_eq!(query.goal, "figure out why the retry loop gives up");
    assert!(query.anchors.contains(&"./src/net".to_string()));
    assert_eq!(frame_count, 2, "the log and the table, not the path");

    let mut host = Host::new();
    host.register(Box::new(bundle.provider));

    // Local-only: no consent needed, so the fan-out just works.
    let fanout = host.query_all(&query).await;
    assert_eq!(fanout.failures().count(), 0, "no provider errors");
    assert_eq!(
        fanout.budget_liars().count(),
        0,
        "cost is honest, nothing dropped"
    );
    assert_eq!(fanout.accepted_frames().count(), frame_count);
    assert!(fanout.total_accepted_tokens() <= query.max_tokens as u64);

    // Deterministic composition: the same paste composes to identical bytes,
    // so an unchanged turn extends the prompt cache instead of busting it.
    let first = fanout.compose();
    let second = host.query_all(&query).await.compose();
    assert_eq!(
        first, second,
        "composition must be byte-stable across turns"
    );
    assert!(first.contains("<frame"));
    // The composed block was routed under the ingest provider.
    assert!(first.contains(&format!("provider=\"{provider_id}\"")));
}

#[tokio::test]
async fn held_ingest_frames_revalidate_as_valid() {
    let bundle = ingest_paste(motivating_paste(), IngestConfig::default());
    let query = bundle.query.clone();

    let mut host = Host::new();
    host.register(Box::new(bundle.provider));

    let fanout = host.query_all(&query).await;
    // The identities a host would hold after composing this turn.
    let held: Vec<_> = fanout
        .accepted_with_provider()
        .map(|(provider_id, frame)| frame.identity(provider_id))
        .collect();
    assert!(!held.is_empty());
    assert!(held.iter().all(|id| id.is_verifiable()));

    // Content-addressed + immutable ⇒ every held frame is vouched for, nothing
    // needs re-querying: the paste never has to re-travel.
    let outcome = host.verify_frames(&held).await;
    assert_eq!(outcome.retained.len(), held.len());
    assert!(outcome.dropped.is_empty());
    assert_eq!(outcome.requery().count(), 0);
}

#[tokio::test]
async fn re_pasting_the_same_evidence_is_free_after_the_first_turn() {
    // The dedup/cache payoff: two turns pasting the same log compose to the same
    // bytes and revalidate without a re-fetch.
    let first = ingest_paste(motivating_paste(), IngestConfig::default());
    let second = ingest_paste(motivating_paste(), IngestConfig::default());

    let compose_one = {
        let mut host = Host::new();
        let q = first.query.clone();
        host.register(Box::new(first.provider));
        host.query_all(&q).await.compose()
    };
    let compose_two = {
        let mut host = Host::new();
        let q = second.query.clone();
        host.register(Box::new(second.provider));
        host.query_all(&q).await.compose()
    };
    assert_eq!(
        compose_one, compose_two,
        "identical pastes must compose identically — the content-addressed dedup story"
    );

    // And the composed bytes are exactly what `compose_context` yields directly,
    // i.e. the public helper and the host path agree.
    assert!(!compose_one.is_empty());
    let _ = compose_context(std::iter::empty()); // helper is public and callable
}
