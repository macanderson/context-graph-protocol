//! Structured error codes (`SPEC.md` §Errors, issue #9).
//!
//! Before this module the wire error was a bare string. A host receiving one
//! could log it and nothing else: it could not distinguish "your query was
//! malformed" (do not retry) from "I am overloaded" (retry with backoff) from
//! "I do not serve that frame kind" (stop asking) from "internal fault" (fail
//! over). Every SDK and host would have invented its own message-string
//! sniffing — exactly the convention-over-contract the protocol exists to
//! eliminate.
//!
//! [`ErrorCode`] is a small, open vocabulary carried **alongside** the
//! free-form message, never replacing it: the code is for the machine, the
//! message is for the human reading the log.
//!
//! # Forward compatibility
//!
//! The vocabulary is open. An unrecognised code round-trips losslessly as
//! [`ErrorCode::Unknown`] and **MUST** be reacted to as though it were
//! [`ErrorCode::Internal`] — the conservative choice, since a host that guessed
//! optimistically about an error it does not understand would retry things it
//! should not. This is what lets the vocabulary grow in a 1.x minor without
//! breaking deployed hosts.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What a host should do about a provider error. Advisory guidance attached to
/// each [`ErrorCode`], so the reaction lives with the vocabulary rather than
/// being re-derived by every host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReaction {
    /// The request itself was wrong. Retrying it unchanged will fail again.
    DoNotRetry,
    /// The provider does not serve what was asked for. Narrow the query's
    /// `kinds`, or stop querying this provider for them.
    NarrowOrSkip,
    /// No useful frame fits the stated budget. Raise `max_tokens` or skip.
    RaiseBudgetOrSkip,
    /// Transient. Retry with backoff.
    RetryWithBackoff,
    /// The provider is tearing down. Re-spawn it or drop it from the fan-out.
    Respawn,
    /// A provider fault. Report it and count it against the provider's health.
    ReportAndCount,
}

/// A machine-readable provider error code.
///
/// Serializes as a snake_case string, so the wire stays human-diffable:
/// `{"type":"error","code":"unsupported_kind","message":"..."}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// Malformed or unintelligible query.
    BadRequest,
    /// The requested frame kinds are not served by this provider.
    UnsupportedKind,
    /// The budget is too small for any meaningful frame.
    BudgetUnsatisfiable,
    /// Transient overload, or a backing store is down.
    Unavailable,
    /// The provider is shutting down.
    ShuttingDown,
    /// A provider-side fault.
    Internal,
    /// A code this implementation does not recognise. Preserved verbatim so it
    /// survives a round-trip, and treated as [`Internal`](Self::Internal) for
    /// the purpose of [`reaction`](Self::reaction).
    Unknown(String),
}

impl ErrorCode {
    /// The wire spelling of this code.
    pub fn as_str(&self) -> &str {
        match self {
            Self::BadRequest => "bad_request",
            Self::UnsupportedKind => "unsupported_kind",
            Self::BudgetUnsatisfiable => "budget_unsatisfiable",
            Self::Unavailable => "unavailable",
            Self::ShuttingDown => "shutting_down",
            Self::Internal => "internal",
            Self::Unknown(raw) => raw,
        }
    }

    /// The advised host reaction. An unknown code reacts as `Internal` — see
    /// the module docs on why the conservative choice is the correct one.
    pub fn reaction(&self) -> HostReaction {
        match self {
            Self::BadRequest => HostReaction::DoNotRetry,
            Self::UnsupportedKind => HostReaction::NarrowOrSkip,
            Self::BudgetUnsatisfiable => HostReaction::RaiseBudgetOrSkip,
            Self::Unavailable => HostReaction::RetryWithBackoff,
            Self::ShuttingDown => HostReaction::Respawn,
            Self::Internal | Self::Unknown(_) => HostReaction::ReportAndCount,
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.reaction(),
            HostReaction::RetryWithBackoff | HostReaction::Respawn
        )
    }

    /// Whether this code was recognised by this implementation.
    pub fn is_recognized(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl From<&str> for ErrorCode {
    fn from(raw: &str) -> Self {
        match raw {
            "bad_request" => Self::BadRequest,
            "unsupported_kind" => Self::UnsupportedKind,
            "budget_unsatisfiable" => Self::BudgetUnsatisfiable,
            "unavailable" => Self::Unavailable,
            "shutting_down" => Self::ShuttingDown,
            "internal" => Self::Internal,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // A plain `#[derive(Deserialize)]` on a unit-variant enum would *reject*
        // an unrecognised code, which would make adding a code in a 1.x minor a
        // breaking change for every deployed host. Going through the string
        // keeps the vocabulary open.
        let raw = String::deserialize(d)?;
        Ok(ErrorCode::from(raw.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_code_roundtrips_through_its_wire_spelling() {
        let codes = [
            ErrorCode::BadRequest,
            ErrorCode::UnsupportedKind,
            ErrorCode::BudgetUnsatisfiable,
            ErrorCode::Unavailable,
            ErrorCode::ShuttingDown,
            ErrorCode::Internal,
        ];
        for code in codes {
            let json = serde_json::to_string(&code).unwrap();
            let back: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, code, "{code} did not survive a round-trip");
            assert!(code.is_recognized());
        }
    }

    #[test]
    fn codes_serialize_as_bare_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::UnsupportedKind).unwrap(),
            "\"unsupported_kind\""
        );
    }

    #[test]
    fn an_unknown_code_survives_a_roundtrip_verbatim() {
        // Forward compatibility: a host built today must be able to receive,
        // log, and re-emit a code added to the spec tomorrow.
        let code: ErrorCode = serde_json::from_str("\"quota_exceeded\"").unwrap();
        assert_eq!(code, ErrorCode::Unknown("quota_exceeded".into()));
        assert_eq!(serde_json::to_string(&code).unwrap(), "\"quota_exceeded\"");
        assert!(!code.is_recognized());
    }

    #[test]
    fn an_unknown_code_reacts_as_internal_never_as_retryable() {
        // The conservative default: a host that optimistically retried an
        // error it did not understand could hammer a provider that told it to
        // stop.
        let code = ErrorCode::Unknown("something_new".into());
        assert_eq!(code.reaction(), HostReaction::ReportAndCount);
        assert!(!code.is_retryable());
    }

    #[test]
    fn reactions_separate_retryable_faults_from_permanent_ones() {
        assert!(ErrorCode::Unavailable.is_retryable());
        assert!(ErrorCode::ShuttingDown.is_retryable());

        assert!(!ErrorCode::BadRequest.is_retryable());
        assert!(!ErrorCode::UnsupportedKind.is_retryable());
        assert!(!ErrorCode::BudgetUnsatisfiable.is_retryable());
        assert!(!ErrorCode::Internal.is_retryable());
    }
}
