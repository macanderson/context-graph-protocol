"""The HTTP twin of ``example_docs.py``: the same honest two-frame documentation
provider, served over the "streamable HTTP" transport (``SPEC.md`` §3) instead of
stdio. It answers the whole CGP protocol on one POST endpoint, so the conformance
suite can drive it remotely::

    PORT=8788 python3 sdk/python/examples/example_docs_http.py &
    contextgraph-inspect http http://127.0.0.1:8788

The provider logic is identical to the stdio example -- only the transport
differs, which is the whole point of a framework-agnostic ``make_wsgi_app``:
write the provider once, host it however you like. The server here is the
stdlib's ``wsgiref.simple_server``, so the example stays zero-dependency.
"""

from __future__ import annotations

import os
import sys
from typing import Any
from wsgiref.simple_server import WSGIRequestHandler, make_server

# Allow running the example directly from the repo without installing the SDK.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from contextgraph_sdk import (  # noqa: E402
    ProviderError,
    budget_tokens,
    make_wsgi_app,
)

EMBEDDING_FINGERPRINT = "bge-small-en-v1.5/384/l2"
EMBEDDING_DIMENSIONS = int(EMBEDDING_FINGERPRINT.split("/")[1])

# Stable, syntactically valid sha256:<64 hex> digests (SPEC.md F5) -- the same
# values verify answers with, so served frames and verify verdicts never drift.
GETTING_STARTED_DIGEST = "sha256:" + ("11" * 32)
CONFIGURATION_DIGEST = "sha256:" + ("22" * 32)


def _current_digest(frame_id: str) -> str | None:
    return {
        "frm_getting_started": GETTING_STARTED_DIGEST,
        "frm_configuration": CONFIGURATION_DIGEST,
    }.get(frame_id)


def _is_anchored(frame: dict[str, Any], anchors: list[str]) -> bool:
    if frame.get("uri") in anchors:
        return True
    return any(rel.get("target_uri") in anchors for rel in frame.get("relations", []))


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
                "by": "contextgraph-py-example-docs-http",
            }
        ],
        "citation_label": f"{file} {rng}",
        "relations": [
            {
                "rel": "doc.documents",
                "target_uri": f"symbol:///docs/{file}#overview",
                "display_name": f"{title} overview",
            }
        ],
    }


class ExampleDocsHttpProvider:
    def info(self) -> dict[str, Any]:
        # The provider serves local canned frames; nothing leaves the machine of
        # its own accord, so it declares the honest local-only scope. The HTTP
        # transport is treated as egress by the host regardless (SPEC.md §4).
        return {
            "name": "contextgraph-py-example-docs-http",
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
            "graph": True,
            "embeddings_fingerprint": EMBEDDING_FINGERPRINT,
            "verify": True,
        }

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
        embedding = query.get("embedding")
        if embedding is not None and len(embedding) != EMBEDDING_DIMENSIONS:
            raise ProviderError(
                f"query embedding has {len(embedding)} dimensions; this provider "
                f"indexes {EMBEDDING_DIMENSIONS} ({EMBEDDING_FINGERPRINT}) (§E1)",
                code="bad_request",
            )
        frames = [
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
        ]
        anchors = query.get("anchors") or []
        if anchors:
            frames.sort(key=lambda f: not _is_anchored(f, anchors))
        return {"frames": frames, "truncated": False}

    def verify(self, request: dict[str, Any]) -> dict[str, Any]:
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


class _QuietHandler(WSGIRequestHandler):
    """Silence the per-request access log so the one stdout line stays the URL."""

    def log_message(self, *args: Any) -> None:  # noqa: D401
        pass


if __name__ == "__main__":
    port = int(os.environ.get("PORT", "8788"))
    host = os.environ.get("HOST", "127.0.0.1")
    app = make_wsgi_app(ExampleDocsHttpProvider())
    with make_server(host, port, app, handler_class=_QuietHandler) as server:
        # One line to stdout so a supervising script (or a human) knows the URL
        # to point `contextgraph-inspect http` at.
        print(f"contextgraph provider listening on http://{host}:{port}", flush=True)
        server.serve_forever()
