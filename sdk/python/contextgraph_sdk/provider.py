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
import traceback
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

    An exception that is *not* a :class:`ProviderError` is an unexpected crash
    rather than a refusal, so it gets no code of its own: the runtime answers
    ``internal`` and reports the traceback to stderr. Either way the host gets
    exactly one reply per request -- silence is never a valid answer
    (``SPEC.md`` §11 R1).
    """

    def __init__(self, message: str, code: str | None = None) -> None:
        super().__init__(message)
        self.message = message
        self.code = code


def _write(envelope: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(envelope, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def bad_request(message: str, echoed: Any = None) -> dict[str, Any]:
    """The reply to a request the host got wrong.

    A line that parses to something other than an object, or a well-formed
    envelope with its payload missing, is the host's mistake -- not a provider
    fault. ``SPEC.md`` §11 R1 says a provider MUST NOT crash on one and SHOULD
    reply ``error`` with code ``bad_request``.
    """
    reply: dict[str, Any] = {"type": "error", "code": "bad_request", "message": message}
    if echoed is not None:
        reply["id"] = echoed
    return reply


def error_envelope_for(
    error: BaseException, context: str, echoed: Any = None
) -> dict[str, Any]:
    """Turn a caught exception into the reply envelope it deserves.

    A :class:`ProviderError` is a deliberate, coded refusal (§E1) and keeps its
    own code. Anything else is an unexpected crash: it becomes ``internal``, and
    the real traceback goes to stderr because the operator still needs to see it
    even though the host only gets a code.
    """
    if isinstance(error, ProviderError):
        reply: dict[str, Any] = {"type": "error", "message": error.message}
        if error.code is not None:
            reply["code"] = error.code
    else:
        print(f"contextgraph: unexpected error in {context}", file=sys.stderr)
        traceback.print_exception(type(error), error, error.__traceback__, file=sys.stderr)
        sys.stderr.flush()
        reply = {
            "type": "error",
            "code": "internal",
            "message": str(error) or type(error).__name__,
        }
    if echoed is not None:
        reply["id"] = echoed
    return reply


def payload_of(envelope: dict[str, Any], key: str) -> dict[str, Any] | None:
    """The ``query``/``request`` object an envelope carries, or ``None``.

    A bare ``envelope["query"]`` raises ``KeyError`` on a well-formed envelope
    that simply omits its payload, which used to kill the process mid-session
    and strand every correlation id already in flight.
    """
    value = envelope.get(key)
    return value if isinstance(value, dict) else None


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

        # `json.loads` succeeds for 42, "x", [], null and true. None of them has
        # a `.get`, so reading `type` off one used to raise AttributeError and
        # kill the process. They are bad requests, not envelopes.
        if not isinstance(envelope, dict):
            _write(bad_request("line was not a CGP envelope object"))
            continue

        kind = envelope.get("type")
        if kind == "handshake":
            # `info()` and `capabilities()` are provider code and can raise; an
            # unanswered handshake strands the host before the session starts.
            try:
                reply: dict[str, Any] = {
                    "type": "handshake_ack",
                    "protocol_version": PROTOCOL_VERSION,
                    "provider": provider.info(),
                    "capabilities": provider.capabilities(),
                }
            except Exception as error:  # noqa: BLE001 - every fault becomes a reply
                reply = error_envelope_for(error, "provider.info()/capabilities()")
            _write(reply)
        elif kind == "query":
            # Echo the correlation id so the host can match reply to request (H4).
            echoed = envelope.get("id")
            payload = payload_of(envelope, "query")
            if payload is None:
                _write(bad_request("query envelope is missing its `query` payload", echoed))
                continue
            try:
                result = provider.query(payload)
            except Exception as error:  # noqa: BLE001 - every fault becomes a reply
                # A ProviderError keeps its own code (§E1); anything else is
                # answered `internal` and reported to stderr. Letting it
                # propagate used to kill the process, losing every query
                # already in flight along with it.
                reply = error_envelope_for(error, "provider.query()", echoed)
            else:
                reply = {"type": "frames", "result": result}
                if echoed is not None:
                    reply["id"] = echoed
            _write(reply)
        elif kind == "verify":
            # `verify` carries no `id` in the wire schema today, but echoing one
            # a host did send costs nothing and keeps its slot correlatable.
            echoed = envelope.get("id")
            request = payload_of(envelope, "request")
            if request is None or not isinstance(request.get("frames"), list):
                _write(
                    bad_request(
                        "verify envelope is missing its `request.frames` payload", echoed
                    )
                )
                continue
            try:
                verify = getattr(provider, "verify", None)
                if callable(verify):
                    response = verify(request)
                else:
                    # No verify support: vouch for nothing; the host re-queries.
                    response = {
                        "verdicts": [
                            {"frame": frame, "status": "unknown"}
                            for frame in request["frames"]
                        ]
                    }
            except Exception as error:  # noqa: BLE001 - every fault becomes a reply
                # Identical stranded-host outcome to a failing query, so the
                # identical answer.
                _write(error_envelope_for(error, "provider.verify()", echoed))
            else:
                _write({"type": "verified", "response": response})
        elif kind == "shutdown":
            sys.exit(0)
        # handshake_ack / frames / verified / error are host->provider-invalid; ignore.
