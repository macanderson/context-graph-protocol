//! Host-side conformance suite (`SPEC.md` §11.1, issue #14) — the dual of
//! `conformance_suite.rs`. Where that drives adversarial *providers* and asserts
//! the suite catches them, this drives the reference host against adversarial
//! in-process providers and asserts the *host* upholds the rules that bind it.
//!
//! Each check is adversarial by construction (it passes only if the host both
//! catches a misbehaving provider and accepts a well-behaved one), so a green
//! run here is the host-side analogue of `conformance-red.sh`: every adversarial
//! provider was caught.

use contextgraph_conformance::{
    CheckStatus, HCHECK_BUDGET_DROP, HCHECK_CONSENT_GATE, HCHECK_CONTENT_QUOTING,
    HCHECK_FRAME_LIMIT, HCHECK_PROVENANCE_BYTES, HCHECK_SCOPE_RECEIPT, run_host_conformance,
};

#[tokio::test]
async fn the_reference_host_upholds_every_host_binding_rule() {
    let report = run_host_conformance().await;
    assert!(
        report.passed(),
        "the reference host must be conformant; failures: {:?}",
        report.failures().collect::<Vec<_>>()
    );
    // Every host-binding check ran and passed — none skipped, none vacuous.
    assert_eq!(report.checks.len(), 6);
    for name in [
        HCHECK_BUDGET_DROP,
        HCHECK_FRAME_LIMIT,
        HCHECK_CONSENT_GATE,
        HCHECK_SCOPE_RECEIPT,
        HCHECK_PROVENANCE_BYTES,
        HCHECK_CONTENT_QUOTING,
    ] {
        let status = report
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("report is missing the `{name}` check"))
            .status;
        assert_eq!(status, CheckStatus::Pass, "{name}: {report:?}");
    }
}
