//! The typed oracle report — the same pass/fail/skip + evidence vocabulary as
//! `contextgraph-conformance`'s report, applied to a journal instead of a live
//! provider. Kept as this crate's own small types rather than a dependency:
//! the conformance crate pulls in the host runtime (tokio, transports), and a
//! journal oracle must stay runnable anywhere the journal can be read.

use serde::{Deserialize, Serialize};

/// The verdict for a single oracle check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// Not exercised by this journal (e.g. `resume-integrity` on a run that
    /// never crashed) — declared honestly rather than counted as a pass.
    Skipped,
}

/// One check's outcome: which check, its verdict, and human-readable evidence
/// naming the exact `seq` numbers involved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub evidence: String,
}

impl CheckResult {
    pub fn pass(name: impl Into<String>, evidence: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Pass,
            evidence: evidence.into(),
        }
    }

    pub fn fail(name: impl Into<String>, evidence: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Fail,
            evidence: evidence.into(),
        }
    }

    pub fn skip(name: impl Into<String>, evidence: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Skipped,
            evidence: evidence.into(),
        }
    }

    /// Pass when `violations` is empty, otherwise fail with the violations
    /// joined — the common shape of every journal oracle.
    pub fn from_violations(
        name: impl Into<String>,
        violations: Vec<String>,
        pass_evidence: impl Into<String>,
    ) -> Self {
        if violations.is_empty() {
            Self::pass(name, pass_evidence)
        } else {
            Self::fail(name, violations.join("; "))
        }
    }
}

/// The result of running the oracles over one journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceReport {
    /// Human description of the journal under judgement
    /// ([`crate::Journal::describe`]).
    pub target: String,
    pub checks: Vec<CheckResult>,
}

impl TraceReport {
    /// True when no check failed (skips don't fail a run) — the "this journal
    /// upholds the loop invariants" verdict.
    pub fn passed(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Fail)
    }

    /// The checks that failed, in order.
    pub fn failures(&self) -> impl Iterator<Item = &CheckResult> {
        self.checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
    }

    /// `(passed, failed, skipped)` tallies.
    pub fn tally(&self) -> (usize, usize, usize) {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for check in &self.checks {
            match check.status {
                CheckStatus::Pass => passed += 1,
                CheckStatus::Fail => failed += 1,
                CheckStatus::Skipped => skipped += 1,
            }
        }
        (passed, failed, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_passes_only_when_nothing_failed() {
        let clean = TraceReport {
            target: "session sess_1".into(),
            checks: vec![
                CheckResult::pass("sequence-integrity", "16 events, dense"),
                CheckResult::skip("resume-integrity", "no resume recorded"),
            ],
        };
        assert!(clean.passed());
        assert_eq!(clean.tally(), (1, 0, 1));

        let broken = TraceReport {
            target: "session sess_1".into(),
            checks: vec![CheckResult::from_violations(
                "effect-exactly-once",
                vec!["effect `write:x#1` replayed at seq 12".into()],
                "",
            )],
        };
        assert!(!broken.passed());
        assert_eq!(broken.failures().count(), 1);
    }

    #[test]
    fn from_violations_passes_on_empty_and_joins_on_failure() {
        let pass = CheckResult::from_violations("citation-at-use", vec![], "3 frame(s) labelled");
        assert_eq!(pass.status, CheckStatus::Pass);
        assert_eq!(pass.evidence, "3 frame(s) labelled");

        let fail =
            CheckResult::from_violations("citation-at-use", vec!["a".into(), "b".into()], "unused");
        assert_eq!(fail.status, CheckStatus::Fail);
        assert_eq!(fail.evidence, "a; b");
    }

    #[test]
    fn report_is_serde_roundtrippable_for_json_output() {
        let report = TraceReport {
            target: "session sess_1".into(),
            checks: vec![CheckResult::pass("turn-loop-pairing", "2 call(s) paired")],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: TraceReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }
}
