//! Key validity windows — the one definition of "was this signing key in
//! service?" that both attestation layers share
//! ([ADR 0028](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0028-key-validity-windows-are-evaluated-at-receipt.md)).
//!
//! A [`KeyValidity`] is an optional `not_before` and an optional `not_after`,
//! each a `SPEC.md` §6.1 (F4) protocol timestamp. It is carried by the
//! verifier's own record of a key — a frame-layer trust store, a record-layer
//! key ring — and **never** by an attestation. The protocol defines how a
//! window is spelled and how it is evaluated; whether a verifier keeps one for
//! a key is that verifier's decision.
//!
//! # Which clock decides
//!
//! A window is evaluated at **the instant the verifier received the evidence**,
//! never at the attestation's `issued_at`.
//!
//! `issued_at` is outside the signed preimage (`SPEC.md` §6.5.2, F18;
//! [ADR 0021](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0021-attestation-metadata-outside-the-signature.md)):
//! anyone who relays an attestation can rewrite it to any instant and the
//! signature still verifies. A window checked against it would stop exactly
//! one party — an honest signer who states the true time — and would wave
//! through the party `not_after` exists for, the holder of a lapsed or stolen
//! key, who simply writes an in-window date.
//!
//! The receipt instant cannot be forged by the signer, and it is a **sound
//! upper bound** on when the signature was made: nothing can be received
//! before it exists. So "received no later than `not_after`" proves "signed no
//! later than `not_after`", which is the property a key lapse needs. A live
//! host passes its own clock at the moment the answer arrives. An auditor
//! replaying archived evidence passes the receipt instant that was recorded
//! when the evidence was first taken in — so an attestation received while its
//! key was in service verifies forever, and one received after the key lapsed
//! never did. What the auditor cannot do is prove that instant to a third
//! party: it is the verifier's own record, trusted exactly as far as the
//! verifier is, like the trust store itself.
//!
//! # Bounds are inclusive, and absent means unbounded
//!
//! A key is in service at `at` when `not_before <= at <= not_after`, each bound
//! applying only when present — the convention X.509 uses for `notBefore` and
//! `notAfter` (RFC 5280 §4.1.2.5), so an operator copying dates from a
//! certificate gets the meaning they expect. A window with neither bound is
//! [unbounded](KeyValidity::is_unbounded) and admits every instant, including
//! one that is not a well-formed timestamp: a key with no window behaves
//! exactly as a key did before windows existed.
//!
//! # A window that cannot be evaluated fails closed
//!
//! A malformed bound, an inverted window, or a malformed instant evaluated
//! against a bounded window is never [`Within`](WindowPosition::Within). "I
//! cannot tell whether this key was in service" is not "it was".

use core::cmp::Ordering;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::validate::{compare_protocol_timestamps, is_protocol_timestamp};

/// When a signing key is in service: an optional inclusive `not_before` and an
/// optional inclusive `not_after`, each a `SPEC.md` §6.1 protocol timestamp.
///
/// The fields are public so a verifier can persist and display a window as it
/// was written; [`new`](Self::new) is the constructor that refuses a malformed
/// or inverted one at the door. A window assembled around it is still checked
/// every time it is [evaluated](Self::position), and fails closed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyValidity {
    /// The first instant the key is in service, inclusive. Absent: no lower
    /// bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<String>,
    /// The last instant the key is in service, inclusive. Absent: no upper
    /// bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<String>,
}

/// Why a [`KeyValidity`] was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyValidityError {
    /// `not_before` is not a `SPEC.md` §6.1 protocol timestamp.
    MalformedNotBefore(String),
    /// `not_after` is not a `SPEC.md` §6.1 protocol timestamp.
    MalformedNotAfter(String),
    /// `not_before` is later than `not_after`, so no instant is in the window.
    /// Refused rather than kept as a key that can never verify: that is almost
    /// always two dates pasted the wrong way round, and a person should hear
    /// about it where they typed it.
    Inverted {
        /// The lower bound as written.
        not_before: String,
        /// The upper bound as written.
        not_after: String,
    },
}

impl fmt::Display for KeyValidityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedNotBefore(value) => write!(
                f,
                "not_before `{value}` is not a protocol timestamp (YYYY-MM-DDTHH:MM:SS(.f+)?Z)"
            ),
            Self::MalformedNotAfter(value) => write!(
                f,
                "not_after `{value}` is not a protocol timestamp (YYYY-MM-DDTHH:MM:SS(.f+)?Z)"
            ),
            Self::Inverted {
                not_before,
                not_after,
            } => write!(
                f,
                "not_before `{not_before}` is later than not_after `{not_after}`, so the key \
                 would never be in service"
            ),
        }
    }
}

