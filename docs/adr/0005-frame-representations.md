# 0005 — Frame representations (full, compact, reference)

**Status:** Accepted (draft; `contextgraph/1.0-draft`)

**Context:** CGEP lifecycle work, phase 2 — frame representations. Numbering is
provisional: this ADR lands ahead of PR #33's ADR set (0002–0004); if it
collides on rebase, renumber — the decision, not the number, is what matters.

## Context

`ContextFrame` inlines its content as a required `String`. That is honest for a
snippet, but wasteful or impossible for large or already-durable material: a
host that only needs a pointer still pays to move the bytes, and a provider that
holds content behind a resolver cannot return a frame at all without copying it
inline. The lifecycle build prompt (`§ContextFrame representations`) specifies an
additive fix: let a frame *state how it carries its content*.

A witness test (`frame_representation_witness.rs`) shipped ahead of this change
and failed on `main` itself, because a reference frame carrying no inline content
could not deserialize. This ADR records the implementation that satisfies it.

## Decision

Extend the existing `ContextFrame` — do **not** introduce a competing frame
shape — with a `representation` discriminant and its supporting fields:

- `full` — canonical inline `content` is required. This is the default; a frame
  with no `representation` field is `full`, and a `full` frame omits the field on
  the wire, so pre-representation providers and stored frames are unchanged.
- `compact` — inline `content` (a transformed rendering) **plus** the inline hash
  (`content_digest`), `canonical_content_hash`, a `transform` identity, and a
  `content_ref`.
- `reference` — **no** inline content; only a `content_ref` and
  `canonical_content_hash`. Never encoded as `content: ""`; the field is omitted.

Supporting additive fields: `content_fidelity`, `canonical_content_hash`,
`content_ref { provider_id, uri, expires_at? }`, `transform`,
`minimum_content_fidelity`, `inline_content_requirement`, `canonical_token_cost`,
and `tokenizer_ref`. `content` becomes `Option<String>` (absent for references).

Negotiation is additive too: `ContextQuery.representation_preferences` (absent ⇒
`[full]`) and `Capabilities.representations` + `Capabilities.resolve`, with the
consistency rule that advertising `compact`/`reference` requires `resolve`.

The invariants are enforced in three places that must agree: Rust
(`ContextFrame::representation_invariants`), the JSON Schema (`allOf`
conditionals), and the conformance/unit tests.

### Two names for two hashes

The spec names the inline-content hash `content_hash` and the full-source hash
`canonical_content_hash`. This repo already has `content_digest` with exactly the
inline-content meaning, and it is the third component of `FrameId`. We **keep
`content_digest`** as the inline hash (it *is* the spec's `content_hash` under an
established name) and **add `canonical_content_hash`** as the distinct
full-source hash. A rename to `content_hash` is a separate, compatibility-aware
change (the build prompt says not to fold renames into feature work).

### `token_cost` stays required, for now

The build prompt makes `token_cost` optional and adds `canonical_token_cost` +
`tokenizer_ref`. We add the latter two additively but **keep `token_cost: u32`
required**. Making it optional reopens the exact `budget-honesty` B3 decision
that PR #33 lands (`token_cost` MUST equal `ceil(utf8_len(content)/4)`), on the
same field and the same budget-accounting functions PR #33 rewrites. Deferring
`token_cost`-optionality to that reconciliation avoids a conflict on PR #33's
headline field; the witness fixture is satisfied with `token_cost: 0` present.

## Consequences

- `full`, `compact`, and `reference` are **representations of one frame**, not
  replacement entities. `CompiledContextFrame`, snapshots, aggregate deltas, and
  prompt rendering remain host concerns and are **not** part of this protocol.
- Backward compatible: every new field is optional/default-absent, legacy full
  frames deserialize and re-serialize byte-for-byte, and query-only providers are
  untouched.
- A `reference` frame must be resolved (`context/resolve`, a later phase) before
  its content can be composed; until then a host renders it as empty rather than
  fabricating bytes.
- Resolution routing, compaction algorithms, and token allocation stay provider/
  host policy and are not standardized here.
