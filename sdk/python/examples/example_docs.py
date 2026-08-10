"""A tiny reference Context Graph Protocol provider, in Python — the mirror of
the Rust ``contextgraph-example-docs`` and the TypeScript example. It serves two
canned documentation frames honestly, and is the fixture the language-neutral
conformance suite drives to prove a third independent implementation passes::

    contextgraph-inspect stdio --json -- python3 sdk/python/examples/example_docs.py
"""

from __future__ import annotations

import hashlib
import os
import sys
from typing import Any

# Allow running the example directly from the repo without installing the SDK.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from contextgraph_sdk import (  # noqa: E402
    ProviderError,
    budget_tokens,
    run_stdio_provider,
)

# The embedding space this fixture declares it indexes (SPEC.md §E1). Its
# dimension -- the 2nd `/`-separated segment (384) -- is the length a query
# embedding must match; a contradicting length is a vector from a different
# space, rejected `bad_request` rather than scored into meaningless similarity.
EMBEDDING_FINGERPRINT = "bge-small-en-v1.5/384/l2"
EMBEDDING_DIMENSIONS = int(EMBEDDING_FINGERPRINT.split("/")[1])

# The directory holding this provider's on-disk backing files, resolved
# relative to this file so a digest is computed over the same bytes no matter
# where the provider is spawned from (SPEC.md §6.2).
FIXTURE_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def _fixture_uri(file: str) -> str:
    """The absolute ``file://`` URI a host re-reads to verify a frame's
    provenance digest (``provenance-fixture-consistency``). Absolute and
    cwd-independent, so verification never depends on the host's working
    directory."""
    return "file://" + os.path.join(FIXTURE_DIR, file)


def _fixture_digest(file: str) -> str:
    """The real ``sha256:<64 lowercase hex>`` digest over a backing file's exact
    on-disk bytes -- byte-for-byte what a host recomputes when it re-reads the
    file, so an unmutated frame verifies end to end (SPEC.md §6.2, §F5)."""
    try:
        with open(os.path.join(FIXTURE_DIR, file), "rb") as handle:
            data = handle.read()
    except OSError:
        data = b""
    return "sha256:" + hashlib.sha256(data).hexdigest()


GETTING_STARTED_DIGEST = _fixture_digest("getting-started.md")
CONFIGURATION_DIGEST = _fixture_digest("configuration.md")


def _current_digest(frame_id: str) -> str | None:
    return {
        "frm_getting_started": GETTING_STARTED_DIGEST,
        "frm_configuration": CONFIGURATION_DIGEST,
    }.get(frame_id)


def _is_anchored(frame: dict[str, Any], anchors: list[str]) -> bool:
    """Whether ``frame`` is anchored by any of ``anchors`` (SPEC.md §G4).

    Zero hops via the frame's own ``uri``, one hop via a relation's
    ``target_uri``. String equality, so the predicate is decidable and two
    implementations cannot disagree about what "anchored" means.
    """
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
        "uri": _fixture_uri(file),
        "score": score,
        # Honest cost: ceil(utf8_len(content)/4) (B3).
        "token_cost": budget_tokens(content),
        "valid_from": "2026-01-01T00:00:00Z",
        "recorded_at": "2026-07-20T18:00:00Z",
        "provenance": [
            {
                "type": "file",
                "uri": _fixture_uri(file),
                "range": rng,
                "digest": digest,
                "by": "contextgraph-py-example-docs",
            }
        ],
        "citation_label": f"{file} {rng}",
        # A labelled edge to the symbol this page documents. §G4 makes a frame
        # "anchored" when its own `uri` or any relation's `target_uri` equals a
        # query anchor, so this edge is what an anchored query reaches at one hop.
        "relations": [
            {
                "rel": "doc.documents",
                "target_uri": f"symbol:///docs/{file}#overview",
                "display_name": f"{title} overview",
            }
        ],
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
            "graph": True,
            # Declaring the embedding space it indexes lets the provider reject a
            # vector from a different one (§E1). A provider that declares no
            # fingerprint has nothing to contradict and is not E1-probed.
            "embeddings_fingerprint": EMBEDDING_FINGERPRINT,
            "verify": True,
        }

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
        # §E1: a query embedding whose length contradicts this provider's
        # declared fingerprint dimension names a different vector space; scoring
        # it would yield plausible-looking, meaningless similarity. An honest
        # provider rejects it `bad_request` rather than pretending.
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
        # §G4: a frame is anchored when its own `uri`, or any relation's
        # `target_uri`, equals one of the query's anchors. A graph-declaring
        # provider ranks anchored frames first -- the "boost" §G3 asks for.
        anchors = query.get("anchors") or []
        if anchors:
            frames.sort(key=lambda f: not _is_anchored(f, anchors))
        return {"frames": frames, "truncated": False}

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
