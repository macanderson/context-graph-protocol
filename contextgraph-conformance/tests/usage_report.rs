//! Usage-report conformance case (`docs/context-reuse.md` §2, requirement U1).
//!
//! Drives the real `contextgraph-example-docs` fixture through a `Host`, rolls
//! the fan-out up into a `UsageReport`, and asserts the arithmetic identity a
//! metering pipeline depends on: the report's consumed total equals an
//! *independent* re-sum of the served frames' declared `token_cost`, and every
//! served frame is itemized by its stable identity for audit walk-back. This is
//! the host-side "produce a report for any query it executed" guarantee against
//! a live provider, not an in-memory fixture.

use contextgraph_host::Host;
use contextgraph_types::ContextQuery;

fn fixture() -> String {
    env!("CARGO_BIN_EXE_contextgraph-example-docs").to_string()
}

fn query() -> ContextQuery {
    ContextQuery {
        goal: "how do I configure it".into(),
        query_text: Some("configuration".into()),
        embedding: None,
        kinds: vec![],
        anchors: vec![],
        max_frames: 8,
        max_tokens: 4096,
        as_of: None,
        representation_preferences: vec![],
    }
}

#[tokio::test]
async fn a_host_produces_a_self_consistent_usage_report_for_a_real_provider() {
    let mut host = Host::new();
    host.add_stdio("docs", &fixture(), &[])
        .await
        .expect("handshake with the fixture should succeed");

    let query = query();
    let fanout = host.query_all(&query).await;
    let report = fanout.usage_report(&query, "2026-07-21T00:00:00Z");

    // The report re-sums from its own itemized served frames.
    assert!(
        report.is_consistent(),
        "usage report totals must re-sum from the served frames: {report:?}"
    );
    assert!(report.within_budget());
    assert_eq!(report.budget_requested, query.max_tokens);

    // The consumed total equals an INDEPENDENT sum of the accepted frames'
    // declared costs — the checkable arithmetic identity (U1), proven against a
    // real provider rather than assumed from the builder.
    let independent: u64 = fanout.accepted_frames().map(|f| f.token_cost as u64).sum();
    assert_eq!(report.budget_consumed, independent);
    assert!(independent > 0, "the fixture serves non-zero-cost frames");

    // Exactly one provider, itemizing every served frame by stable identity so
    // an auditor can walk from the billed total to the exact frames.
    assert_eq!(report.providers.len(), 1);
    let docs = &report.providers[0];
    assert_eq!(docs.provider_id, "docs");
    assert_eq!(docs.frames_served as usize, docs.served_frames.len());
    assert!(docs.frames_served >= 1);
    for served in &docs.served_frames {
        assert_eq!(served.frame.provider_id, "docs");
        assert!(!served.frame.frame_id.is_empty());
        // The fixture declares content digests, so served frames are
        // verifiable — the identity carries the digest for later revalidation.
        assert!(served.frame.is_verifiable());
    }

    let shutdown = host.shutdown().await;
    assert!(shutdown.iter().all(|(_, result)| result.is_ok()));
}
