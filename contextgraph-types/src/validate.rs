//! Format validators for fields the protocol declares but could not previously
//! check (`SPEC.md` §F4, §F5; issues #10 and #12).
//!
//! Two guarantees were unfalsifiable before this module existed:
//!
//! - **Bi-temporal retrieval.** `valid_from` / `valid_to` / `recorded_at` /
//!   `as_of` were free-form strings with no format rule anywhere, so a provider
//!   emitting `"valid_from": "last tuesday"` was fully conformant. A guarantee
//!   nothing can falsify is not a guarantee.
//! - **Provenance integrity.** `Provenance.digest` was documented as the thing
//!   that lets a host detect tampering "before the frame enters a prompt", but
//!   no grammar said which algorithms were valid, what case the hex was in, or
//!   which bytes were digested. Two independent providers would disagree and
//!   both would be "conformant".
//!
//! These validators are deliberately dependency-free — no `chrono`, no `regex`.
//! `contextgraph-types` is the crate every provider in every language ports
//! from, and each dependency it carries is one more thing an implementer has to
//! reproduce or justify.

/// Whether `s` is a timestamp in the protocol's temporal profile.
///
/// The profile is a **strict subset** of RFC 3339: `YYYY-MM-DDTHH:MM:SS(.f+)?Z`
/// — uppercase `T`, uppercase `Z`, UTC only.
///
/// Naming it a subset is deliberate honesty. RFC 3339 also permits a lowercase
/// `t`, a space separator, and numeric offsets like `+02:00`. Allowing those
/// would mean every implementation needs offset arithmetic just to compare two
/// timestamps, and two frames with the same instant would compare unequal as
/// strings — which quietly breaks the dedup and cache-key properties other
/// parts of the protocol depend on. One spelling per instant is worth more than
/// full generality here.
///
/// ```
/// use contextgraph_types::is_protocol_timestamp;
///
/// assert!(is_protocol_timestamp("2026-07-20T18:00:00Z"));
/// assert!(is_protocol_timestamp("2026-07-20T18:00:00.123Z"));
/// assert!(!is_protocol_timestamp("2026-07-20T18:00:00+02:00")); // UTC only
/// assert!(!is_protocol_timestamp("last tuesday"));
/// ```
pub fn is_protocol_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    // Shortest legal form is "YYYY-MM-DDTHH:MM:SSZ" = 20 bytes.
    if b.len() < 20 {
        return false;
    }
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return false;
    }
    if !b[..4].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Some(month) = two_digits(&b[5..7]) else {
        return false;
    };
    let Some(day) = two_digits(&b[8..10]) else {
        return false;
    };
    let Some(hour) = two_digits(&b[11..13]) else {
        return false;
    };
    let Some(minute) = two_digits(&b[14..16]) else {
        return false;
    };
    let Some(second) = two_digits(&b[17..19]) else {
        return false;
    };

    let year: u32 = s[..4].parse().unwrap_or(0);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return false;
    }
    // RFC 3339 permits second 60 to represent a leap second.
    if hour > 23 || minute > 59 || second > 60 {
        return false;
    }

    match &b[19..] {
        // No fractional part.
        [b'Z'] => true,
        // Fractional seconds: at least one digit, then Z.
        [b'.', rest @ ..] => {
            let Some((last, digits)) = rest.split_last() else {
                return false;
            };
            *last == b'Z' && !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

fn two_digits(pair: &[u8]) -> Option<u32> {
    if pair.len() == 2 && pair.iter().all(u8::is_ascii_digit) {
        Some((pair[0] - b'0') as u32 * 10 + (pair[1] - b'0') as u32)
    } else {
        None
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The digest algorithms this protocol revision defines.
///
/// The prefix is part of the grammar precisely so a future algorithm is an
/// additive change rather than an ambiguous reinterpretation of existing
/// digests.
pub const DIGEST_ALGORITHMS: &[&str] = &["sha256"];

/// Whether `s` is a well-formed content digest: `<algorithm>:<lowercase hex>`,
/// e.g. `sha256:` followed by 64 lowercase hex characters (`SPEC.md` §F5).
///
/// Lowercase is mandated rather than merely conventional: a digest is compared
/// byte-for-byte, and two implementations disagreeing on hex case would produce
/// spurious mismatches that look exactly like tampering.
///
/// This checks the *grammar* only. Whether the digest matches the bytes it
/// claims to cover is a separate, host-side question — see
/// `contextgraph_host::verify`.
///
/// ```
/// use contextgraph_types::is_well_formed_digest;
///
/// let ok = format!("sha256:{}", "a".repeat(64));
/// assert!(is_well_formed_digest(&ok));
/// assert!(!is_well_formed_digest("sha256:abc"));           // wrong length
/// assert!(!is_well_formed_digest(&ok.to_uppercase()));     // must be lowercase
/// ```
pub fn is_well_formed_digest(s: &str) -> bool {
    let Some((algorithm, hex)) = s.split_once(':') else {
        return false;
    };
    let expected_hex_len = match algorithm {
        "sha256" => 64,
        _ => return false,
    };
    hex.len() == expected_hex_len
        && hex
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(hex: &str) -> String {
        format!("sha256:{hex}")
    }

    #[test]
    fn accepts_the_canonical_timestamp_spelling() {
        assert!(is_protocol_timestamp("2026-07-20T18:00:00Z"));
        assert!(is_protocol_timestamp("1970-01-01T00:00:00Z"));
        assert!(is_protocol_timestamp("2026-12-31T23:59:59Z"));
    }

    #[test]
    fn accepts_fractional_seconds_of_any_precision() {
        assert!(is_protocol_timestamp("2026-07-20T18:00:00.1Z"));
        assert!(is_protocol_timestamp("2026-07-20T18:00:00.123Z"));
        assert!(is_protocol_timestamp("2026-07-20T18:00:00.123456789Z"));
    }

    #[test]
    fn rejects_prose_which_is_the_bug_this_check_exists_for() {
        // The literal example from issue #10: this was fully conformant before.
        assert!(!is_protocol_timestamp("last tuesday"));
        assert!(!is_protocol_timestamp(""));
        assert!(!is_protocol_timestamp("2026-07-20"));
    }

    #[test]
    fn rejects_non_utc_spellings_of_a_valid_instant() {
        // All of these are legal RFC 3339; none are in the protocol profile.
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00+02:00"));
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00-05:00"));
        assert!(!is_protocol_timestamp("2026-07-20t18:00:00Z"));
        assert!(!is_protocol_timestamp("2026-07-20 18:00:00Z"));
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00z"));
    }

    #[test]
    fn rejects_out_of_range_components() {
        assert!(!is_protocol_timestamp("2026-13-01T00:00:00Z")); // month 13
        assert!(!is_protocol_timestamp("2026-00-01T00:00:00Z")); // month 0
        assert!(!is_protocol_timestamp("2026-07-32T00:00:00Z")); // day 32
        assert!(!is_protocol_timestamp("2026-07-00T00:00:00Z")); // day 0
        assert!(!is_protocol_timestamp("2026-07-20T24:00:00Z")); // hour 24
        assert!(!is_protocol_timestamp("2026-07-20T00:60:00Z")); // minute 60
    }

    #[test]
    fn honors_month_lengths_and_leap_years() {
        assert!(is_protocol_timestamp("2026-01-31T00:00:00Z"));
        assert!(!is_protocol_timestamp("2026-04-31T00:00:00Z")); // April has 30

        assert!(!is_protocol_timestamp("2026-02-29T00:00:00Z")); // 2026 is not a leap year
        assert!(is_protocol_timestamp("2024-02-29T00:00:00Z")); // 2024 is
        assert!(is_protocol_timestamp("2000-02-29T00:00:00Z")); // 400-divisible
        assert!(!is_protocol_timestamp("1900-02-29T00:00:00Z")); // 100-divisible, not 400
    }

    #[test]
    fn accepts_a_leap_second() {
        // RFC 3339 permits :60 for a leap second; rejecting it would make the
        // protocol reject legitimate timestamps from correct clocks.
        assert!(is_protocol_timestamp("2016-12-31T23:59:60Z"));
        assert!(!is_protocol_timestamp("2016-12-31T23:59:61Z"));
    }

    #[test]
    fn rejects_a_malformed_fractional_part() {
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00.Z")); // no digits
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00.12")); // no Z
        assert!(!is_protocol_timestamp("2026-07-20T18:00:00.1a2Z")); // non-digit
    }

    #[test]
    fn accepts_a_well_formed_sha256_digest() {
        assert!(is_well_formed_digest(&digest(&"a".repeat(64))));
        assert!(is_well_formed_digest(&digest(
            &"0123456789abcdef".repeat(4)
        )));
    }

    #[test]
    fn rejects_the_placeholder_digest_the_repo_used_in_examples() {
        // `sha256:abc` appears throughout the pre-spec fixtures. It is not a
        // digest, and now it does not pass for one.
        assert!(!is_well_formed_digest("sha256:abc"));
    }

    #[test]
    fn rejects_uppercase_hex_so_comparison_never_yields_a_false_mismatch() {
        let upper = format!("sha256:{}", "A".repeat(64));
        assert!(!is_well_formed_digest(&upper));
    }

    #[test]
    fn rejects_a_missing_or_unknown_algorithm_prefix() {
        assert!(!is_well_formed_digest(&"a".repeat(64))); // bare hex, no prefix
        assert!(!is_well_formed_digest(&format!("md5:{}", "a".repeat(32))));
        assert!(!is_well_formed_digest(&format!(
            "sha512:{}",
            "a".repeat(64)
        )));
    }

    #[test]
    fn rejects_non_hex_characters_of_the_right_length() {
        assert!(!is_well_formed_digest(&digest(&"g".repeat(64))));
    }

    #[test]
    fn the_declared_algorithm_list_matches_what_the_validator_accepts() {
        // Keeps the public constant honest if a future algorithm is added to
        // one place and not the other.
        for algorithm in DIGEST_ALGORITHMS {
            let candidate = format!("{algorithm}:{}", "a".repeat(64));
            assert!(
                is_well_formed_digest(&candidate),
                "{algorithm} is advertised but not accepted"
            );
        }
    }
}