impl std::error::Error for KeyValidityError {}

/// Where an instant falls relative to a [`KeyValidity`] window.
///
/// Named rather than a boolean for the reason every verdict in this crate is:
/// a key that has not entered service, a key that has lapsed, and a window
/// nobody can evaluate are three different findings, and the second is the one
/// an operator rotating keys is waiting to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowPosition {
    /// The instant is inside the window (or the window is unbounded).
    Within,
    /// The instant is before `not_before`: the key had not entered service.
    NotYetValid,
    /// The instant is after `not_after`: the key had lapsed.
    Expired,
    /// The window itself is malformed or inverted — a configuration defect in
    /// the verifier's own record of the key, not a finding about the signer.
    MalformedWindow,
    /// The window is bounded and the instant it was asked about is not a
    /// protocol timestamp, so there is nothing to compare.
    MalformedInstant,
}

impl WindowPosition {
    /// Whether the key was in service. Every other position is `false`,
    /// including the two that could not be evaluated.
    pub fn is_within(self) -> bool {
        matches!(self, Self::Within)
    }
}

impl KeyValidity {
    /// A window with neither bound: every instant is in service. The default,
    /// and exactly the behavior of a key before windows existed.
    pub fn unbounded() -> Self {
        Self::default()
    }

    /// A window from its two optional bounds, refused when a bound is not a
    /// protocol timestamp or when `not_before` is later than `not_after`.
    ///
    /// ```
    /// use contextgraph_types::{KeyValidity, KeyValidityError};
    ///
    /// let window = KeyValidity::new(
    ///     Some("2026-01-01T00:00:00Z".into()),
    ///     Some("2026-12-31T23:59:59Z".into()),
    /// )
    /// .expect("a well-formed window");
    /// assert!(window.position("2026-06-01T00:00:00Z").is_within());
    ///
    /// assert!(matches!(
    ///     KeyValidity::new(Some("2027-01-01T00:00:00Z".into()), Some("2026-01-01T00:00:00Z".into())),
    ///     Err(KeyValidityError::Inverted { .. })
    /// ));
    /// ```
    pub fn new(
        not_before: Option<String>,
        not_after: Option<String>,
    ) -> Result<Self, KeyValidityError> {
        let window = Self {
            not_before,
            not_after,
        };
        window.validate()?;
        Ok(window)
    }

    /// Whether this window has neither bound.
    pub fn is_unbounded(&self) -> bool {
        self.not_before.is_none() && self.not_after.is_none()
    }

    /// Check that every present bound is a protocol timestamp and that the
    /// window is not inverted. An unbounded window is always well-formed.
    pub fn validate(&self) -> Result<(), KeyValidityError> {
        if let Some(not_before) = &self.not_before
            && !is_protocol_timestamp(not_before)
        {
            return Err(KeyValidityError::MalformedNotBefore(not_before.clone()));
        }
        if let Some(not_after) = &self.not_after
            && !is_protocol_timestamp(not_after)
        {
            return Err(KeyValidityError::MalformedNotAfter(not_after.clone()));
        }
        if let (Some(not_before), Some(not_after)) = (&self.not_before, &self.not_after)
            && compare_protocol_timestamps(not_before, not_after) == Some(Ordering::Greater)
        {
            return Err(KeyValidityError::Inverted {
                not_before: not_before.clone(),
                not_after: not_after.clone(),
            });
        }
        Ok(())
    }

