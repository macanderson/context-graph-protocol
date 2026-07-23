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
| Go | [`sdk/go`](./go) | 🚧 planned (needs a Go toolchain in CI to validate) |

Every SDK is validated the same way:

```sh
cargo build --workspace --bins
.github/scripts/conformance-external.sh -- <the SDK's example provider command>
```

`conformance-external.sh` asserts the provider is **green** (all seven checks).
The companion `conformance-red.sh` proves the *suite* catches cheaters using the
Rust fixture, so an SDK provider only has to be honest, not reimplement the
misbehaviour modes.
