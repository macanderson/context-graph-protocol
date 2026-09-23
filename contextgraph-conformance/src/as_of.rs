//! **§Q2** — the `as-of-temporal` probe (`SPEC.md` §5.3,
//! [ADR 0022](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0022-as-of-is-a-point-in-window-predicate.md)).
//!
//! A query pinned with `as_of` asks what was true *at that instant*. Q2 makes
//! that a decidable predicate over fields the provider itself emits: every
//! returned frame's valid-time window must contain the pin, where the window is
//! half-open — `valid_from <= as_of < valid_to` — and an absent bound is
//! unbounded on that side. A frame carrying neither bound makes no temporal
//! claim and is always admitted.
//!
//! The probe checks **both** halves. Before Q2 existed it read `valid_from`
//! alone, so a frame whose `valid_to` had passed years before the pin — a fact
//! that had stopped being true — was returned for a pinned query and the check
//! passed. Premature content was enforced without being specified; stale
//! content was specified by nothing and checked by nothing.
//!
//! It lives in its own module rather than in `lib.rs` because it is a probe
//! with its own pins, its own predicate, and its own unit tests; the check-name
//! constant stays in `lib.rs` with the others, where
//! `check-conformance-counts.py` reads it.

use std::cmp::Ordering;

use contextgraph_host::Host;
use contextgraph_types::{ContextFrame, ContextQuery, is_protocol_timestamp};

use crate::{CHECK_AS_OF, CheckResult, sample_query};

/// The instants the probe pins retrieval to, in the order it fires them.
///
/// Two pins, because one cannot witness both halves of the window against a
/// provider that has content on both sides of it. The reference fixture serves
/// one frame valid over `[2026-01-01, 2026-09-01)` and one valid from
/// `2026-09-01` onwards:
///
/// - the **mid-year** pin falls inside the first window and before the second,
///   so an honest answer keeps the first and omits the second — the
///   `valid_from` half has observable work to do;
/// - the **autumn** pin falls after the first window closed and inside the
///   second, so an honest answer omits the first — the `valid_to` half has
///   observable work to do.
///
/// Against any other provider both pins are simply two instants; the predicate
/// does not depend on the fixture's data.
pub(crate) const AS_OF_PINS: [&str; 2] = ["2026-07-01T00:00:00Z", "2026-10-01T00:00:00Z"];

/// Compare two protocol timestamps (`SPEC.md` §F4) as instants.
///
/// `None` when either is not in the profile — a malformed timestamp is
/// `frame-validity`'s finding, and this probe does not report it twice.
///
/// Lexicographic order on the whole string is *almost* chronological under the
/// profile, and the "almost" is the reason this function exists: an optional
/// fractional part means `…:00.5Z` sorts before `…:00Z` (`'.'` is below `'Z'`)
/// although it is half a second later, and `…:00Z` and `…:00.0Z` spell the same
/// instant differently. So the fixed-width `YYYY-MM-DDTHH:MM:SS` prefix is
/// compared lexicographically — which is chronological, every field being
/// zero-padded — and the fraction is compared as a decimal with trailing zeros
/// ignored.
pub(crate) fn compare_instants(a: &str, b: &str) -> Option<Ordering> {
    if !is_protocol_timestamp(a) || !is_protocol_timestamp(b) {
        return None;
    }
    // Both are ASCII and at least 20 bytes, so byte 19 is a char boundary.
    let (a_whole, a_rest) = a.split_at(19);
    let (b_whole, b_rest) = b.split_at(19);
    let fraction = |rest: &str| -> String {
        rest.trim_start_matches('.')
            .trim_end_matches('Z')
            .trim_end_matches('0')
            .to_string()
    };
    let (a_fraction, b_fraction) = (fraction(a_rest), fraction(b_rest));
    // Right-pad the shorter fraction so digit strings of equal length compare
    // numerically.
    let width = a_fraction.len().max(b_fraction.len());
    let pad = |digits: &str| format!("{digits:0<width$}");
    Some(
        a_whole
            .cmp(b_whole)
            .then_with(|| pad(&a_fraction).cmp(&pad(&b_fraction))),
    )
}

/// Why `frame` falls outside the Q2 window at `as_of`, or `None` when the
/// window contains the pin.
///
/// The window is half-open, `[valid_from, valid_to)`: a frame is admitted at
/// the instant it becomes true and excluded at the instant it stops being true,
/// so two consecutive windows that share a boundary never both admit the
/// boundary instant. A bound that is absent, or not in the §F4 profile (which
/// `frame-validity` reports), constrains nothing here.
pub(crate) fn outside_window(frame: &ContextFrame, as_of: &str) -> Option<String> {
    if let Some(valid_from) = frame.valid_from.as_deref()
        && compare_instants(valid_from, as_of) == Some(Ordering::Greater)
    {
        return Some(format!(
            "{} (valid_from={valid_from} is after the pin: not yet true)",
            frame.id
        ));
    }
    if let Some(valid_to) = frame.valid_to.as_deref()
        && matches!(
            compare_instants(valid_to, as_of),
            Some(Ordering::Less | Ordering::Equal)
        )
    {
        return Some(format!(
            "{} (valid_to={valid_to} is not after the pin: no longer true)",
            frame.id
        ));
    }
    None
}

