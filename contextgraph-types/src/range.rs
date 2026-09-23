//! The `range` grammar of `file` provenance — which bytes a §F5 digest covers
//! (`SPEC.md` §6.2.1, F17;
//! [ADR 0020](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0020-provenance-range-grammar.md)).
//!
//! F5 digests "the exact UTF-8 source bytes addressed by `uri` + `range`". A
//! digest is compared byte for byte, so two implementations that disagree about
//! which bytes `L120-160` names compute different digests over identical files
//! — and the verifier reports that as **tampering**, not as a parsing
//! disagreement. The grammar is therefore normative, and this module is its
//! reference implementation:
//!
//! ```abnf
//! line-range  = %x4C line-number [ "-" line-number ]  ; "L", uppercase only
//! line-number = %x31-39 *DIGIT                         ; decimal, >= 1, no leading zero
//! ```
//!
//! and the addressing rules §6.2.1 states in prose:
//!
//! - **Lines split on LF (`0x0A`) only.** A line runs through and *including*
//!   its terminating `\n`, or to the end of the resource if none follows. A
//!   final `\n` does not open an extra empty line; an empty resource has zero
//!   lines. `\r` is an ordinary content byte — never stripped, never a
//!   terminator — which is §6.2's "no line-ending translation" applied to
//!   ranges. Splitting is over bytes, with no UTF-8 decoding (a multi-byte
//!   UTF-8 sequence never contains `0x0A`, so the two agree on valid input).
//! - **1-indexed, end inclusive.** `L2-3` is lines two and three; `L5` is
//!   `L5-5`.
//! - **An end past the last line clamps** to the last line. **A start past the
//!   last line is an error**: it addresses nothing, and an empty span is not
//!   something a range can mean.
//! - **An end before the start is an error**, never an empty digest.
//! - **Every other spelling is reserved** — byte, character, and column ranges,
//!   GitHub's `L10-L20`, a lowercase `l`. A verifier reports one as
//!   *unverifiable*: never a whole-resource fallback (that would confirm bytes
//!   the range never named), never a mismatch (that is the tampering signal).
//!
//! The end clamps, rather than erroring, because the digest and not the range
//! is the integrity check: a clamped span must still hash to the declared
//! digest, so clamping can never make altered bytes verify. Every reference
//! provider in this repository and all four SDK examples rely on it — they
//! declare `L1-40` over a four-line file. See ADR 0020 for the full argument.
//!
//! Dependency-free, like the rest of this crate's validators: a port in any
//! language reproduces this from the prose above and the reference vectors in
//! `tests/vectors/range-vectors.json`.

use core::fmt;
use core::ops::Range;

/// A parsed `line-range` (`SPEC.md` §6.2.1): an inclusive, 1-indexed span of
/// lines, with `start <= end` guaranteed by construction.
///
/// A line number too large for `usize` saturates to `usize::MAX` rather than
/// failing to parse. That keeps the grammar's meaning independent of the
/// verifier's integer width: an `end` beyond any representable line clamps like
/// any other end past the last line, and a `start` that large lies past the
/// last line of every resource that can exist, so it fails exactly as §6.2.1
/// says a start past the last line fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    start: usize,
    end: usize,
}

/// Why a `range` does not address a byte span (`SPEC.md` §6.2.1).
///
/// Each of these is reported by a verifier as *unverifiable* — none of them is
/// a finding about the bytes, so none is a mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineRangeError {
    /// The string is not a `line-range` at all: a missing or lowercase `L`, a
    /// zero or zero-padded line number, a sign, whitespace, a second `L`, or a
    /// byte/character/column form this revision reserves.
    Unrecognised,
    /// A grammatical range whose end precedes its start (`L5-3`). It addresses
    /// nothing, and an empty span is not something a range can mean.
    Inverted,
    /// The range starts past the resource's last line, so it addresses no bytes
    /// at all. Only knowable once the resource has been read.
    StartPastEnd {
        /// The first line the range names.
        start: usize,
        /// How many lines the resource actually has.
        lines: usize,
    },
}

impl fmt::Display for LineRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrecognised => write!(
                f,
                "not a line range; expected `L<start>` or `L<start>-<end>` with decimal line numbers from 1 (§6.2.1)"
            ),
            Self::Inverted => write!(f, "the range ends before it starts (§6.2.1)"),
            Self::StartPastEnd { start, lines } => write!(
                f,
                "the range starts at line {start} but the resource has {lines} line(s) (§6.2.1)"
            ),
        }
    }
}

