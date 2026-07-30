# {{PROJECT_NAME}}

A [Context Graph Protocol](https://cgp.oxagen.sh) provider, scaffolded with
`create-contextgraph-provider`. It ships **conformant from the first commit** —
`.github/workflows/conformance.yml` runs `contextgraph-inspect` against it in CI
on every push.

## Layout

- `src/provider.ts` — your provider's behavior (edit `query` to serve real frames).
- `src/stdio.ts` — the stdio entrypoint (a child process a host spawns).
- `src/server.ts` — the HTTP entrypoint (one POST endpoint a host calls).
- `scripts/check-conformance.mjs` — the local + CI conformance check.

## Develop

```sh
npm install
npm run build

# Prove it conformant (needs contextgraph-inspect on PATH —
# `cargo install contextgraph-conformance`, or set CONTEXTGRAPH_INSPECT):
npm run conformance
```

## Run it

```sh
# stdio (a host spawns this):
npm run stdio

# HTTP (a host POSTs to this):
npm run serve
# then, in another shell:
contextgraph-inspect http http://127.0.0.1:8787
```

## Next

Edit `src/provider.ts` — swap the two canned docs frames for your real
retrieval. Keep `token_cost` computed with `budgetTokens(...)`, give every frame
a `citation_label`, and declare your `data_flow` honestly. `npm run conformance`
tells you the moment you drift out of spec.

## License

MIT OR Apache-2.0.
