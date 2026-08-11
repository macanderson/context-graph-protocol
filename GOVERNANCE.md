# Context Graph Protocol governance

This document describes how the Context Graph Protocol is maintained, how
it changes, and the maintenance of the frozen `contextgraph/1.0` family. It exists
so adopters can trust that the protocol is maintained deliberately and that
"Context Graph Protocol conformant" is a stable, verifiable claim — not a maintainer's mood.

## Roles

- **Maintainer.** Mac Anderson (`@macanderson`) is the current maintainer. The
  maintainer owns release decisions, approval of normative changes, and the
  stewardship of the frozen `contextgraph/1.0` family.
- **Contributors.** Anyone. Contributions land via pull request under the
  [DCO](./CONTRIBUTING.md) — no CLA, no copyright assignment.

Context Graph Protocol is **maintainer-led today, not committee-led** — deliberately. A steering
committee before there are independent implementations is theater. The path to
broader governance is defined below and is triggered by adoption, not ambition.

## What counts as a normative change

A change is **normative** — it affects the protocol version or the conformance
contract — if it does any of the following:

- adds, removes, or renames a field in the wire types (`contextgraph-types`);
- changes a field's required-ness or its serialized name;
- adds or tightens a [conformance requirement](./SPEC.md) (the normative home; [protocol-surface.md](./docs/protocol-surface.md#conformance-requirements) mirrors it);
- changes the [version-compatibility rule](./SPEC.md) (SPEC.md §3.1); or
- changes the envelope vocabulary or framing.

A change is **non-normative** if it only touches host internals, documentation,
error messages, ergonomics, or tests. Non-normative changes need no protocol
version bump.

## How a normative change lands

1. **Proposal.** Open an issue describing the change, the use case it unlocks,
   and the alternatives considered. Normative changes are not drive-by.
2. **Discussion.** The maintainer and contributors weigh it against the
   [seven guarantees](./docs/overview.md) and the stability promise. A change
   that silently breaks a deployed provider is rejected unless it justifies a
   new major family.
3. **Implementation.** A pull request implements it, updates this spec and
   [`CHANGELOG.md`](./CHANGELOG.md) under `[Unreleased]`, and adds or updates a
   witness — a conformance check or a wire example in [`examples/`](./examples/).
4. **Decision.** The maintainer approves or closes. While pre-1.0, the
   maintainer is the decision authority; after the freeze, normative changes
   within the `contextgraph/1` family are additive-only and additive changes SHOULD land
   without objection.

The bias is **additive, not breaking.** A new optional field is a minor change;
a removed or renamed field requires a new major family (`contextgraph/2`).

## The `contextgraph/1.0` freeze

The protocol froze on 2026-08-11 after the pre-freeze backlog and conformance
enforcement sweep. The release ships independent TypeScript, Python, Go, and
Rust provider implementations, an external-provider harness, host and custom
composition suites, and an attested fixture profile. Stella remains compatible
with both the former draft identifier and stable peers through major-family
negotiation.

Within `contextgraph/1`, normative evolution is additive-only. A removed,
renamed, or repurposed wire field requires `contextgraph/2`; conformance checks
may be strengthened when doing so enforces an already-normative requirement.

## Governance evolution

When Context Graph Protocol has a healthy base of independent providers and hosts, the maintainer
intends to transition from a single maintainer to a small group of maintainers
drawn from independent implementers, with the conformance suite as the neutral
arbiter of "conformant." The trigger is adoption, not a calendar date. Until
then, the single-maintainer + public-PR + DCO model keeps the barrier to
contribution low and the spec coherent.

## Scope

Context Graph Protocol specifies **context retrieval**: typed, budgeted, provenance-carrying,
consent-gated, conformance-verified frames that a host composes into a prompt.
It does not specify tool invocation — that is
[MCP](https://modelcontextprotocol.io)'s scope — and will not absorb it. An
agent that needs both composes them: Context Graph Protocol frames feed the prompt, MCP tools do
the work. See ["How Context Graph Protocol relates to MCP"](./docs/overview.md).