impl std::error::Error for LineRangeError {}

impl LineRange {
    /// Parse a `range` string against the §6.2.1 grammar.
    ///
    /// ```
    /// use contextgraph_types::{LineRange, LineRangeError};
    ///
    /// let r = LineRange::parse("L120-160").unwrap();
    /// assert_eq!((r.start(), r.end()), (120, 160));
    /// assert_eq!(LineRange::parse("L7").unwrap().end(), 7);
    ///
    /// // Reserved spellings are unrecognised, not guessed at.
    /// for other in ["120-160", "l120", "L10-L20", "L0", "L07", "L+7", "B0-64"] {
    ///     assert_eq!(LineRange::parse(other), Err(LineRangeError::Unrecognised));
    /// }
    /// assert_eq!(LineRange::parse("L9-3"), Err(LineRangeError::Inverted));
    /// ```
    pub fn parse(spec: &str) -> Result<Self, LineRangeError> {
        let digits = spec.strip_prefix('L').ok_or(LineRangeError::Unrecognised)?;
        let (start, end) = match digits.split_once('-') {
            Some((first, last)) => (line_number(first)?, line_number(last)?),
            None => {
                let single = line_number(digits)?;
                (single, single)
            }
        };
        if end < start {
            return Err(LineRangeError::Inverted);
        }
        Ok(Self { start, end })
    }

    /// The first line addressed, 1-indexed.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The last line addressed, 1-indexed and inclusive, *before* clamping to
    /// the resource — see [`byte_span`](Self::byte_span).
    pub fn end(&self) -> usize {
        self.end
    }

    /// The byte span this range addresses in `bytes`, exactly as §6.2.1 defines
    /// it: from the first byte of line `start` through the terminating `\n` (if
    /// any) of line `end`, with `end` clamped to the last line.
    ///
    /// ```
    /// use contextgraph_types::{LineRange, LineRangeError};
    ///
    /// let text = b"alpha\r\nbeta\ngamma";
    /// let span = |r: &str| LineRange::parse(r).unwrap().byte_span(text);
    /// assert_eq!(&text[span("L1").unwrap()], b"alpha\r\n");   // `\r` kept, `\n` included
    /// assert_eq!(&text[span("L2-99").unwrap()], b"beta\ngamma"); // end clamps
    /// assert_eq!(span("L4"), Err(LineRangeError::StartPastEnd { start: 4, lines: 3 }));
    /// ```
    pub fn byte_span(&self, bytes: &[u8]) -> Result<Range<usize>, LineRangeError> {
        // Walk line starts, remembering where line `start` begins and where
        // line `end` finishes, without materialising every span: a range near
        // the top of a large file should not cost a pass over the whole of it
        // beyond the line it ends on.
        let mut line = 1usize; // the line that begins at `line_begin`
        let mut line_begin = 0usize;
        let mut from = None;
        for (i, &byte) in bytes.iter().enumerate() {
            if byte != b'\n' {
                continue;
            }
            if line == self.start {
                from = Some(line_begin);
            }
            if line == self.end {
                // `from` is set: `start <= end`, so line `start` was reached.
                return Ok(from.unwrap_or(line_begin)..i + 1);
            }
            line += 1;
            line_begin = i + 1;
        }
        // Out of newlines. A trailing unterminated line is a line; a final
        // `\n` does not open an extra empty one.
        let lines = if line_begin < bytes.len() {
            line
        } else {
            line - 1
        };
        if self.start > lines {
            return Err(LineRangeError::StartPastEnd {
                start: self.start,
                lines,
            });
        }
        // `end` lies past the last line: clamp to the end of the resource.
        let from = from.unwrap_or(line_begin);
        Ok(from..bytes.len())
    }
}

/// Whether `spec` is a `line-range` in the §6.2.1 grammar with `start <= end` —
/// the static half of F17, checkable without reading the resource.
///
/// ```
/// use contextgraph_types::is_well_formed_line_range;
///
/// assert!(is_well_formed_line_range("L120-160"));
/// assert!(!is_well_formed_line_range("120-160"));
/// ```
pub fn is_well_formed_line_range(spec: &str) -> bool {
    LineRange::parse(spec).is_ok()
}

