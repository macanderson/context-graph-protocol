//! End-to-end: drive the MCP→CGP bridge through the real `contextgraph-host`
//! fan-out (issue #19, direction 1). The bridge wraps the hermetic
//! `contextgraph-mcp-fixture` MCP server, and the host queries it exactly as it
//! would any stdio provider — so this is the composition demo the issue asks
//! for, as an assertion: per-provider outcome, budget audit, and citations that
//! MCP alone does not carry.

use contextgraph_host::{Host, ProviderResult};
use contextgraph_types::{ConsentReceipt, ContextQuery, EgressScope, Grantor};

/// The two bins under test, located by Cargo's per-crate exe env vars.
const BRIDGE: &str = env!("CARGO_BIN_EXE_contextgraph-mcp-bridge");
const FIXTURE: &str = env!("CARGO_BIN_EXE_contextgraph-mcp-fixture");

fn query(goal: &str) -> ContextQuery {
    ContextQuery {
        goal: goal.into(),
        query_text: Some(goal.into()),
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
async fn a_local_bridge_serves_mcp_resources_as_budgeted_cited_frames() {
    let mut host = Host::new();
    host.add_stdio("mcp", BRIDGE, &["--".into(), FIXTURE.into()])
        .await
        .expect("bridge handshake should succeed");

    let query = query("how do we roll out and roll back a deploy");
    let fanout = host.query_all(&query).await;

    // One provider leg, and it carried frames (not a consent/timeout/budget miss).
    assert_eq!(fanout.outcomes.len(), 1);
    assert!(
        matches!(fanout.outcomes[0].result, ProviderResult::Frames(_)),
        "the local bridge should not be gated: {:?}",
        fanout.outcomes[0].result
    );

    let frames: Vec<_> = fanout.accepted_frames().collect();
    assert!(!frames.is_empty(), "the bridge served no frames");

    // Every frame carries MCP-resource provenance and a human citation label —
    // the difference from a raw MCP blob.
    for frame in &frames {
        assert!(
            frame
                .provenance
                .iter()
                .any(|p| p.kind == "mcp-resource"
                    && p.by.as_deref() == Some("contextgraph-mcp-fixture")),
            "frame {} lost its mcp-resource provenance",
            frame.id
        );
        assert!(
            frame
                .citation_label
                .as_deref()
                .is_some_and(|l| l.contains("mcp:")),
            "frame {} has no MCP citation label",
            frame.id
        );
    }

    // The budget audit: the fan-out rolls up into a self-consistent usage report
    // whose consumed total is the honest sum of the served frames.
    let report = fanout.usage_report(&query, "2026-07-29T00:00:00Z");
    assert!(report.is_consistent());
    assert!(report.within_budget());
    assert_eq!(report.budget_consumed, fanout.total_accepted_tokens());
    assert!(report.budget_consumed > 0);

    let _ = host.shutdown().await;
}

#[tokio::test]
async fn a_remote_bridge_is_consent_gated_until_a_receipt_is_recorded() {
    // The transitive transport-honesty rule: a bridge wrapping a *remote* MCP
    // server declares egress and is not queried until consent is granted — even
    // though the wrapped server here is the same local fixture, `--remote` is
    // what the operator asserts about the destination.
    let mut host = Host::new();
    host.add_stdio(
        "mcp-remote",
        BRIDGE,
        &[
            "--remote".into(),
            "--egress-scope".into(),
            "third-party-index".into(),
            "--".into(),
            FIXTURE.into(),
        ],
    )
    .await
    .expect("bridge handshake should succeed");

    // Without a receipt: the leg is skipped and, critically, no frames leak.
    let fanout = host.query_all(&query("deploy")).await;
    assert_eq!(fanout.accepted_frames().count(), 0);
    let missing = match &fanout.outcomes[0].result {
        ProviderResult::ConsentScopeRequired { missing, .. } => missing.clone(),
        other => panic!("expected ConsentScopeRequired, got {other:?}"),
    };
    assert_eq!(missing, vec![EgressScope::ThirdPartyIndex]);

    // The declared egress posture is visible to the host before any query.
    let info = host.provider("mcp-remote").unwrap().info().clone();
    assert!(info.data_flow.egress);

    // After a receipt for the declared scope: queried and its frames accepted.
    host.record_receipt(ConsentReceipt::new(
        "mcp-remote",
        &info,
        EgressScope::ThirdPartyIndex,
        Grantor::Human("ops@oxagen.sh".into()),
        "2026-07-29T00:00:00Z",
    ));
    let fanout = host.query_all(&query("deploy")).await;
    assert!(fanout.accepted_frames().count() > 0);

    let _ = host.shutdown().await;
}
