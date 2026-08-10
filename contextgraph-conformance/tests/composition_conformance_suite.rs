//! Composition conformance (`SPEC.md` §11.1) — the suite a *downstream* host runs
//! against its own composition layer.
//!
//! `host_conformance_suite.rs` certifies the reference host; this certifies the
//! **contract** a non-reference host is held to. The distinction matters for what
//! this file can assert: the interesting subject is code in another repository, so
//! what is testable here is that the reference implementation satisfies the bar
//! and that the bar is reachable through the crate's public API — the same way a
//! downstream host will reach it.
//!
//! The adversarial half (a saboteur per check, each failing exactly the rule it
//! violates) lives in the module's unit tests, where the saboteurs can stay
//! private.

use contextgraph_conformance::{
    CCHECK_BUDGET_BOUND, CCHECK_DETERMINISM, CCHECK_QUARANTINE, CCHECK_TOTAL_PARTITION,
    CheckStatus, ReferenceComposingHost, run_composition_conformance,
};

#[tokio::test]
async fn the_reference_composition_layer_upholds_every_composition_rule() {
    let report =
        run_composition_conformance(&ReferenceComposingHost, "reference: compose_for_prompt").await;
    assert!(
        report.passed(),
        "the reference composition layer must satisfy the suite it defines; failures: {:?}",
        report
            .failures()
            .map(|check| format!("{}: {}", check.name, check.evidence))
            .collect::<Vec<_>>()
    );

    // Every check ran and passed — none skipped, none vacuous.
    assert_eq!(report.checks.len(), 4);
    for name in [
        CCHECK_BUDGET_BOUND,
        CCHECK_TOTAL_PARTITION,
        CCHECK_QUARANTINE,
        CCHECK_DETERMINISM,
    ] {
        let status = report
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("report is missing the `{name}` check"))
            .status;
        assert_eq!(status, CheckStatus::Pass, "{name}: {report:?}");
    }

    // The target string is carried through verbatim, so a downstream host's CI
    // output names the host that was certified rather than just "passed".
    assert_eq!(report.target, "reference: compose_for_prompt");
}
