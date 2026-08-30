# contextgraph-sdk — Python

A zero-dependency (stdlib-only) Python SDK for building **conformant** Context
Graph Protocol providers. Implement one small interface, hand it to the runtime,
and you have a provider that speaks the line-oriented JSON wire over stdio and
passes the same conformance suite that judges the Rust reference provider.

> Third independent implementation (after Rust and TypeScript); passes the full
> conformance suite. See [`sdk/README.md`](../README.md) for the whole picture.

## Install

```sh
pip install contextgraph-sdk
```

## Write a provider

```python
from contextgraph_sdk import run_stdio_provider, budget_tokens


class MyDocsProvider:
    def info(self):
        # Nothing leaves the machine -> declare the honest local-only egress scope.
        return {
            "name": "my-docs-provider",
            "version": "0.1.0",
            "data_flow": {"reads": True, "writes": False, "egress": False,
                          "egress_scopes": ["local-only"]},
        }

    def capabilities(self):
        return {"query": {"kinds": ["doc"]}, "correlation": True, "verify": True}

    def query(self, query):
        content = "Install the binding, then implement the required methods."
        return {
            "frames": [{
                "id": "doc:1", "kind": "doc", "title": "Getting started",
                "content": content,
                "content_digest": "sha256:" + ("11" * 32),
                "score": 0.9,
                # token_cost MUST equal ceil(utf8_len(content)/4).
                "token_cost": budget_tokens(content),
                "valid_from": "2026-01-01T00:00:00Z",
                "provenance": [{"type": "file", "uri": "file:///docs/start.md",
                                "range": "L1-10", "digest": "sha256:" + ("11" * 32)}],
                "citation_label": "start.md L1-10", "relations": [],
            }],
            "truncated": False,
        }


run_stdio_provider(MyDocsProvider())
```

`verify` is optional. The runtime handles the whole lifecycle — handshake, query
(echoing the correlation `id`), verify, shutdown — and stays alive with a typed
error on a malformed line rather than crashing.

## Host it over HTTP

The same provider runs behind a single POST endpoint (the streamable-HTTP
transport, SPEC.md §3) via a WSGI app — runnable on the stdlib server or any WSGI
host (gunicorn, Flask):

```python
from wsgiref.simple_server import make_server
from contextgraph_sdk import make_wsgi_app

make_server("127.0.0.1", 8788, make_wsgi_app(MyDocsProvider())).serve_forever()
# Flask:           app.wsgi_app = make_wsgi_app(provider)
# FastAPI (ASGI):  reply with respond_to_body(provider, await request.body()) in your route
```

`handle_envelope(provider, envelope)` is the transport-free state machine if you
want to wire it into a framework yourself. A runnable HTTP example lives at
`examples/example_docs_http.py`; confirm it green with
`contextgraph-inspect http http://127.0.0.1:8788` (the `malformed-input-tolerance`,
`embedding-fingerprint`, and `correlation` probes report *skipped* over HTTP —
they inspect raw framing this transport doesn't expose).

## Verify a provenance attestation

`SPEC.md` §6.5 makes a frame's provenance *evidence* rather than merely
tamper-evident: a detached Ed25519 signature over a commitment to the frame's
identity and its provenance chain. `contextgraph_sdk.attest` implements the
whole construction — the length-prefixed link encoding, the source-first chain
fold, the frame commitment, an RFC 6962 Merkle root over a result set with
inclusion proofs, and verification.

```python
from contextgraph_sdk import verify_frame_attestation, Verdict

result = verify_frame_attestation("repo-graph", frame, attestation, public_key)
if result.verdict != Verdict.VALID:
    # Never a boolean: "the frame changed after signing" and "the key is
    # wrong" call for opposite responses, and F9 says an unverifiable
    # attestation degrades a frame to unattested rather than disqualifying it.
    print(result)
```

Signing is not here. The protocol specifies the preimage, never the custody of
the key: a provider computes `frame_commitment(...)`, signs those 32 bytes with
whatever backend holds its key, and assembles the attestation itself.

Two things worth knowing:

- **`len(s)` is not a UTF-8 byte count.** The §6.5.1 length prefix is bytes;
  `len` on a `str` counts code points, which differs for every non-ASCII
  string. This SDK measures what `s.encode("utf-8")` produced.
- **Ed25519 verification carries no dependency.** The standard library has no
  Ed25519 and this SDK promises no third-party packages, so
  `contextgraph_sdk._ed25519` is a self-contained RFC 8032 **verifier** —
  never a signer — matching `ed25519-dalek`'s `verify_strict`. It is checked
  against RFC 8032 §7.1's own vectors, against a dalek-produced signature, and
  differentially against the `cryptography` package wherever that happens to be
  installed.

The vectors are shared across every language:

```sh
cd sdk/python && python3 -m unittest discover -s tests -v
```

They come from `tests/vectors/attestation-vectors.json`, which the Rust
reference publishes and pins.

## Prove it conformant

From the repository root, with the Rust bins built:

```sh
cargo build --workspace --bins
./.github/scripts/conformance-external.sh -- python3 sdk/python/examples/example_docs.py
```

A green run is the machine-checkable claim that your provider honors the protocol.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.
