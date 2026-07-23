# 0006 — Prompt ingestion as a local provider

**Status:** Accepted (draft; `contextgraph/1.0-draft`)

## Context

Everything CGP disciplines — budget honesty, provenance, content-addressed
reuse, byte-stable composition — applies to what a *provider* returns. The one
input that bypasses all of it is the largest and least disciplined: the text a
user pastes into the prompt. A realistic turn looks like

> here are 75 lines of a log, and 15 rows of a table, and the directory
> `./src/net`, and what I actually want is: figure out why the retry loop
> gives up.

Four different things wear one trenchcoat there, and only one of them is
*intent*. The log and the table are **evidence**; the directory is an
**anchor** the graph/overview tools resolve far better than pasted text ever
could; the last sentence is the **query**. Pasting them as one undifferentiated
blob means: the whole thing is re-sent verbatim every turn (no caching, no
dedup), its cost is never accounted, nothing is content-addressed, and the model
is handed material it must itself decide is 90 % irrelevant. CGP already knows
how to carry each of these honestly — it just never got to, because the paste
never became frames.

This ADR records a **host-side reference component** that closes that gap: it
treats the user's paste as a *local provider*. Like
[`compose_context`](../../contextgraph-host/src/compose.rs) — the ingestion-side
of which this is the dual — it is reference host behavior, **not** part of the
wire protocol. No envelope shape or `SPEC.md` normative text changes; it uses
only the frame fields [ADR 0005](./0005-frame-representations.md) already added.

It does carry **one corrective schema fix**, and finding it is the point.
Ingesting is the first thing in the repo to *serialize* frames from the
reference Rust type and validate that JSON against
[`schema/contextgraph-envelope.schema.json`](../../schema/contextgraph-envelope.schema.json)
(prior schema coverage validated only hand-authored example transcripts). Doing
so exposed that the schema listed `provenance` and `relations` as globally
`required`, while the reference `ContextFrame` serializer omits both when empty
(`skip_serializing_if = "Vec::is_empty"`) and no frame-validity check (SPEC §6,
F1–F5) requires either. A frame with no graph edges — which every ingest frame
is — therefore failed schema validation. The schema's `required` is corrected to
exactly what the serializer always emits (`id`, `kind`, `title`, `score`,
`token_cost`); `content` stays governed per-representation by the existing
`allOf`. This is the repo's own discipline working as intended: a claim
("frames are wire-conformant") made loud and then made true.

### What it is not

It does **not** rewrite the user's intent. The single load-bearing UX guarantee
is that intent prose passes through **verbatim** as `query.goal`; only evidence
is mediated. A mechanism that silently paraphrased what the user asked for would
trade token waste for the strictly worse failure of meaning loss. It also does
not promise "zero wasted tokens" — relevance is only knowable downstream, and
the salient line in a log is often the `WARN` three seconds before the `ERROR`,
not the `ERROR` a dumb filter would keep. The achievable and better guarantee is
**bounded default cost with lossless retrieval**: the model sees a distilled,
budgeted rendering; the full bytes stay content-addressed and pullable.

## Decision

Add `contextgraph_host::ingest`: a deterministic segmenter, a content-addressed
artifact store, and an `IngestProvider` that implements the ordinary
[`ContextProvider`](../../contextgraph-host/src/provider.rs) trait. A paste in,
a normal provider out.

**Segmentation is deterministic — never model-driven.** A paste is split into
blocks and each is classified by cheap heuristics into `Log`, `Table`, `Code`,
`Prose`, or `PathRef`. This is the same posture as `validate.rs`: hand-rolled,
dependency-light, reproducible from the bytes alone. Classification is surfaced
in the returned `SegmentReport` list so a host UI can render correctable pills
("log · 75 lines", "table · 15 rows", "anchor · ./src/net") rather than
transforming input invisibly — the second UX guarantee.

**Routing by segment kind.**

- `PathRef` → a `query.anchors` entry. Zero content, zero tokens; the graph
  provider resolves it. It is *never* provenance — a path is focal, not a byte
  range the host re-reads.
- `Prose` → a `full` `doc` frame, exact and verbatim (evidence the user chose to
  include, distinct from the intent, which is separate).
- `Log` → an `episode` frame, `Table` → a `fact` frame, `Code` → a `snippet`
  frame — served `compact` when a smaller faithful rendering exists, `full`
  otherwise.

