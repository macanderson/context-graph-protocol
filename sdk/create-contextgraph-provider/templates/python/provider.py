"""{{PROJECT_NAME}} -- a Context Graph Protocol provider.

This is the one place your provider's behavior lives. Both transports import it:
running this file directly is the stdio provider (a child process the host
spawns), and ``server.py`` hosts the same ``PROVIDER`` over HTTP. Edit ``query``
to serve your real frames -- the scaffold ships an honest two-frame docs example
that passes the conformance suite out of the box, so you always start from green.
"""

from __future__ import annotations

from typing import Any

from contextgraph_sdk import ProviderError, budget_tokens, run_stdio_provider

# Stable, syntactically valid sha256:<64 hex> digests (SPEC.md F5). Replace with
# real content hashes once you serve real bytes -- verify compares the digest a
# host presents against the one you currently serve.
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
        # Honest cost: ceil(utf8_len(content)/4) (B3). Always compute it -- never
        # guess -- or the budget-honesty conformance check fails.
        "token_cost": budget_tokens(content),
        "valid_from": "2026-01-01T00:00:00Z",
        "recorded_at": "2026-07-20T18:00:00Z",
        "provenance": [
            {
                "type": "file",
                "uri": f"file:///docs/{file}",
                "range": rng,
                "digest": digest,
                "by": "{{PROJECT_NAME}}",
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


class Provider:
    def info(self) -> dict[str, Any]:
        # Declare your data-flow honestly. `egress: false` here because this
        # example only serves local canned frames; set it true if your provider
        # reaches any remote service, or the host cannot gate consent correctly.
        return {
            "name": "{{PROJECT_NAME}}",
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
            "query": {"kinds": ["doc"]},
            "correlation": True,
            "graph": True,
            "verify": True,
        }

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
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
        # §G4: rank anchored frames first when the query carries anchors.
        anchors = query.get("anchors") or []
        if anchors:
            frames.sort(key=lambda f: not _is_anchored(f, anchors))
        # If you have more relevant material than fits `query["max_tokens"]`,
        # return your best frames within budget and set truncated=True instead.
        # Raise ProviderError("...", code="bad_request") to refuse a bad request.
        _ = ProviderError  # exported for you to throw a coded refusal (e.g. §E1).
        return {"frames": frames, "truncated": False}

    def verify(self, request: dict[str, Any]) -> dict[str, Any]:
        # Compare each presented digest against what you currently serve. Never
        # send frame bodies back -- a `verified` reply carries identities only.
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


PROVIDER = Provider()


if __name__ == "__main__":
    run_stdio_provider(PROVIDER)
