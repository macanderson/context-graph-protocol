"""The provider runtime.

Implement :class:`Provider` (info / capabilities / query, and optionally verify),
hand it to :func:`run_stdio_provider`, and you have a conformant Context Graph
Protocol provider speaking the line-oriented JSON wire over stdio. The runtime
drives the full lifecycle — handshake, query (echoing the correlation ``id``),
verify, shutdown — and stays alive with a typed error on a malformed line rather
than crashing (the ``malformed-input-tolerance`` guarantee).
"""

from __future__ import annotations

import json
import sys
from typing import Any, Protocol, runtime_checkable

from .types import PROTOCOL_VERSION


@runtime_checkable
class Provider(Protocol):
    """A Context Graph Protocol provider.

    ``query`` is mandatory. ``verify`` is optional — a provider that omits it is
    treated as unable to vouch for its frames, and the host re-queries.
    """

    def info(self) -> dict[str, Any]:
        """Identity and data-flow posture, reported at handshake."""
        ...

    def capabilities(self) -> dict[str, Any]:
        """What this provider can do, negotiated at handshake."""
        ...

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
        """Answer a retrieval request with budgeted, provenance-carrying frames."""
        ...

    # def verify(self, request: dict[str, Any]) -> dict[str, Any]: ...  # optional


class ProviderError(Exception):
    """A protocol-level error a provider raises from ``query`` to reply with an
    ``error`` envelope carrying a machine-readable ``code`` instead of frames.

    This is how a provider refuses a request it cannot honestly serve — e.g.
    rejecting a query embedding whose length contradicts its declared
    ``embeddings_fingerprint`` dimension with ``bad_request`` (``SPEC.md`` §E1).
    The runtime catches it, echoes the request's correlation ``id``, and writes
    ``{"type": "error", "code": code, "message": message}``.
    """

    def __init__(self, message: str, code: str | None = None) -> None:
        super().__init__(message)
        self.message = message
        self.code = code


def _write(envelope: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(envelope, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def run_stdio_provider(provider: Provider) -> None:
    """Run ``provider`` as a stdio child process — the shape the reference host
    and the conformance suite drive. One envelope per line in and out, until a
    ``shutdown`` (or EOF)."""
    while True:
        line = sys.stdin.readline()
        if not line:  # EOF / broken pipe: the host is gone.
            break
        stripped = line.strip()
        if not stripped:
            continue

        try:
            envelope = json.loads(stripped)
        except json.JSONDecodeError:
            # Malformed line: stay alive and say so with a code, don't crash.
            _write(
                {
                    "type": "error",
                    "code": "bad_request",
                    "message": "line was not a valid CGP envelope",
                }
            )
            continue

        kind = envelope.get("type")
        if kind == "handshake":
            _write(
                {
                    "type": "handshake_ack",
                    "protocol_version": PROTOCOL_VERSION,
                    "provider": provider.info(),
                    "capabilities": provider.capabilities(),
                }
            )
        elif kind == "query":
            # Echo the correlation id so the host can match reply to request (H4).
            echoed = envelope.get("id")
            try:
                result = provider.query(envelope["query"])
            except ProviderError as error:
                # The provider refused a request it can't honestly serve (§E1):
                # reply with a coded error envelope, not frames.
                reply: dict[str, Any] = {"type": "error", "message": error.message}
                if error.code is not None:
                    reply["code"] = error.code
            else:
                reply = {"type": "frames", "result": result}
            if echoed is not None:
                reply["id"] = echoed
            _write(reply)
        elif kind == "verify":
            verify = getattr(provider, "verify", None)
            if callable(verify):
                response = verify(envelope["request"])
            else:
                # No verify support: vouch for nothing; the host re-queries.
                response = {
                    "verdicts": [
                        {"frame": frame, "status": "unknown"}
                        for frame in envelope["request"]["frames"]
                    ]
                }
            _write({"type": "verified", "response": response})
        elif kind == "shutdown":
            sys.exit(0)
        # handshake_ack / frames / verified / error are host->provider-invalid; ignore.