**Every artifact is content-addressed and lossless.** The full source bytes are
stored under their SHA-256; a `compact` frame carries a distilled inline
rendering *plus* `canonical_content_hash` over the full bytes and a
`content_ref` back into the store. The same paste twice yields the same hash,
the same frame id, and therefore one deduplicated frame — the dedup and
cache-stability CGP's F4 (one spelling per instant) and F5 (lowercase digests)
rules exist to enable now reach the ingestion side too.

**Honest by construction.** Each served frame recomputes `token_cost =
ceil(utf8_len(content)/4)` (§B3) over the *inline content it actually emits* —
the full rendering and the compact rendering have different inline bytes, so the
cost and the inline `content_digest` are computed per representation, never
carried across a representation flip. Every frame satisfies its
`representation_invariants`.

**Provenance uses the `derivation` kind, not `file`.** Pasted text has no URI a
host can independently re-read, so a `file` digest (§F5) would be a lie and would
trip `provenance_with_unusable_digests`. The real hash lives in
`canonical_content_hash`; provenance records only that the frame was *derived
from a paste*.

**`verify` is exact and cheap.** Because artifacts are content-addressed and
immutable, `IngestProvider::verify` answers `valid` when a held digest matches an
artifact it served, `stale` (with the current digest) when the id is known but
the digest differs, and `gone` when the id is unknown — the store is
authoritative-complete for the session. A re-paste never needs to re-travel.

### `capabilities.resolve = true` — and why this is not an ADR 0004 dead flag

The provider serves `compact`/`reference` frames, and
`representations_consistent` *requires* `resolve: true` for that to be honest — a
provider handing back a `content_ref` it cannot rehydrate is exactly the lie that
rule prevents.

[ADR 0004](./0004-dead-capability-surface.md) deleted `upsert`/`subscribe`
precisely because they were flags with **no callable path anywhere** — no wire
method, no host API, no implementation. `resolve` is the opposite on both
counts, and that distinction is the whole justification:

1. It has a **live consistency rule** that a conformance suite can and does check
   (`representations_consistent`), so the flag is falsifiable, not decorative.
2. It has a **working in-process rehydration path today**: a host that wants the
   full bytes re-queries with `representation_preferences = [full]`, and
   `IngestProvider::query` returns the full-content frame straight from the
   artifact store — honest `token_cost`, honest `content_digest`, no reference
   left dangling. That path is implemented and tested.

The dedicated `context/resolve` wire method remains a later phase (ADR 0005). Its
absence does not make `resolve` a dead flag here, because the capability it
denotes — "I can give you back the full bytes behind a `content_ref` I handed
you" — is real, exercised, and covered. When the wire method lands,
`IngestProvider` overrides the trait default; nothing about this decision
changes.

### Local-only, egress-free

`IngestProvider` declares `DataFlow { reads: true, egress: false }` with an
`EgressScope::LocalOnly` scope. The entire point is that a paste the user typed
never leaves the machine, so the provider is auto-permitted (§C1 gates only
egress providers) — no consent friction for the one provider that is definitely
local.

## Consequences

- The paste stops being the protocol's blind spot: it is budgeted, cited,
  content-addressed, deduplicated, and revalidated like any other source, and it
  composes byte-stably through the existing `compose_context`.
- A dependency-free SHA-256 was **not** hand-rolled; the host crate takes a
  direct `sha2` dependency. The "keep it crypto-free" argument is a
  *`contextgraph-types`* value (that crate stays zero-dep beyond serde); the host
  runtime already pulls SHA-256 transitively and is the right home for real
  hashing.
- Segmentation heuristics are intentionally few (`Log`, `Table`, `Code`,
  `Prose`, `PathRef`) behind a single `classify` seam. Stack-trace and richer
  tabular parsing are additive follow-ups, not blockers — they do not change any
  frame shape.
- Distillation quality (which log lines are salient, how a table is sampled) is
  provider policy, exactly as ranking and compaction are elsewhere. It is not
  standardized, and improving it never touches the protocol.
- The JSON Schema now requires of a `ContextFrame` only what the reference
  serializer always emits. A new conformance test
  (`contextgraph-conformance/tests/ingest_conformance.rs`) validates real
  ingested frames — full, compact, and reference — against the SPEC §6 frame
  check, the §B budget check, the representation invariants, and the schema's
  required-key set, closing the gap that let the mismatch exist unexercised.
