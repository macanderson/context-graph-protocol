"""``contextgraph-sdk`` — a zero-dependency Python SDK for building conformant
Context Graph Protocol providers.

    from contextgraph_sdk import run_stdio_provider, budget_tokens

    class MyProvider:
        def info(self):
            return {"name": "my-provider", "version": "0.1.0",
                    "data_flow": {"reads": True, "writes": False, "egress": False,
                                  "egress_scopes": ["local-only"]}}
        def capabilities(self):
            return {"query": {"kinds": ["doc"]}, "correlation": True}
        def query(self, query):
            return {"frames": [], "truncated": False}

    run_stdio_provider(MyProvider())
"""

from .budget import BYTES_PER_BUDGET_TOKEN, budget_tokens
from .provider import Provider, run_stdio_provider
from .types import PROTOCOL_VERSION

__all__ = [
    "PROTOCOL_VERSION",
    "BYTES_PER_BUDGET_TOKEN",
    "budget_tokens",
    "Provider",
    "run_stdio_provider",
]
