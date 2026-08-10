"""The HTTP adapter: host a provider behind a single POST endpoint, speaking the
same Context Graph Protocol wire as :func:`run_stdio_provider` -- the "streamable
HTTP" transport (``SPEC.md`` §3). The host POSTs one envelope as the request body
and expects one envelope back as the response body; :func:`handle_envelope` is
that request/response state machine, framework-agnostic so it drops under Flask,
FastAPI, or any WSGI server.

The one deliberate difference from stdio: an HTTP provider is a long-lived server
reached by many independent hosts, so a ``shutdown`` envelope ends *that
exchange* -- it never calls :func:`sys.exit`. (``contextgraph-inspect http`` in
fact handshakes and shuts down twice per run: once to probe, once to run the
conformance suite. A server that exited on the first shutdown could not answer
the second handshake.)
"""

from __future__ import annotations

import json
from typing import Any, Callable, Iterable

from .provider import Provider, ProviderError
from .types import PROTOCOL_VERSION


def handle_envelope(provider: Provider, envelope: dict[str, Any]) -> dict[str, Any] | None:
    """Drive one request ``envelope`` through ``provider`` and return the one
    response envelope -- or ``None`` for a ``shutdown`` (and for any
    host->provider envelope a provider must ignore), which has no reply body.

    This is the whole protocol state machine, transport-free: hand it a decoded
    envelope from whatever web framework you use and serialize what it returns.
    It mirrors :func:`run_stdio_provider`'s per-line handling exactly --
    including echoing a ``query``'s correlation ``id`` (H4) and catching a
    :class:`ProviderError` into a coded ``error`` envelope (§E1) -- minus the
    process lifecycle.
    """
    kind = envelope.get("type")

    if kind == "handshake":
        return {
            "type": "handshake_ack",
            "protocol_version": PROTOCOL_VERSION,
            "provider": provider.info(),
            "capabilities": provider.capabilities(),
        }

    if kind == "query":
        echoed = envelope.get("id")
        try:
            result = provider.query(envelope["query"])
        except ProviderError as error:
            # A deliberate, coded refusal of a request the provider can't
            # honestly serve (§E1): an error envelope, not frames.
            reply: dict[str, Any] = {"type": "error", "message": error.message}
            if error.code is not None:
                reply["code"] = error.code
        else:
            reply = {"type": "frames", "result": result}
        # Echo the correlation id so the host can match reply to request (H4).
        if echoed is not None:
            reply["id"] = echoed
        return reply

    if kind == "verify":
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
        return {"type": "verified", "response": response}

    # `shutdown` ends the exchange but keeps the server alive for the next host;
    # handshake_ack / frames / verified / error are host->provider-invalid. Both
    # have no reply body.
    return None


def respond_to_body(provider: Provider, raw_body: str | bytes) -> tuple[int, str]:
    """Decode one raw request body, drive it through :func:`handle_envelope`, and
    return ``(status, body)`` to send back. Use this when your framework hands
    you the raw body (a FastAPI/Flask route): respond with the status and the
    JSON string.

    A body that is not a valid CGP envelope is answered ``400`` with a coded
    ``error`` envelope rather than crashing -- the HTTP mirror of the stdio
    ``malformed-input-tolerance`` guarantee.
    """
    if isinstance(raw_body, (bytes, bytearray)):
        raw_body = raw_body.decode("utf-8", "replace")
    try:
        envelope = json.loads(raw_body)
    except (json.JSONDecodeError, ValueError):
        return 400, json.dumps(
            {
                "type": "error",
                "code": "bad_request",
                "message": "request body was not a valid CGP envelope",
            },
            separators=(",", ":"),
        )
    if not isinstance(envelope, dict):
        return 400, json.dumps(
            {
                "type": "error",
                "code": "bad_request",
                "message": "request body was not a CGP envelope object",
            },
            separators=(",", ":"),
        )
    reply = handle_envelope(provider, envelope)
    # `shutdown` (and ignored inputs) has no reply body: 204 No Content.
    if reply is None:
        return 204, ""
    return 200, json.dumps(reply, separators=(",", ":"))


def make_wsgi_app(
    provider: Provider,
) -> Callable[[dict[str, Any], Callable[..., Any]], Iterable[bytes]]:
    """Build a WSGI application that answers the CGP protocol on one endpoint --
    the zero-config path, dependency-free and runnable under the stdlib's
    ``wsgiref.simple_server`` or any production WSGI server (gunicorn, uWSGI):

    ::

        from wsgiref.simple_server import make_server
        make_server("127.0.0.1", 8788, make_wsgi_app(provider)).serve_forever()

    It also mounts under Flask (``app.wsgi_app = make_wsgi_app(provider)``) and,
    since it reads the raw ``wsgi.input`` itself, needs no body parser. Under an
    ASGI framework like FastAPI, call :func:`respond_to_body` with the request
    body inside your route instead.
    """

    def app(
        environ: dict[str, Any],
        start_response: Callable[..., Any],
    ) -> Iterable[bytes]:
        try:
            length = int(environ.get("CONTENT_LENGTH") or 0)
        except (TypeError, ValueError):
            length = 0
        raw_body = environ["wsgi.input"].read(length) if length > 0 else b""
        status_code, body = respond_to_body(provider, raw_body)
        payload = body.encode("utf-8")
        reason = {200: "OK", 204: "No Content", 400: "Bad Request"}.get(status_code, "OK")
        start_response(
            f"{status_code} {reason}",
            [
                ("Content-Type", "application/json"),
                ("Content-Length", str(len(payload))),
            ],
        )
        return [payload]

    return app
