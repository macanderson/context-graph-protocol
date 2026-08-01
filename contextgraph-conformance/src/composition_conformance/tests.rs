//! Tests for the composition suite.
//!
//! Two obligations, and the second matters more than the first. The reference
//! host must **pass** (a bar nothing clears is a bug in the bar), and each check
//! must **fail** a host that violates exactly the rule it names — otherwise the
//! suite is decoration. So every check gets a purpose-built saboteur: a
//! `ComposingHost` that is correct in every respect except one.
//!
//! This is the in-module equivalent of `.github/scripts/conformance-red.sh`,
//! which proves the provider suite bites by running it against a deliberately
//! broken provider. A green suite is only evidence if red is reachable.

use super::*;
use crate::report::CheckStatus;

/// The outcome of one check by name, so a test can assert on the check it is
/// about rather than on report-wide `passed()`.
fn status(report: &ConformanceReport, name: &str) -> CheckStatus {
    report
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("check `{name}` missing from report"))
        .status
}

fn evidence(report: &ConformanceReport, name: &str) -> String {
    report
        .checks
        .iter()
        .find(|check| check.name == name)
        .map(|check| check.evidence.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn the_reference_composing_host_passes_every_check() {
    let report = run_composition_conformance(&ReferenceComposingHost, "reference").await;
    assert!(
        report.passed(),
        "the reference composition layer must satisfy the suite it defines; failures: {:?}",
        report
            .failures()
            .map(|check| format!("{}: {}", check.name, check.evidence))
            .collect::<Vec<_>>()
    );
    assert_eq!(report.checks.len(), 4, "all four checks ran");
    assert_eq!(report.target, "reference");
}

/// A host that runs the fan-out and admits **everything** the providers returned,
/// with no shared-budget pack and no drop report. This is the naive composition —
/// and precisely the shape a host lands on when it assumes `query_all`'s
/// per-provider audit already bounded the total.
struct AdmitsEverything;

#[async_trait]
impl ComposingHost for AdmitsEverything {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        let mut host = Host::new();
        for provider in providers {
            host.register(provider);
        }
        let fanout = host.query_all(query).await;
        let admitted = fanout
            .outcomes
            .iter()
            .filter_map(|outcome| match &outcome.result {
                ProviderResult::Frames(result) => Some(
                    result
                        .frames
                        .iter()
                        .map(|frame| (outcome.provider_id.clone(), frame.clone())),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        Composition {
            admitted,
            dropped: vec![],
        }
    }
}

#[tokio::test]
async fn admitting_every_frame_fails_the_cross_provider_budget_bound() {
    let report = run_composition_conformance(&AdmitsEverything, "admits-everything").await;
    assert_eq!(
        status(&report, CCHECK_BUDGET_BOUND),
        CheckStatus::Fail,
        "three honest 400-token providers against a 1000-token budget sum to 1200 — a host \
         that admits them all is over budget: {}",
        evidence(&report, CCHECK_BUDGET_BOUND)
    );
    // It passes quarantine and determinism: it *is* composing from the audited
    // accepted set, and it *is* stable. That non-overlap is the point — each
    // check isolates one rule instead of every check failing together.
    assert_eq!(status(&report, CCHECK_QUARANTINE), CheckStatus::Pass);
    assert_eq!(status(&report, CCHECK_DETERMINISM), CheckStatus::Pass);
}

/// A host that packs to the token budget correctly but stops walking the moment
/// the budget fills, so the frames after that point are neither admitted nor
/// reported. The classic silent truncation: `break` where the code needed
/// `continue`-with-a-report.
struct SilentlyTruncates;

#[async_trait]
impl ComposingHost for SilentlyTruncates {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        let mut host = Host::new();
        for provider in providers {
            host.register(provider);
        }
        let fanout = host.query_all(query).await;
        let mut admitted = Vec::new();
        let mut spent = 0u32;
        for outcome in &fanout.outcomes {
            if let ProviderResult::Frames(result) = &outcome.result {
                for frame in &result.frames {
                    if spent.saturating_add(frame.token_cost) > query.max_tokens {
                        // The bug: stop, and say nothing about the rest.
                        break;
                    }
                    spent += frame.token_cost;
                    admitted.push((outcome.provider_id.clone(), frame.clone()));
                }
            }
        }
        Composition {
            admitted,
            dropped: vec![],
        }
    }
}

#[tokio::test]
async fn silent_truncation_fails_the_total_partition() {
    let report = run_composition_conformance(&SilentlyTruncates, "silently-truncates").await;
    assert_eq!(
        status(&report, CCHECK_TOTAL_PARTITION),
        CheckStatus::Fail,
        "a frame that is neither admitted nor reported has vanished unaccounted: {}",
        evidence(&report, CCHECK_TOTAL_PARTITION)
    );
    // Staying within budget is not the failing rule here — this host does that.
    // The check that fires is the one about accounting, which is the distinction
    // the two checks exist to draw.
    assert_eq!(status(&report, CCHECK_BUDGET_BOUND), CheckStatus::Fail);
    assert!(
        evidence(&report, CCHECK_BUDGET_BOUND).contains("report a drop=false"),
        "budget-bound fails on the missing drop report, not on the bound itself: {}",
        evidence(&report, CCHECK_BUDGET_BOUND)
    );
}

/// A host that composes from **raw provider results**, skipping the audit — so a
/// frame flooder's frames are put back after `query_all` rejected them. Every
/// frame it admits is well-formed and individually cheap, which is what makes
/// this failure invisible both to a frame-validity check and to its own
/// token-budget pack: nothing except the audit was ever going to keep them out.
struct IgnoresTheAudit;

#[async_trait]
impl ComposingHost for IgnoresTheAudit {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        // Query the providers directly, bypassing the host's audit entirely.
        let mut admitted = Vec::new();
        let mut dropped = Vec::new();
        let mut spent = 0u32;
        for provider in &providers {
            let Ok(result) = provider.query(query).await else {
                continue;
            };
            for frame in result.frames {
                if spent.saturating_add(frame.token_cost) > query.max_tokens {
                    dropped.push(ExcludedFrame {
                        provider_id: provider.id().to_string(),
                        frame_id: frame.id.clone(),
                    });
                    continue;
                }
                spent += frame.token_cost;
                admitted.push((provider.id().to_string(), frame));
            }
        }
        Composition { admitted, dropped }
    }
}

#[tokio::test]
async fn composing_from_raw_provider_results_fails_quarantine() {
    let report = run_composition_conformance(&IgnoresTheAudit, "ignores-the-audit").await;
    assert_eq!(
        status(&report, CCHECK_QUARANTINE),
        CheckStatus::Fail,
        "a host that skips the audit re-admits exactly what B4 rejected: {}",
        evidence(&report, CCHECK_QUARANTINE)
    );
    // It respects the budget and accounts for its drops — it is wrong in exactly
    // one way, and that is the way the check names.
    assert_eq!(status(&report, CCHECK_BUDGET_BOUND), CheckStatus::Pass);
    assert_eq!(status(&report, CCHECK_TOTAL_PARTITION), CheckStatus::Pass);
}

/// A host whose render order depends on something outside the frame set. Rather
/// than fake a clock or a hash seed, it alternates on an internal counter — the
/// observable behavior of any host whose ordering leaks nondeterminism, and the
/// reason the determinism check uses tied scores.
struct UnstableOrder {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ComposingHost for UnstableOrder {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        let inner = ReferenceComposingHost.compose(providers, query).await;
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut admitted = inner.admitted;
        if n % 2 == 1 {
            admitted.reverse();
        }
        Composition {
            admitted,
            dropped: inner.dropped,
        }
    }
}

#[tokio::test]
async fn an_order_that_varies_between_identical_calls_fails_determinism() {
    let host = UnstableOrder {
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let report = run_composition_conformance(&host, "unstable-order").await;
    assert_eq!(
        status(&report, CCHECK_DETERMINISM),
        CheckStatus::Fail,
        "an unchanged frame set that renders in a different order busts the prompt cache: {}",
        evidence(&report, CCHECK_DETERMINISM)
    );
}

/// A host that admits nothing at all. It cannot exceed a budget, cannot silently
/// truncate (it reports every frame as dropped), and is perfectly stable — so it
/// would sail through a suite written without well-behaved counterparts. Every
/// check must reject it.
struct AdmitsNothing;

#[async_trait]
impl ComposingHost for AdmitsNothing {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        let mut dropped = Vec::new();
        for provider in &providers {
            if let Ok(result) = provider.query(query).await {
                for frame in result.frames {
                    dropped.push(ExcludedFrame {
                        provider_id: provider.id().to_string(),
                        frame_id: frame.id,
                    });
                }
            }
        }
        Composition {
            admitted: vec![],
            dropped,
        }
    }
}

#[tokio::test]
async fn a_host_that_admits_nothing_passes_no_check_vacuously() {
    let report = run_composition_conformance(&AdmitsNothing, "admits-nothing").await;
    for check in &report.checks {
        assert_eq!(
            check.status,
            CheckStatus::Fail,
            "`{}` must not pass vacuously for a host that serves no context: {}",
            check.name,
            check.evidence
        );
    }
}
