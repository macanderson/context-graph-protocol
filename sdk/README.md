# Provider SDKs

Idiomatic SDKs for building **conformant** Context Graph Protocol providers in
languages other than Rust. Each one implements the same line-oriented JSON wire
and is held to the same bar: its example provider must pass the Rust
conformance suite, driven language-neutrally by
`contextgraph-inspect stdio -- <program> [args...]`.

That shared oracle is the point. "≥2 independent implementations pass
conformance" is a GOVERNANCE.md freeze criterion; every SDK here that goes green
is one more independent implementation proving the wire is real.

| SDK | Location | Status |
| --- | --- | --- |
| TypeScript | [`sdk/typescript`](./typescript) | ✅ conformant — passes all 7 checks in CI |
| Python | [`sdk/python`](./python) | ✅ conformant — passes all 7 checks in CI |
| Go | [`sdk/go`](./go) | ✅ conformant — passes all 7 checks in CI |

Every SDK is validated the same way:

```sh
cargo build --workspace --bins
.github/scripts/conformance-external.sh -- <the SDK's example provider command>
```

`conformance-external.sh` asserts the provider is **green** (all seven checks).
The companion `conformance-red.sh` proves the *suite* catches cheaters using the
Rust fixture, so an SDK provider only has to be honest, not reimplement the
misbehaviour modes.

Every SDK also ships an **HTTP adapter** — the same provider behind one POST
endpoint (the streamable-HTTP transport, SPEC.md §3): `createHttpHandler`
(TypeScript), `make_wsgi_app` (Python), `Handler` (Go), each with a runnable
`example-docs-http` provider that goes green under
`contextgraph-inspect http <url>`.

## Scaffold a new provider

[`create-contextgraph-provider`](./create-contextgraph-provider) generates a
conformant provider project (TypeScript or Python) wired to both transports,
**with a GitHub Actions workflow that runs `contextgraph-inspect` against it on
every push** — so it's conformant from the first commit:

```sh
npm create contextgraph-provider@latest my-provider              # TypeScript
npm create contextgraph-provider@latest my-provider -- --lang python
```

Conformant is a separate axis from **published**: see
[`PUBLISHING.md`](./PUBLISHING.md) for each SDK's registry status and the
release checklist. As of this writing only the TypeScript SDK is on a real
registry (npm); Python and Go are conformant but not yet installable outside
a checkout.
