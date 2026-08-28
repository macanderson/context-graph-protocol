# Version & stability

Context Graph Protocol (CGP) has two independent version axes:

- **Crate version:** `2.0.0`, inherited from the workspace `Cargo.toml` by the
  public Rust crates. This follows ordinary semantic versioning.
- **Protocol version:** `contextgraph/1.0`, exposed as
  `contextgraph_types::PROTOCOL_VERSION` and negotiated on the wire.

The axes move independently, and today they disagree: the crates are on `2.x`
while the wire contract is still `contextgraph/1.0`. A crate patch can improve
implementation behavior without changing the wire protocol; a crate major can
reshape the Rust API over an unchanged wire. The implication runs one way only —
a protocol-breaking change requires a new protocol **and** a crate major, but a
crate major does not imply a protocol break.

## The 1.0 stability guarantee

The `contextgraph/1.0` wire contract is frozen. Within the `contextgraph/1`
family, changes are additive: defined fields are not removed, renamed, or
repurposed; receivers continue to follow the extensibility rules in SPEC §13.
Rust crates follow semver independently of that freeze: within a crate major,
releases preserve public compatibility; a breaking Rust API change takes a crate
major and nothing else. Crate `2.0.0` is the first exercise of this — the
open-`FrameKind` change ([ADR 0011](adr/0011-open-frame-kind-vocabulary.md)) is
Rust-breaking (`#[non_exhaustive]`, a new `Unknown(String)` variant, no longer
`Copy`) while emitting and accepting exactly the bytes `contextgraph/1.0`
already defined. A **wire**-breaking redesign is the stricter case: it requires
`contextgraph/2.0` and a crate major together.

The former `contextgraph/1.0-draft` identifier belongs to the same major family
and remains wire-compatible. This deliberate compatibility means existing
Stella deployments and other draft-era providers can migrate without a flag
day. New implementations should emit `contextgraph/1.0`.

## Version-family negotiation

`contextgraph_host::wire::versions_compatible` compares the major-family prefix
through the protocol major. Consequently `contextgraph/1.0-draft`,
`contextgraph/1.0`, and future additive `contextgraph/1.x` revisions
interoperate; `contextgraph/2.0` does not.

## Conformance

"CGP conformant" means green on `contextgraph-conformance` for the declared
capability set. Providers should run the suite on every implementation change;
hosts with custom composition should also run the host and composition suites.
The attested `contextgraph-1.0` fixture bundle keeps schema, Rust serialization,
and external implementations tied to the stable contract.

## Dependency guidance

Use a compatible stable requirement such as `contextgraph-types = "2"`, and
upgrade within `2.x` normally. `1.x` remains on crates.io and still speaks
`contextgraph/1.0` on the wire, so a `1.x` consumer interoperates with a `2.x`
one; see [MIGRATION.md](../MIGRATION.md) §5 for the source changes the major
asks of you. Do not hardcode a protocol identifier: use
`contextgraph_types::PROTOCOL_VERSION` and
`contextgraph_host::wire::versions_compatible`, or implement the equivalent
major-family comparison in another language.

## MSRV and edition

The Rust crates use `rust-version = "1.90"` and edition 2024. An MSRV increase
will be handled as a semver-significant compatibility decision.
