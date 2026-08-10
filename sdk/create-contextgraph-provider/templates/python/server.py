"""The HTTP entrypoint: host the same PROVIDER behind one POST endpoint (the
"streamable HTTP" transport, SPEC.md §3). Point a host -- or
``contextgraph-inspect http http://127.0.0.1:8788`` -- at it.

``make_wsgi_app`` reads the raw request body itself, so this stdlib
``wsgiref.simple_server`` needs no framework and no body parser. Under Flask, set
``app.wsgi_app = make_wsgi_app(PROVIDER)``; under FastAPI (ASGI), call
``respond_to_body`` with the request body inside your route.
"""

from __future__ import annotations

import os
from typing import Any
from wsgiref.simple_server import WSGIRequestHandler, make_server

from contextgraph_sdk import make_wsgi_app

from provider import PROVIDER


class _QuietHandler(WSGIRequestHandler):
    def log_message(self, *args: Any) -> None:
        pass


if __name__ == "__main__":
    port = int(os.environ.get("PORT", "8788"))
    host = os.environ.get("HOST", "127.0.0.1")
    app = make_wsgi_app(PROVIDER)
    with make_server(host, port, app, handler_class=_QuietHandler) as server:
        print(f"{{PROJECT_NAME}} listening on http://{host}:{port}", flush=True)
        server.serve_forever()
