"""A tiny reference Context Graph Protocol provider, in Python — the mirror of
the Rust ``contextgraph-example-docs`` and the TypeScript example. It serves two
canned documentation frames honestly, and is the fixture the language-neutral
conformance suite drives to prove a third independent implementation passes::

    contextgraph-inspect stdio --json -- python3 sdk/python/examples/example_docs.py
"""

from __future__ import annotations

import os
import sys
from typing import Any

# Allow running the example directly from the repo without installing the SDK.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from contextgraph_sdk import budget_tokens, run_stdio_provider  # noqa: E402

# Stable, syntactically valid sha256:<64 hex> digests (SPEC.md F5). Not real
# hashes of anything -- this fixture serves string literals, not on-disk bytes --
# but well-formed, and the same value verify answers with, so served frames and
# verify verdicts can never drift apart.
GETTING_STARTED_DIGEST = "sha256:" + ("11" * 32)
CONFIGURATION_DIGEST = "sha256:" + ("22" * 32)


def _current_digest(frame_id: str) -> str | None:
    return {
        "frm_getting_started": GETTING_STARTED_DIGEST,
        "frm_configuration": CONFIGURATION_DIGEST,
    }.get(frame_id)


def _doc_frame(
    frame_id: str,
    title: str,
    content: str,
    file: str,
    rng: str,
    score: float,
    digest: str,
) -> dict[str, Any]:
    return {
        "id": frame_id,
        "kind": "doc",
        "title": title,
        "content": content,
        "content_digest": digest,
        "uri": f"file:///docs/{file}",
        "score": score,
        # Honest cost: ceil(utf8_len(content)/4) (B3).
        "token_cost": budget_tokens(content),
        "valid_from": "2026-01-01T00:00:00Z",
        "recorded_at": "2026-07-20T18:00:00Z",
        "provenance": [
            {
                "type": "file",
                "uri": f"file:///docs/{file}",
                "range": rng,
                "digest": digest,
                "by": "contextgraph-py-example-docs",
            }
        ],
        "citation_label": f"{file} {rng}",
        "relations": [],
    }


class ExampleDocsProvider:
    def info(self) -> dict[str, Any]:
        # A docs index reads the query and serves local frames; nothing leaves
        # the machine, so it honestly declares the local-only egress scope.
        return {
            "name": "contextgraph-py-example-docs",
            "version": "0.1.0",
            "data_flow": {
                "reads": True,
                "writes": False,
                "egress": False,
                "egress_scopes": ["local-only"],
            },
        }

    def capabilities(self) -> dict[str, Any]:
        return {
            "query": {"kinds": ["doc", "snippet"]},
            "correlation": True,
            "graph": False,
            "verify": True,
        }

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
        return {
            "frames": [
                _doc_frame(
                    "frm_getting_started",
                    "Getting Started",
                    "Install the reference binding, then implement the required provider methods.",
                    "getting-started.md",
                    "L1-40",
                    0.82,
                    GETTING_STARTED_DIGEST,
                ),
                _doc_frame(
                    "frm_configuration",
                    "Configuration",
                    "Providers declare their data-flow direction at the handshake so hosts can gate consent before sending any query.",
                    "configuration.md",
                    "L1-25",
                    0.61,
                    CONFIGURATION_DIGEST,
                ),
            ],
            "truncated": False,
        }

    def verify(self, request: dict[str, Any]) -> dict[str, Any]:
        # Honest verify: compare each presented digest against what is currently
        # served. A differing digest is exactly what a mutated source looks like.
        verdicts = []
        for frame in request["frames"]:
            current = _current_digest(frame.get("frame_id", ""))
            presented = frame.get("content_digest")
            if current is None:
                verdict = {"frame": frame, "status": "gone"}
            elif not presented:
                verdict = {"frame": frame, "status": "unknown"}
            elif presented == current:
                verdict = {"frame": frame, "status": "valid"}
            else:
                verdict = {"frame": frame, "status": "stale", "replacement_digest": current}
            verdicts.append(verdict)
        return {"verdicts": verdicts}


if __name__ == "__main__":
    run_stdio_provider(ExampleDocsProvider())
