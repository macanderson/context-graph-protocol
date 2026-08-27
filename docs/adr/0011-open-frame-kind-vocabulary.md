# 0011 — `FrameKind` is an open vocabulary

**Status:** Accepted (`contextgraph/1.0`; Rust-semver breaking, wire-compatible)

## Context

`SPEC.md` §3.1 promises that two versions sharing a major family interoperate:
`contextgraph/1.0` and `contextgraph/1.1` are compatible by construction, and the
freeze "drops `-draft` without a flag day". §13 U2 spells out the consequence for
frame kinds — a receiver encountering an unrecognised `kind` **MUST** treat the
frame as opaque evidence, **MUST NOT** crash, and a new kind is an ordinary `1.x`
addition.

The reference implementation contradicted its own specification. `FrameKind` was
a closed `#[derive(Deserialize)]` enum over seven variants, which means:

- **A `1.1` frame breaks a `1.0` host.** Not degrades — *fails*. `serde` rejects
  an unknown variant, so the whole frame fails to deserialize, and in the NDJSON
  binding that fails the envelope. The exact flag day §3.1 promises cannot happen.
- **Adding a kind breaks every downstream `match`.** Any exhaustive match in
  consumer code stops compiling the day a variant is added, so even the
  additive-only path was a breaking change for the ecosystem.

The JSON Schema carried the same closed `enum`, and the TypeScript and Python
SDKs carried closed union/`Literal` types with the same effect at their type
layers. Only the Go SDK, which types `Kind` as `string`, was accidentally correct.

A guarantee the reference implementation cannot honour is not a guarantee.

## Decision

`FrameKind` becomes an open vocabulary, following the shape `EgressScope`
already established in this codebase: a closed **base** vocabulary of seven
kinds, plus `Unknown(String)`, with hand-written `Serialize`/`Deserialize` over a
flat string.

### `Unknown(String)`, not `#[serde(other)]`

`#[serde(other)]` requires a unit variant and **discards the original value**. A
host relaying a frame it did not fully understand would silently rewrite
`"kind": "trajectory"` to something else on the way out. A forward-compatibility
mechanism that corrupts data in a relay is worse than the failure it replaces —
the closed enum at least failed loudly.

Preserving the string means unknown kinds round-trip byte-identically, which
§13 U2 now states explicitly as a requirement.

### `#[non_exhaustive]`

The enum is marked `#[non_exhaustive]`, forcing downstream `match` expressions to
carry a wildcard arm. This is a one-time break taken now so that every *future*
kind addition is non-breaking for every consumer — which is precisely what the
additive-only promise of §15 requires but could not previously deliver.

### `FrameKind` is no longer `Copy`

Owning an unknown kind's string costs the `Copy` impl. `Copy` was an ergonomic
convenience; forward compatibility is a protocol guarantee. Where the two
conflict the guarantee wins.

## Two versioning axes, and why only one broke

This change is **wire-compatible** and **Rust-semver breaking**, and conflating
the two would misread it.

On the wire nothing changed: the same seven strings serialize identically, and
the *only* behavioural difference is that a frame which previously failed to
parse now parses. That is strictly more compatible, so no protocol-family bump is
implied — `contextgraph/1` is intact, and GOVERNANCE.md's additive-only rule is
satisfied.

In Rust, adding a variant, marking the enum `#[non_exhaustive]`, and dropping
`Copy` are all breaking, so the crates need a major version. `frame_kind_name`
in `contextgraph-host` correspondingly changes from
`fn(FrameKind) -> &'static str` to `fn(&FrameKind) -> &str` and becomes a
delegation to `FrameKind::as_str`, removing a second copy of the vocabulary.

`docs/stability.md` already documents the crate-semver / protocol-version
distinction; this is its first substantive instance.

## Alternatives considered

**`kind: String` against a registry.** Rejected. It discards the type safety that
makes the seven common cases pleasant to work with, and pushes every consumer
into stringly-typed comparisons for no gain over `Unknown(String)`.

**Leave it closed and treat new kinds as a major-family break.** Rejected. It
contradicts §3.1 and §13 U2, and makes the vocabulary effectively unextendable —
adding one frame kind would force `contextgraph/2` and a flag day for every
deployed implementation.

**Open the JSON Schema's `enum` too.** Rejected. The schema is deliberately an
*authoring-strict lint* (§13), not the interop contract; the closed enum is what
catches a typo like `"dcc"` in a fixture. A `$comment` at that definition now
states the distinction explicitly, because it is exactly where a reader would
misread the schema as the wire rule.

## Consequences

- `contextgraph-types` and `contextgraph-host` require a major version bump.
- `SPEC.md` §13 U2 additionally requires verbatim preservation of an unrecognised
  `kind` on re-emission.
- The TypeScript SDK exports `KnownFrameKind`, `KNOWN_FRAME_KINDS`, and
  `isKnownFrameKind`, with `FrameKind = KnownFrameKind | (string & {})` so
  autocomplete survives. The Python SDK exports `KnownFrameKind` and
  `KNOWN_FRAME_KINDS`, with `FrameKind = str`. The Go SDK needed no change.
- `contextgraph-conformance`'s `frame_kind_from_wire` now delegates to
  `FrameKind::from_wire` and filters on `is_known`, rather than restating the
  seven names a third time.
