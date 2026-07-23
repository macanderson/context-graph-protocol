//! The adversarial fixture suite — the same proof obligation
//! `contextgraph-example-docs --misbehave` discharges for the provider
//! conformance suite, applied to journals: a golden recording passes every
//! oracle, and one fixture per check trips **exactly** that check while
//! leaving every other one green. An oracle that can only pass a healthy
//! harness proves nothing; this is where each one demonstrates it catches
//! the broken harness it exists for.

use contextgraph_trace::{
    CHECK_ASSEMBLY_BUDGET, CHECK_CITATION, CHECK_COMPOSITION, CHECK_EFFECT_ONCE, CHECK_RESUME,
    CHECK_SEQUENCE, CHECK_STALENESS, CHECK_TURN_LOOP, CheckStatus, Journal, TraceReport,
    run_oracles,
};

fn load(fixture: &str) -> Journal {
    let path = format!("{}/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fixture {path} must be readable: {error}"));
    Journal::from_ndjson(&raw).unwrap_or_else(|error| panic!("fixture {path} must parse: {error}"))
}

fn statuses(report: &TraceReport) -> String {
    report
        .checks
        .iter()
        .map(|check| format!("{}={:?}: {}", check.name, check.status, check.evidence))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_golden_journal_passes_every_oracle() {
    let report = run_oracles(&load("golden.ndjson"));
    assert!(report.passed(), "golden must pass:\n{}", statuses(&report));

    // The golden journal exercises the context checks for real — a pass by
    // vacuity would prove nothing.
    for exercised in [CHECK_STALENESS, CHECK_COMPOSITION] {
        let check = report
            .checks
            .iter()
            .find(|check| check.name == exercised)
            .expect("check present");
        assert_eq!(
            check.status,
            CheckStatus::Pass,
            "golden must exercise {exercised}, not skip it: {}",
            check.evidence
        );
    }
    // ...while durability is honestly declared unexercised (no crash in the
    // golden run) rather than silently counted as upheld.
    let resume = report
        .checks
        .iter()
        .find(|check| check.name == CHECK_RESUME)
        .expect("check present");
    assert_eq!(resume.status, CheckStatus::Skipped);
}

#[test]
fn the_golden_resume_journal_proves_a_lossless_crash_recovery_passes() {
    let report = run_oracles(&load("golden-resume.ndjson"));
    assert!(
        report.passed(),
        "golden-resume must pass:\n{}",
        statuses(&report)
    );

    // Here durability IS exercised: the crash orphaned a tool call and an
    // effect, and the resume recovered exactly the recorded prefix.
    let resume = report
        .checks
        .iter()
        .find(|check| check.name == CHECK_RESUME)
        .expect("check present");
    assert_eq!(resume.status, CheckStatus::Pass, "{}", resume.evidence);
}

/// Every trip fixture fails exactly the check it is named for — and nothing
/// else. A fixture that trips two checks would mean the defects are not
/// independently detectable, and a fixture that trips none would mean the
/// oracle is decorative.
#[test]
fn each_trip_fixture_fails_exactly_the_check_it_is_named_for() {
    let cases = [
        ("trip-sequence-integrity.ndjson", CHECK_SEQUENCE),
        ("trip-turn-loop-pairing.ndjson", CHECK_TURN_LOOP),
        ("trip-assembly-budget-honesty.ndjson", CHECK_ASSEMBLY_BUDGET),
        ("trip-staleness-at-use.ndjson", CHECK_STALENESS),
        ("trip-citation-at-use.ndjson", CHECK_CITATION),
        ("trip-deterministic-composition.ndjson", CHECK_COMPOSITION),
        ("trip-effect-exactly-once.ndjson", CHECK_EFFECT_ONCE),
        ("trip-resume-integrity.ndjson", CHECK_RESUME),
    ];

    for (fixture, expected_failure) in cases {
        let report = run_oracles(&load(fixture));
        let failed: Vec<&str> = report.failures().map(|check| check.name.as_str()).collect();
        assert_eq!(
            failed,
            vec![expected_failure],
            "{fixture} must fail exactly `{expected_failure}`:\n{}",
            statuses(&report)
        );
    }
}

/// The evidence strings are the actionable half of a failure: each must name
/// the `seq` where the defect happened, because "durability failed" is not
/// something a harness author can act on.
#[test]
fn failures_name_the_seq_numbers_that_convict() {
    let cases = [
        ("trip-staleness-at-use.ndjson", "verified `stale` at seq 2"),
        (
            "trip-effect-exactly-once.ndjson",
            "replayed across the resume at seq 6",
        ),
        (
            "trip-resume-integrity.ndjson",
            "2 recorded event(s) invisible",
        ),
        ("trip-turn-loop-pairing.ndjson", "phantom execution"),
    ];
    for (fixture, expected_fragment) in cases {
        let report = run_oracles(&load(fixture));
        let evidence: Vec<String> = report
            .failures()
            .map(|check| check.evidence.clone())
            .collect();
        assert!(
            evidence.iter().any(|text| text.contains(expected_fragment)),
            "{fixture} evidence must contain {expected_fragment:?}, got: {evidence:?}"
        );
    }
}
