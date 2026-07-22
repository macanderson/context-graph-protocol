//! Canonical token accounting — the rule that makes budget honesty checkable
//! (`SPEC.md` §B3, [ADR 0003](../../docs/adr/0003-canonical-token-accounting.md)).
//!
//! Budget honesty is CGP's flagship guarantee, but before this rule existed the
//! conformance suite could only verify *arithmetic*: it summed the costs a
//! provider declared and compared the total to the budget. A provider reporting
//! `token_cost: 1` on a ten-thousand-token frame satisfied that check perfectly
//! while destroying the host's actual budget. The one lie that mattered was the
//! one lie the suite could not catch.
//!
//! [`budget_tokens`] closes that hole by making cost a function of bytes both
//! sides observe:
//!
//! ```text
//! budget_tokens(content) = ceil(utf8_byte_length(content) / 4)
//! ```
//!
//! # This is an accounting unit, not a tokenizer
//!
//! A budget token is deliberately **not** a prediction of any model's
//! tokenizer. Its job is to make every provider's cost claims comparable and
//! verifiable, which no real tokenizer can do without being mandated in every
//! language an implementation might be written in.
//!
//! The approximation is honest about its direction. At roughly four bytes per
//! token it tracks English prose closely, and it **under-estimates** dense
//! source code (≈3–3.5 bytes/token) and CJK text (≈3 bytes/token). A host
//! therefore **MUST NOT** treat one budget token as one model token: it maps
//! its real model budget into budget tokens with a safety factor. See
//! [`SUGGESTED_HOST_SAFETY_FACTOR`].
//!
//! # Scope
//!
//! The count covers `ContextFrame::content` only — not `title`, not
//! `citation_label`, not provenance, and not the fences and labels a host wraps
//! around a frame. `content` is the one field the provider fully controls and
//! whose exact bytes both sides observe identically, so it is the only input on
//! which a byte-exact check can be built. The host's own rendering chrome is
//! the host's cost to budget.

/// Bytes per budget token. See the module docs for why this constant is an
/// accounting convention rather than an empirical tokenizer ratio.
pub const BYTES_PER_BUDGET_TOKEN: usize = 4;

/// The factor a host is advised to apply when converting a real model context
/// budget into budget tokens, compensating for the under-estimate on source
/// code and CJK text.
///
/// Advisory, not normative: a host that knows its corpus is English prose can
/// safely use less headroom, and one serving minified JSON may want more. It is
/// stated as a constant so the reference host's choice is inspectable rather
/// than buried in a literal.
pub const SUGGESTED_HOST_SAFETY_FACTOR: f32 = 1.35;

/// The canonical budget-token cost of a piece of frame content.
///
/// This is the value `ContextFrame::token_cost` **MUST** carry (`SPEC.md` §B3).
/// Exact equality is required — there is no tolerance band, because any band
/// wide enough to absorb genuine tokenizer disagreement is also wide enough to
/// hide meaningful under-reporting, which puts the suite back to guessing. A
/// provider cannot "disagree" with a byte count.
///
/// ```
/// use contextgraph_types::budget_tokens;
///
/// assert_eq!(budget_tokens(""), 0);
/// assert_eq!(budget_tokens("abcd"), 1);
/// assert_eq!(budget_tokens("abcde"), 2); // ceil, never floor
/// ```
pub fn budget_tokens(content: &str) -> u32 {
    // `str::len` is the UTF-8 byte length, which is exactly what the rule
    // specifies — not `chars().count()`, which would make the count depend on
    // Unicode normalization and diverge across implementations.
    let bytes = content.len();
    bytes.div_ceil(BYTES_PER_BUDGET_TOKEN) as u32
}

/// Convert a real model context budget into budget tokens, applying `factor` as
/// headroom against the under-estimate documented on [`budget_tokens`].
///
/// A host asking for `model_tokens` worth of real context should request this
/// many budget tokens, so that honest providers filling the budget exactly do
/// not overflow the model window.
pub fn budget_from_model_tokens(model_tokens: u32, factor: f32) -> u32 {
    if factor <= 0.0 {
        return model_tokens;
    }
    (model_tokens as f32 / factor) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_costs_nothing() {
        assert_eq!(budget_tokens(""), 0);
    }

    #[test]
    fn the_count_rounds_up_so_a_partial_token_is_never_free() {
        // The rounding direction is the whole point: floor would let a
        // provider shave a token off every frame and call it arithmetic.
        assert_eq!(budget_tokens("a"), 1);
        assert_eq!(budget_tokens("abc"), 1);
        assert_eq!(budget_tokens("abcd"), 1);
        assert_eq!(budget_tokens("abcde"), 2);
        assert_eq!(budget_tokens("abcdefgh"), 2);
    }

    #[test]
    fn the_count_is_over_utf8_bytes_not_characters() {
        // A single emoji is 4 UTF-8 bytes but 1 char. Counting characters
        // would let a provider serve 4x the bytes it declared, and would make
        // the count depend on Unicode normalization — so the rule names bytes
        // explicitly and this test pins it.
        let emoji = "😀";
        assert_eq!(emoji.chars().count(), 1);
        assert_eq!(emoji.len(), 4);
        assert_eq!(budget_tokens(emoji), 1);

        // Three-byte CJK: 3 chars, 9 bytes, 3 budget tokens (not 1).
        let cjk = "文脈図";
        assert_eq!(cjk.chars().count(), 3);
        assert_eq!(cjk.len(), 9);
        assert_eq!(budget_tokens(cjk), 3);
    }

    #[test]
    fn the_rule_is_reproducible_from_bytes_alone() {
        // The property that makes conformance possible in any language: the
        // count depends on nothing but the bytes.
        let content = "fn main() { println!(\"hello\"); }";
        assert_eq!(budget_tokens(content), content.len().div_ceil(4) as u32);
    }

    #[test]
    fn host_safety_factor_shrinks_the_requested_budget() {
        // 10_000 real model tokens with 1.35x headroom means asking providers
        // for ~7_407 budget tokens, so an honest fill does not overflow.
        let budget = budget_from_model_tokens(10_000, SUGGESTED_HOST_SAFETY_FACTOR);
        assert!(budget < 10_000);
        assert_eq!(budget, 7407);
    }

    #[test]
    fn a_nonsense_safety_factor_degrades_to_identity_rather_than_panicking() {
        assert_eq!(budget_from_model_tokens(500, 0.0), 500);
        assert_eq!(budget_from_model_tokens(500, -1.0), 500);
    }
}
