# {{PROJECT_NAME}}

A [Context Graph Protocol](https://contextgraphprotocol.org) provider, scaffolded with
`create-contextgraph-provider`. It ships **conformant from the first commit** —
`.github/workflows/conformance.yml` runs `contextgraph-inspect` against it in CI
on every push.

## Layout

- `provider.py` — your provider's behavior (edit `query`), and the stdio entrypoint.
- `server.py` — the HTTP entrypoint (one POST endpoint a host calls).
- `scripts/check_conformance.py` — the local + CI conformance check.

## Develop

```sh
python3 -m venv .venv && . .venv/bin/activate
pip install -e .

# Prove it conformant (needs contextgraph-inspect on PATH —
# `cargo install contextgraph-conformance`, or set CONTEXTGRAPH_INSPECT):
python scripts/check_conformance.py
```

## Run it

```sh
# stdio (a host spawns this):
python provider.py

# HTTP (a host POSTs to this):
python server.py
# then, in another shell:
contextgraph-inspect http http://127.0.0.1:8788
```

## Next

Edit `provider.py` — swap the two canned docs frames for your real retrieval.
Keep `token_cost` computed with `budget_tokens(...)`, give every frame a
`citation_label`, and declare your `data_flow` honestly.
`python scripts/check_conformance.py` tells you the moment you drift out of spec.

## License

MIT OR Apache-2.0.
