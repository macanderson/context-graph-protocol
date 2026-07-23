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

## Prove it conformant

From the repository root, with the Rust bins built:

```sh
cargo build --workspace --bins
./.github/scripts/conformance-external.sh -- python3 sdk/python/examples/example_docs.py
```

A green run is the machine-checkable claim that your provider honors the protocol.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.
