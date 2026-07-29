//! End-to-end conformance-suite tests against the real `contextgraph-example-docs`
//! fixture (`SPEC.md` §11). A well-behaved provider passes
//! every check; each `--misbehave` mode trips exactly the check it violates,
//! proving the suite catches a broken provider (task deliverable).

use contextgraph_conformance::{
    CHECK_ANCHOR_RELEVANCE, CHECK_AS_OF, CHECK_BUDGET_HONESTY, CHECK_CONSENT_SCOPE,
    CHECK_CORRELATION, CHECK_EMBEDDING_FINGERPRINT, CHECK_FRAME_VALIDITY, CHECK_HANDSHAKE,
    CHECK_KINDS_FILTER, CHECK_MALFORMED, CHECK_SHUTDOWN, CHECK_VERIFY_HONESTY, CheckStatus,
    ProviderTarget, run_conformance,
};

/// Path to the fixture binary, built automatically for integration tests.
fn fixture() -> String {
    env!("CARGO_BIN_EXE_contextgraph-example-docs").to_string()
}

fn target(args: &[&str]) -> ProviderTarget {
    ProviderTarget::Stdio {
        program: fixture(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

fn status_of(report: &contextgraph_conformance::ConformanceReport, name: &str) -> CheckStatus {
    report
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("report is missing the `{name}` check"))
        .status
}

#[tokio::test]
async fn a_well_behaved_provider_is_fully_conformant() {
    let report = run_conformance(target(&[])).await;
    assert!(
        report.passed(),
        "expected conformant; failures: {:?}",
        report.failures().collect::<Vec<_>>()
    );
    // Every check ran and passed (none skipped for a stdio provider).
    assert_eq!(report.checks.len(), 12);
    for name in [
        CHECK_HANDSHAKE,
        CHECK_CONSENT_SCOPE,
        CHECK_FRAME_VALIDITY,
        CHECK_VERIFY_HONESTY,
        CHECK_BUDGET_HONESTY,
        CHECK_AS_OF,
        CHECK_KINDS_FILTER,
        CHECK_ANCHOR_RELEVANCE,
        CHECK_SHUTDOWN,
        CHECK_MALFORMED,
        CHECK_EMBEDDING_FINGERPRINT,
        CHECK_CORRELATION,
    ] {
        assert_eq!(status_of(&report, name), CheckStatus::Pass, "{name}");
    }
}

#[tokio::test]
async fn dropping_the_correlation_id_fails_the_correlation_check() {
    // §H4 had no check of its own: the `drop-correlation-id` mode only ever
    // went red because losing the id desynchronizes everything downstream, so
    // an SDK could declare `correlation: true`, never echo, and pass.
    let report = run_conformance(target(&["--misbehave", "drop-correlation-id"])).await;
    assert_eq!(status_of(&report, CHECK_CORRELATION), CheckStatus::Fail);
}

#[tokio::test]
async fn ignoring_a_narrowed_kinds_filter_fails_the_kinds_check() {
    // §Q1. The unfiltered `sample_query` could never catch this: it sends
    // `kinds: []`, so a provider that ignored the filter entirely passed.
    let report = run_conformance(target(&["--misbehave", "ignore-kinds"])).await;
    assert_eq!(status_of(&report, CHECK_KINDS_FILTER), CheckStatus::Fail);
}

#[tokio::test]
async fn an_off_machine_scope_declared_with_egress_false_fails_consent_scope() {
    let report = run_conformance(target(&["--misbehave", "scope-lie"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_CONSENT_SCOPE), CheckStatus::Fail);
    // The handshake itself was fine — only the scope check caught the lie.
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Pass);
}

#[tokio::test]
async fn lying_about_token_cost_fails_budget_honesty() {
    let report = run_conformance(target(&["--misbehave", "lying-costs"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_BUDGET_HONESTY), CheckStatus::Fail);
    // The handshake itself was fine — only the budget check caught the lie.
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Pass);
}

#[tokio::test]
async fn an_out_of_range_score_fails_frame_validity() {
    let report = run_conformance(target(&["--misbehave", "bad-score"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_FRAME_VALIDITY), CheckStatus::Fail);
}

#[tokio::test]
async fn an_empty_citation_label_fails_frame_validity() {
    let report = run_conformance(target(&["--misbehave", "empty-citation"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_FRAME_VALIDITY), CheckStatus::Fail);
}

#[tokio::test]
async fn crashing_on_a_query_fails_the_frame_checks_but_not_the_handshake() {
    let report = run_conformance(target(&["--misbehave", "crash-on-query"])).await;
    assert!(!report.passed());
    // Handshake completed; the provider only died on the query.
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Pass);
    assert_eq!(status_of(&report, CHECK_FRAME_VALIDITY), CheckStatus::Fail);
    assert_eq!(status_of(&report, CHECK_BUDGET_HONESTY), CheckStatus::Fail);
}

#[tokio::test]
async fn crashing_on_garbage_fails_malformed_input_tolerance() {
    let report = run_conformance(target(&["--misbehave", "crash-on-garbage"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_MALFORMED), CheckStatus::Fail);
}

#[tokio::test]
async fn mislabeling_malformed_input_fails_malformed_input_tolerance() {
    // #9: staying alive is the §R1 MUST, but a structured `bad_request` is the
    // SHOULD the check now inspects. A provider that answers a malformed line
    // with `internal` (or any non-`bad_request` code, or none) is flagged —
    // before, passing on "some error" left the code unread.
    let report = run_conformance(target(&["--misbehave", "mislabel-malformed"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_MALFORMED), CheckStatus::Fail);
    // The provider did not crash and the handshake was fine — only the SHOULD,
    // the specific `bad_request` code, is what failed.
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Pass);
}

#[tokio::test]
async fn an_incompatible_protocol_version_fails_the_handshake() {
    let report = run_conformance(target(&["--misbehave", "bad-version"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Fail);
    // With no established provider, the behavioral checks are skipped.
    assert_eq!(
        status_of(&report, CHECK_FRAME_VALIDITY),
        CheckStatus::Skipped
    );
}

#[tokio::test]
async fn rubber_stamping_every_verify_as_valid_fails_verify_honesty() {
    // The dangerous lie: a provider that answers `valid` without comparing
    // digests lets a host go on citing evidence that changed underneath it.
    let report = run_conformance(target(&["--misbehave", "rubber-stamp-verify"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_VERIFY_HONESTY), CheckStatus::Fail);
    // Nothing else is disturbed — the frames it serves are still well-formed
    // and honestly costed, so only the verify check catches this.
    for name in [CHECK_HANDSHAKE, CHECK_FRAME_VALIDITY, CHECK_BUDGET_HONESTY] {
        assert_eq!(status_of(&report, name), CheckStatus::Pass, "{name}");
    }
}

#[tokio::test]
async fn advertising_verify_while_vouching_for_nothing_fails_verify_honesty() {
    // The other direction: a provider that claims the capability but answers
    // `unknown` to everything cannot revalidate its own just-served frames.
    let report = run_conformance(target(&["--misbehave", "hollow-verify"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_VERIFY_HONESTY), CheckStatus::Fail);
    assert_eq!(status_of(&report, CHECK_HANDSHAKE), CheckStatus::Pass);
}

#[tokio::test]
async fn a_frame_that_lies_about_its_representation_fails_frame_validity() {
    // A `reference` frame carrying inline content violates its declared shape
    // (§P1–P3). The predicate `representation_invariants` shipped in PR #42
    // with no caller; this is the witness proving the caller now exists.
    let report = run_conformance(target(&["--misbehave", "lying-representation"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_FRAME_VALIDITY), CheckStatus::Fail);
    // The frame is otherwise well-formed and honestly costed, so only the
    // representation invariant catches it.
    for name in [CHECK_HANDSHAKE, CHECK_BUDGET_HONESTY, CHECK_AS_OF] {
        assert_eq!(status_of(&report, name), CheckStatus::Pass, "{name}");
    }
}

#[tokio::test]
async fn returning_not_yet_valid_content_fails_as_of_temporal() {
    // A frame whose `valid_from` is after the `as_of` pin is content that was
    // not yet true at the pinned instant (§6.1). The honest fixture omits it;
    // `ignore-as-of` returns it anyway.
    let report = run_conformance(target(&["--misbehave", "ignore-as-of"])).await;
    assert!(!report.passed());
    assert_eq!(status_of(&report, CHECK_AS_OF), CheckStatus::Fail);
    // The unpinned query is untouched, so frame-validity and budget still pass.
    for name in [CHECK_HANDSHAKE, CHECK_FRAME_VALIDITY, CHECK_BUDGET_HONESTY] {
        assert_eq!(status_of(&report, name), CheckStatus::Pass, "{name}");
    }
}

#[tokio::test]
async fn scoring_a_dimension_mismatched_embedding_fails_embedding_fingerprint() {
    // The provider declares an `embeddings_fingerprint`, so a query embedding
    // whose length contradicts its dimension must be rejected `bad_request`
    // (§E1). `accept-bad-embedding` scores it anyway.
    let report = run_conformance(target(&["--misbehave", "accept-bad-embedding"])).await;
    assert!(!report.passed());
    assert_eq!(
        status_of(&report, CHECK_EMBEDDING_FINGERPRINT),
        CheckStatus::Fail
    );
    // Nothing else is disturbed — the frames it serves for a normal query are
    // still well-formed and honestly costed.
    for name in [CHECK_HANDSHAKE, CHECK_FRAME_VALIDITY, CHECK_BUDGET_HONESTY] {
        assert_eq!(status_of(&report, name), CheckStatus::Pass, "{name}");
    }
}

#[tokio::test]
async fn ignoring_anchors_fails_the_anchor_relevance_check() {
    // §G3/§G4. The graph is what the protocol is named for and was its least
    // exercised surface: the fixture declared `graph: false` with no relations
    // at all, so every graph requirement passed vacuously.
    let report = run_conformance(target(&["--misbehave", "ignore-anchors"])).await;
    assert_eq!(
        status_of(&report, CHECK_ANCHOR_RELEVANCE),
        CheckStatus::Fail
    );
}
