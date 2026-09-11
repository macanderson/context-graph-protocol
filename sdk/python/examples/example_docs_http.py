"""The HTTP twin of ``example_docs.py``: the same honest two-frame provider --
one ``doc`` and one ``snippet``, with disjoint validity windows -- served over
the "streamable HTTP" transport (``SPEC.md`` §3) instead of stdio. It answers the whole CGP protocol on one POST endpoint, so the conformance
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
    kind: str,
    title: str,
    content: str,
    file: str,
    rng: str,
    score: float,
    digest: str,
    valid_from: str,
) -> dict[str, Any]:
    """One canned frame.

    ``kind`` and ``valid_from`` are parameters rather than constants because the
    two frames deliberately differ in both, mirroring the Rust reference
    fixture: a provider that declares ``["doc", "snippet"]`` and serves only
    ``doc`` frames makes §Q1 unobservable -- every kind-narrowed query it can be
    asked returns frames it would have returned anyway -- and two frames sharing
    one validity window makes an ``as_of`` pin unobservable the same way.
    """
    return {
        "id": frame_id,
        "kind": kind,
        "title": title,
        "content": content,
        "content_digest": digest,
        "uri": f"file:///docs/{file}",
        "score": score,
        # Honest cost: ceil(utf8_len(content)/4) (B3).
        "token_cost": budget_tokens(content),
        "valid_from": valid_from,
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
            # Valid since the start of the year -- before the conformance
            # suite's `as_of` pin, so a pinned query still reaches it.
            _doc_frame(
                "frm_getting_started",
                "doc",
                "Getting Started",
                "Install the reference binding, then implement the required provider methods.",
                "getting-started.md",
                "L1-40",
                0.82,
                GETTING_STARTED_DIGEST,
                "2026-01-01T00:00:00Z",
            ),
            # A `snippet`, and one that only became true in the autumn: the
            # second frame is what gives §Q1 and §F4 something to observe. It is
            # the frame a `kinds: ["doc"]` query must drop and a
            # `kinds: ["snippet"]` query must keep, and the one a mid-year
            # `as_of` pin must exclude.
            _doc_frame(
                "frm_configuration",
                "snippet",
                "Configuration example",
                'host = create_host().with_provider("docs", provider)',
                "configuration.md",
                "L1-25",
                0.61,
                CONFIGURATION_DIGEST,
                "2026-09-01T00:00:00Z",
            ),
        ]
        # §Q1: a non-empty `kinds` is a filter, not a hint. Returning a frame
        # outside it spends the host's budget on content it explicitly excluded.
        kinds = query.get("kinds") or []
        if kinds:
            frames = [frame for frame in frames if frame["kind"] in kinds]
        anchors = query.get("anchors") or []
        if anchors:
            frames.sort(key=lambda f: not _is_anchored(f, anchors))
        # §F4/§6.1: honour an `as_of` pin -- content that was not yet true at the
        # pinned instant is not returned. The timestamp profile admits one
        # spelling per instant, so a lexicographic compare on the UTC strings is
        # a chronological one.
        as_of = query.get("as_of")
        if as_of is not None:
            frames = [
                frame for frame in frames if (frame.get("valid_from") or "") <= as_of
            ]
        # `truncated` stays False: these filters honour the host's own narrowing,
        # they are not this provider running out of budget (§B2).
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
