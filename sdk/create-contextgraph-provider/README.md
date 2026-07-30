# create-contextgraph-provider

Scaffold a **conformant** [Context Graph Protocol](https://cgp.oxagen.sh)
provider — in TypeScript or Python — with `contextgraph-inspect` wired into its
CI from the first commit. The generated project passes the conformance suite out
of the box, so you start from green and stay honest as you edit.

## Use

```sh
# npm create shorthand (recommended):
npm create contextgraph-provider@latest my-provider

# or run the CLI directly:
npx create-contextgraph-provider my-provider --lang python
```

## Options

| Option | Default | Meaning |
|---|---|---|
| `--lang <typescript\|python>` | `typescript` | Which language template to generate. |
| `--name <package-name>` | target dir basename | Project / package name. |
| `--sdk <dependency-spec>` | published version range | Override the SDK dependency — point it at a local checkout to try an unpublished SDK. |
| `--force` | off | Write into a non-empty directory. |

## What you get

A ready-to-run provider with **both transports** wired to the official SDK:

- a **stdio** entrypoint (a child process a host spawns), and
- an **HTTP** entrypoint (one POST endpoint a host calls),

plus a `check-conformance` script and a bundled `.github/workflows/conformance.yml`
that runs `contextgraph-inspect` against the provider on every push. Editing the
one `provider` module is all it takes to serve your real frames.

## Trying it against an unpublished SDK

Until the SDKs are on npm / PyPI (see issue #59), point `--sdk` at a local
checkout of this repo:

```sh
# TypeScript — a file: dependency on the built SDK package:
create-contextgraph-provider my-provider \
  --sdk file:/abs/path/to/context-graph-protocol/sdk/typescript

# Python — install the local SDK into your venv, then generate:
pip install /abs/path/to/context-graph-protocol/sdk/python
create-contextgraph-provider my-provider --lang python
```

Run the generated conformance check with a prebuilt prober by setting
`CONTEXTGRAPH_INSPECT=/abs/path/to/contextgraph-inspect`.

## License

MIT OR Apache-2.0.