/// The [`sample_query`] pinned to `pin` — everything else held equal, so only
/// the pin varies between the unpinned and pinned probes.
fn pinned_query(pin: &str) -> ContextQuery {
    ContextQuery {
        as_of: Some(pin.into()),
        ..sample_query()
    }
}

/// Probe `as_of` (`SPEC.md` §5.3, Q2): at each of [`AS_OF_PINS`], no returned
/// frame may fall outside its own valid-time window.
///
/// One-sided by construction: Q2 says which frames are *eligible*, never how
/// many a provider must return, so an answer with fewer frames — or none —
/// never fails it. A provider serving no dated content trivially passes, which
/// is the point of making the predicate range over the provider's own fields:
/// every provider can comply, so no capability flag and no `unsupported_as_of`
/// error are needed (ADR 0022).
pub(crate) async fn check_as_of(host: &Host, id: &str) -> CheckResult {
    let mut returned = Vec::with_capacity(AS_OF_PINS.len());
    let mut violations = Vec::new();
    for pin in AS_OF_PINS {
        match host.query_provider(id, &pinned_query(pin)).await {
            Ok(result) => {
                returned.push(format!("{} at as_of={pin}", result.frames.len()));
                violations.extend(
                    result
                        .frames
                        .iter()
                        .filter_map(|frame| outside_window(frame, pin))
                        .map(|why| format!("as_of={pin}: {why}")),
                );
            }
            Err(error) => {
                return CheckResult::fail(
                    CHECK_AS_OF,
                    format!("as_of={pin} query failed: {error}"),
                );
            }
        }
    }
    if violations.is_empty() {
        CheckResult::pass(
            CHECK_AS_OF,
            format!(
                "every returned frame's [valid_from, valid_to) window contains its pin — frames returned: {} (§5.3 Q2)",
                returned.join(", ")
            ),
        )
    } else {
        CheckResult::fail(
            CHECK_AS_OF,
            format!(
                "provider returned {} frame(s) outside their valid-time window at the pinned instant — Q2 requires valid_from <= as_of < valid_to (§5.3): {}",
                violations.len(),
                violations.join("; ")
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dated(valid_from: Option<&str>, valid_to: Option<&str>) -> ContextFrame {
        let mut frame: ContextFrame = serde_json::from_value(serde_json::json!({
            "id": "frm",
            "kind": "doc",
            "title": "t",
            "content": "c",
            "score": 0.5,
            "token_cost": 1,
            "citation_label": "t",
        }))
        .expect("a minimal frame parses");
        frame.valid_from = valid_from.map(String::from);
        frame.valid_to = valid_to.map(String::from);
        frame
    }

    const PIN: &str = "2026-07-01T00:00:00Z";

    #[test]
    fn an_undated_frame_makes_no_temporal_claim_and_is_admitted() {
        assert_eq!(outside_window(&dated(None, None), PIN), None);
    }

    #[test]
    fn the_window_is_closed_at_valid_from() {
        // Admitted at the instant it becomes true.
        assert_eq!(outside_window(&dated(Some(PIN), None), PIN), None);
        assert!(outside_window(&dated(Some("2026-07-01T00:00:01Z"), None), PIN).is_some());
    }

    #[test]
    fn the_window_is_open_at_valid_to() {
        // Excluded at the instant it stops being true — the half the probe
        // did not check before Q2.
        assert!(outside_window(&dated(None, Some(PIN)), PIN).is_some());
        assert!(outside_window(&dated(None, Some("2020-01-01T00:00:00Z")), PIN).is_some());
        assert_eq!(
            outside_window(&dated(None, Some("2026-07-01T00:00:00.001Z")), PIN),
            None
        );
    }

    #[test]
    fn a_window_containing_the_pin_is_admitted() {
        let frame = dated(Some("2026-01-01T00:00:00Z"), Some("2026-09-01T00:00:00Z"));
        assert_eq!(outside_window(&frame, PIN), None);
    }

    #[test]
    fn fractional_seconds_compare_as_instants_not_as_strings() {
        // Lexicographically `.5Z` < `Z`; chronologically it is later.
        assert_eq!(
            compare_instants("2026-07-01T00:00:00.5Z", PIN),
            Some(Ordering::Greater)
        );
        // Two spellings of one instant.
        assert_eq!(
            compare_instants("2026-07-01T00:00:00.000Z", PIN),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_instants("2026-07-01T00:00:00.25Z", "2026-07-01T00:00:00.3Z"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn a_malformed_bound_is_left_to_frame_validity() {
        assert_eq!(compare_instants("last tuesday", PIN), None);
        assert_eq!(
            outside_window(&dated(Some("last tuesday"), None), PIN),
            None
        );
    }

    #[test]
    fn the_two_pins_straddle_the_fixture_boundary() {
        // The fixture's boundary instant must lie strictly between the pins,
        // or one half of the window goes unwitnessed against it.
        let boundary = "2026-09-01T00:00:00Z";
        assert_eq!(
            compare_instants(AS_OF_PINS[0], boundary),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_instants(AS_OF_PINS[1], boundary),
            Some(Ordering::Greater)
        );
    }
}
