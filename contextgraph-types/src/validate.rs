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

/// Render a Unix instant (seconds since the epoch, UTC) as a protocol
/// timestamp: the exact spelling [`is_protocol_timestamp`] accepts.
///
/// This exists so a host can *stamp* a temporal field without taking on a date
/// library. `contextgraph-types` deliberately carries no dependency beyond
/// serde (see the module docs), and every implementer in every language has to
/// reproduce whatever this crate does — so "pull in chrono" is a cost paid by
/// the whole ecosystem, for arithmetic that fits in twenty lines.
///
/// The gap it closes is concrete: the host recorded every consent decision with
/// `granted_at: None`, because it had no way to spell the current instant. An
/// audit ledger that never records *when* is missing the field that makes it an
/// audit ledger.
///
/// Leap seconds are not representable — a Unix timestamp cannot express one —
/// so this never emits `:60`, though the validator accepts it from a peer.
///
/// ```
/// use contextgraph_types::{format_protocol_timestamp, is_protocol_timestamp};
///
/// assert_eq!(format_protocol_timestamp(0), "1970-01-01T00:00:00Z");
/// assert_eq!(format_protocol_timestamp(1_784_000_000), "2026-07-14T03:33:20Z");
/// assert!(is_protocol_timestamp(&format_protocol_timestamp(1_784_000_000)));
/// ```
pub fn format_protocol_timestamp(unix_seconds: i64) -> String {
    // `div_euclid`/`rem_euclid` rather than `/` and `%`: for instants before
    // the epoch the truncating operators would round the day *up* and yield a
    // negative time-of-day.
    let days = unix_seconds.div_euclid(SECONDS_PER_DAY);
    let second_of_day = unix_seconds.rem_euclid(SECONDS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

const SECONDS_PER_DAY: i64 = 86_400;

/// Civil (proleptic Gregorian) date from a day count relative to 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, the standard formulation. It shifts the
/// epoch to 0000-03-01 so the leap day lands at the *end* of the year, which is
/// what lets the era arithmetic below avoid special-casing February.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // 719_468 = days from 0000-03-01 to 1970-01-01.
    let z = days + 719_468;
    // A 400-year era is exactly 146_097 days — the Gregorian cycle.
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097); // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    // Month index in the March-based year.
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }; // [1, 12]
    // January and February belong to the following calendar year.
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
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
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
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
    fn formats_known_instants() {
        assert_eq!(format_protocol_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_protocol_timestamp(1), "1970-01-01T00:00:01Z");
        assert_eq!(format_protocol_timestamp(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(format_protocol_timestamp(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(
            format_protocol_timestamp(1_784_000_000),
            "2026-07-14T03:33:20Z"
        );
    }

    #[test]
    fn formats_leap_days_and_century_rules() {
        // 2000 is a leap year (divisible by 400); 1900 was not (divisible by
        // 100 but not 400). The era arithmetic has to get both right.
        assert_eq!(
            format_protocol_timestamp(951_782_400),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            format_protocol_timestamp(-2_203_891_200),
            "1900-03-01T00:00:00Z"
        );
        assert_eq!(
            format_protocol_timestamp(1_709_164_800),
            "2024-02-29T00:00:00Z"
        );
    }

    #[test]
    fn formats_instants_before_the_epoch_without_rounding_the_day_up() {
        // The trap `div_euclid` avoids: truncating division would place this
        // one second *before* the epoch on 1970-01-01 with a negative clock.
        assert_eq!(format_protocol_timestamp(-1), "1969-12-31T23:59:59Z");
        assert_eq!(format_protocol_timestamp(-86_400), "1969-12-31T00:00:00Z");
    }

    /// The formatter and the validator must agree — a host that stamps a field
    /// with the one and is checked by the other cannot be allowed to disagree.
    #[test]
    fn everything_it_formats_is_a_valid_protocol_timestamp() {
        // A wide spread: pre-epoch, epoch, leap days, far future, and a stride
        // that lands on assorted times of day.
        let mut instants = vec![-2_203_891_200, -86_401, -1, 0, 951_782_400, 1_784_000_000];
        let mut t = -62_135_596_800; // 0001-01-01T00:00:00Z
        while t < 4_102_444_800 {
            // through 2100
            instants.push(t);
            t += 999_999_937; // a prime-ish stride, so it doesn't align to days
        }
        for instant in instants {
            let formatted = format_protocol_timestamp(instant);
            assert!(
                is_protocol_timestamp(&formatted),
                "format_protocol_timestamp({instant}) produced `{formatted}`, which the validator rejects"
            );
        }
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
