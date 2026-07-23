/**
 * Canonical token accounting (`SPEC.md` §B3) — the rule that makes budget
 * honesty checkable. A budget token is an accounting unit, not a real
 * tokenizer: `budget_tokens(content) = ceil(utf8_byte_length(content) / 4)`.
 *
 * A conforming provider's `ContextFrame.token_cost` MUST equal this for the
 * frame's inline `content` (a `reference` frame carries no content, so its
 * required cost is 0).
 */

export const BYTES_PER_BUDGET_TOKEN = 4;

/** The canonical budget-token cost of a piece of frame content. */
export function budgetTokens(content: string | undefined): number {
  if (!content) return 0;
  const bytes = new TextEncoder().encode(content).length;
  return Math.ceil(bytes / BYTES_PER_BUDGET_TOKEN);
}
