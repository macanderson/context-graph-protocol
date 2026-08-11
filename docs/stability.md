# Version & stability

Context Graph Protocol (CGP) has two independent version axes:

- **Crate version:** `1.0.0`, inherited from the workspace `Cargo.toml` by the
  public Rust crates. This follows ordinary semantic versioning.
- **Protocol version:** `contextgraph/1.0`, exposed as
  `contextgraph_types::PROTOCOL_VERSION` and negotiated on the wire.

A crate patch can improve implementation behavior without changing the wire
protocol. A protocol-breaking change requires a new protocol and crate major.

## The 1.0 stability guarantee

The `contextgraph/1.0` wire contract is frozen. Within the `contextgraph/1`
family, changes are additive: defined fields are not removed, renamed, or
repurposed; receivers continue to follow the extensibility rules in SPEC §13.
Rust crates follow semver: `1.x` releases preserve public compatibility, while
a breaking redesign requires both `contextgraph/2.0` and crate version `2.0.0`.

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

Use a compatible stable requirement such as `contextgraph-types = "1"`, and
upgrade within `1.x` normally. Do not hardcode a protocol identifier: use
`contextgraph_types::PROTOCOL_VERSION` and
`contextgraph_host::wire::versions_compatible`, or implement the equivalent
major-family comparison in another language.

## MSRV and edition

The Rust crates use `rust-version = "1.90"` and edition 2024. An MSRV increase
will be handled as a semver-significant compatibility decision.
