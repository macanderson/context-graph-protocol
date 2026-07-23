"""Canonical token accounting (SPEC.md B3).

A budget token is an accounting unit, not a real tokenizer:
``budget_tokens(content) = ceil(utf8_byte_length(content) / 4)``. A conforming
provider's ``ContextFrame.token_cost`` MUST equal this for the frame's inline
content (a ``reference`` frame carries no content, so its required cost is 0).
"""

from __future__ import annotations

import math

BYTES_PER_BUDGET_TOKEN = 4


def budget_tokens(content: str | None) -> int:
    """The canonical budget-token cost of a piece of frame content."""
    if not content:
        return 0
    return math.ceil(len(content.encode("utf-8")) / BYTES_PER_BUDGET_TOKEN)