/// A `line-number`: `%x31-39 *DIGIT`. Saturates rather than overflowing — see
/// [`LineRange`].
fn line_number(field: &str) -> Result<usize, LineRangeError> {
    let bytes = field.as_bytes();
    let well_formed =
        matches!(bytes.first(), Some(b'1'..=b'9')) && bytes.iter().all(u8::is_ascii_digit);
    if !well_formed {
        return Err(LineRangeError::Unrecognised);
    }
    // Only overflow can fail here: the digits were checked above.
    Ok(field.parse::<usize>().unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span<'a>(bytes: &'a [u8], spec: &str) -> Result<&'a [u8], LineRangeError> {
        let range = LineRange::parse(spec)?;
        range.byte_span(bytes).map(|r| &bytes[r])
    }

    #[test]
    fn the_grammar_accepts_exactly_the_canonical_line_forms() {
        for ok in [
            "L1",
            "L10",
            "L1-1",
            "L2-3",
            "L120-160",
            "L1-99999999999999999999999",
        ] {
            assert!(is_well_formed_line_range(ok), "{ok}");
        }
        for bad in [
            "", "L", "L-", "L-3", "L3-", "L0", "L0-3", "L3-0", "L01", "L1-02", "L+1", "L1-+2",
            " L1", "L1 ", "L 1", "l1", "1-3", "L1-L3", "L1:3", "L1..3", "B0-64", "C3-9", "L1-2-3",
            "L１",
        ] {
            assert_eq!(
                LineRange::parse(bad),
                Err(LineRangeError::Unrecognised),
                "{bad:?}"
            );
        }
        assert_eq!(LineRange::parse("L3-2"), Err(LineRangeError::Inverted));
    }

    #[test]
    fn an_oversized_line_number_saturates_so_the_end_clamps_and_the_start_fails() {
        let huge = "99999999999999999999999999";
        let text = b"one\ntwo\n";
        assert_eq!(span(text, &format!("L2-{huge}")).unwrap(), b"two\n");
        assert!(matches!(
            span(text, &format!("L{huge}")),
            Err(LineRangeError::StartPastEnd { .. })
        ));
    }

    #[test]
    fn lines_include_their_newline_and_keep_every_carriage_return() {
        let text = b"first\r\nsecond\nthird\r\n";
        assert_eq!(span(text, "L1").unwrap(), b"first\r\n");
        assert_eq!(span(text, "L2").unwrap(), b"second\n");
        assert_eq!(span(text, "L3").unwrap(), b"third\r\n");
        assert_eq!(span(text, "L1-3").unwrap(), text);
        // A trailing newline does not open a fourth, empty line.
        assert_eq!(
            span(text, "L4"),
            Err(LineRangeError::StartPastEnd { start: 4, lines: 3 })
        );
    }

    #[test]
    fn a_final_unterminated_line_runs_to_the_end_of_the_resource() {
        let text = b"one\ntwo";
        assert_eq!(span(text, "L2").unwrap(), b"two");
        assert_eq!(span(text, "L1-2").unwrap(), text);
        assert_eq!(span(text, "L1-40").unwrap(), text);
    }

    #[test]
    fn a_bare_carriage_return_is_content_not_a_line_break() {
        let text = b"a\rb\rc";
        assert_eq!(span(text, "L1").unwrap(), text);
        assert!(span(text, "L2").is_err());
    }

    #[test]
    fn blank_lines_are_lines() {
        let text = b"\n\nx\n";
        assert_eq!(span(text, "L1").unwrap(), b"\n");
        assert_eq!(span(text, "L2").unwrap(), b"\n");
        assert_eq!(span(text, "L3").unwrap(), b"x\n");
        assert_eq!(span(text, "L2-3").unwrap(), b"\nx\n");
    }

    #[test]
    fn an_empty_resource_has_no_lines_to_address() {
        assert_eq!(
            span(b"", "L1"),
            Err(LineRangeError::StartPastEnd { start: 1, lines: 0 })
        );
    }

    #[test]
    fn the_end_clamps_to_the_last_line_and_the_start_does_not() {
        let text = b"alpha\nbeta\ngamma\n";
        assert_eq!(span(text, "L3-9999").unwrap(), b"gamma\n");
        assert_eq!(span(text, "L1-9999").unwrap(), text);
        assert_eq!(
            span(text, "L4-9999"),
            Err(LineRangeError::StartPastEnd { start: 4, lines: 3 })
        );
    }
}
