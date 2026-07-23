package contextgraph

// BytesPerBudgetToken is the accounting-token size (SPEC.md B3).
const BytesPerBudgetToken = 4

// BudgetTokens is the canonical budget-token cost of a piece of frame content:
// ceil(utf8_byte_length(content) / 4). A conforming provider's
// ContextFrame.TokenCost MUST equal this for the frame's inline content (a
// reference frame carries no content, so its required cost is 0).
//
// Go strings are UTF-8, so len(content) is already the byte length.
func BudgetTokens(content string) uint32 {
	n := len(content)
	if n == 0 {
		return 0
	}
	return uint32((n + BytesPerBudgetToken - 1) / BytesPerBudgetToken)
}