    /// Where `at` falls relative to this window.
    ///
    /// `at` is **the instant the verifier received the evidence** — see the
    /// module documentation for why it is never the attestation's `issued_at`.
    ///
    /// An unbounded window answers [`Within`](WindowPosition::Within) without
    /// reading `at` at all, so a key with no window is unaffected by the
    /// verifier's clock, well-formed or not.
    pub fn position(&self, at: &str) -> WindowPosition {
        if self.is_unbounded() {
            return WindowPosition::Within;
        }
        if self.validate().is_err() {
            return WindowPosition::MalformedWindow;
        }
        if !is_protocol_timestamp(at) {
            return WindowPosition::MalformedInstant;
        }
        // Both sides are well-formed from here, so `compare_protocol_timestamps`
        // answers `Some`; the `== Some(..)` form keeps that an argument rather
        // than an `unwrap`.
        if let Some(not_before) = &self.not_before
            && compare_protocol_timestamps(at, not_before) == Some(Ordering::Less)
        {
            return WindowPosition::NotYetValid;
        }
        if let Some(not_after) = &self.not_after
            && compare_protocol_timestamps(at, not_after) == Some(Ordering::Greater)
        {
            return WindowPosition::Expired;
        }
        WindowPosition::Within
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAN: &str = "2026-01-01T00:00:00Z";
    const JUN: &str = "2026-06-01T00:00:00Z";
    const DEC: &str = "2026-12-31T23:59:59Z";

    fn window(not_before: Option<&str>, not_after: Option<&str>) -> KeyValidity {
        KeyValidity::new(not_before.map(Into::into), not_after.map(Into::into))
            .expect("well-formed window")
    }

    #[test]
    fn an_unbounded_window_admits_every_instant_even_a_malformed_one() {
        // A key with no window must behave exactly as before windows existed,
        // so the verifier's clock is never consulted for it.
        let always = KeyValidity::unbounded();
        assert!(always.is_unbounded());
        assert_eq!(always.position(JUN), WindowPosition::Within);
        assert_eq!(always.position("not a time"), WindowPosition::Within);
    }

    #[test]
    fn both_bounds_are_inclusive() {
        let year = window(Some(JAN), Some(DEC));
        assert_eq!(year.position(JAN), WindowPosition::Within);
        assert_eq!(year.position(DEC), WindowPosition::Within);
        assert_eq!(year.position(JUN), WindowPosition::Within);
        assert_eq!(
            year.position("2025-12-31T23:59:59.999Z"),
            WindowPosition::NotYetValid
        );
        assert_eq!(
            year.position("2026-12-31T23:59:59.001Z"),
            WindowPosition::Expired,
            "a fractional second past not_after is past it — bytewise order would say otherwise"
        );
    }

    #[test]
    fn a_single_bound_leaves_the_other_side_open() {
        let from = window(Some(JUN), None);
        assert_eq!(from.position(JAN), WindowPosition::NotYetValid);
        assert_eq!(
            from.position("2099-01-01T00:00:00Z"),
            WindowPosition::Within
        );

        let until = window(None, Some(JUN));
        assert_eq!(
            until.position("1999-01-01T00:00:00Z"),
            WindowPosition::Within
        );
        assert_eq!(until.position(DEC), WindowPosition::Expired);
    }

    #[test]
    fn a_window_that_cannot_be_evaluated_fails_closed() {
        let bounded = window(Some(JAN), Some(DEC));
        assert_eq!(
            bounded.position("yesterday"),
            WindowPosition::MalformedInstant
        );
        assert!(!bounded.position("yesterday").is_within());

        // Assembled around `new`, as a hand-edited file could be.
        let garbage = KeyValidity {
            not_before: Some("soon".into()),
            not_after: None,
        };
        assert_eq!(garbage.position(JUN), WindowPosition::MalformedWindow);
        let inverted = KeyValidity {
            not_before: Some(DEC.into()),
            not_after: Some(JAN.into()),
        };
        assert_eq!(inverted.position(JUN), WindowPosition::MalformedWindow);
    }

    #[test]
    fn malformed_and_inverted_windows_are_refused_at_the_door() {
        assert_eq!(
            KeyValidity::new(Some("soon".into()), None),
            Err(KeyValidityError::MalformedNotBefore("soon".into()))
        );
        assert_eq!(
            KeyValidity::new(None, Some("2026-06-01".into())),
            Err(KeyValidityError::MalformedNotAfter("2026-06-01".into()))
        );
        assert!(matches!(
            KeyValidity::new(Some(DEC.into()), Some(JAN.into())),
            Err(KeyValidityError::Inverted { .. })
        ));
        // A one-instant window is not inverted.
        assert!(KeyValidity::new(Some(JUN.into()), Some(JUN.into())).is_ok());
    }

    #[test]
    fn an_unbounded_window_serializes_to_nothing() {
        // So a persisted key written before windows existed and one written
        // after with no window are byte-identical.
        assert_eq!(
            serde_json::to_string(&KeyValidity::unbounded()).unwrap(),
            "{}"
        );
        let year = window(Some(JAN), Some(DEC));
        let back: KeyValidity =
            serde_json::from_str(&serde_json::to_string(&year).unwrap()).unwrap();
        assert_eq!(back, year);
    }
}
