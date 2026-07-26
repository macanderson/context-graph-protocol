# Prompt ingestion: treating the paste as a provider

Everything the Context Graph Protocol disciplines — budget honesty, provenance,
content-addressed reuse, byte-stable composition — applies to what a *provider*
returns. The one input that bypasses all of it is the largest and least
disciplined: the text a user pastes into the prompt.

A realistic turn looks like this:

> here are 90 lines of a log, and 30 rows of a table, and a Java traceback, and
> the directory `./src/net`, and what I actually want is: figure out why the
> retry loop gives up.

Four different things wear one trenchcoat there, and only one of them is
**intent**. The log, the table, and the traceback are **evidence**; the directory
is an **anchor** the graph and overview tools resolve far better than pasted text
ever could; the last sentence is the **query**. Pasted as one blob, the whole
thing is re-sent verbatim every turn (no cache, no dedup), its cost is never
accounted, nothing is content-addressed, and the model is handed material it must
itself decide is 90 % irrelevant.

`contextgraph_host::ingest` closes that gap by treating the paste as a **local
provider**. It is the ingestion-side dual of
[`compose_context`](./context-reuse.md#1-deterministic-composition): the same
host-side reference behavior, running in the other direction.

> **Not normative.** Nothing on this page is part of the wire protocol. No
> envelope shape changes, no `SPEC.md` requirement is added, and a provider
> implements nothing to make it work — it uses only the frame fields
> [ADR 0005](./adr/0005-frame-representations.md) already defined. The design
> rationale is [ADR 0006](./adr/0006-prompt-ingestion-as-a-local-provider.md);
> this page is the user-facing guide to using it.

## What it produces

One call turns a decomposed paste into three things:

```rust
use contextgraph_host::{Host, IngestConfig, PasteIngest, ingest_paste};

let bundle = ingest_paste(
    PasteIngest {
        intent: "figure out why the retry loop gives up".into(),
        anchors: vec![],                       // focal URIs the host already knows
        attachments: vec![log, table, traceback, "./src/net".into()],
    },
    IngestConfig::default(),
);

// 1. a ready-to-fan-out query   2. an ordinary provider   3. the report
let mut host = Host::new();
host.register(Box::new(bundle.provider));
let fanout = host.query_all(&bundle.query).await;
println!("{}", fanout.compose());
```

Each segment of the paste is routed by what it *is*:

| Segment | Becomes | Cost |
| ------- | ------- | ---- |
| intent | `ContextQuery::goal`, **verbatim** | — |
| a path or directory (`./src/net`) | a `ContextQuery::anchors` entry | zero tokens |
| a log | an `episode` frame | distilled, budgeted |
| a stack trace | an `episode` frame | distilled, budgeted |
| a table | a `fact` frame | distilled, budgeted |
| a code block | a `snippet` frame | distilled, budgeted |
| an attached note | a `doc` frame, verbatim | its own honest cost |

### Intent is never mediated

The single load-bearing UX guarantee is that intent prose passes through
**byte-for-byte** as `query.goal`. Only evidence is mediated. A mechanism that
silently paraphrased what the user asked for would trade token waste for the
strictly worse failure of meaning loss, so the seam does not exist: there is no
code path that rewrites `intent`.

### A path is an anchor, not a frame

A directory reference costs **zero tokens** because it is not inlined at all — it
becomes a query anchor that the graph provider resolves, which it does far better
than pasted text. It is deliberately *not* provenance either: a path is focal, not
a byte range the host re-reads.

## Compact by default, `[full]` on demand

The promise is **not** "zero wasted tokens". Relevance is only knowable
downstream, and the salient line in a log is often the `WARN` three seconds before
the `ERROR`, not the `ERROR` a dumb filter would keep. The achievable — and
better — guarantee is **bounded default cost with lossless retrieval**:

- the model sees a distilled, budgeted rendering (a `compact` frame);
- the full bytes stay content-addressed and pullable, forever, at exact fidelity.

Pulling them back is an ordinary query with a different preference:

```rust
let full = ContextQuery {
    representation_preferences: vec![Representation::Full],
    ..bundle.query.clone()
};
// The same frame id, now carrying the exact source bytes, an `exact` fidelity,
// and a `token_cost` recomputed over what it actually inlined.
let rehydrated = provider.query(&full).await?;
```

That path is why the provider advertises `capabilities.resolve = true` honestly:
it can give back the full bytes behind every `content_ref` it hands out. (The
dedicated `context/resolve` wire method is a later phase; see ADR 0006 on why
this is not an [ADR 0004](./adr/0004-dead-capability-surface.md) dead flag.)

A frame that is too small to be worth distilling is served **verbatim** with
fidelity `exact` — the distiller is skipped whenever the compact rendering would
not actually be smaller, so nothing pays a summarization tax for no saving.

## Honest by construction

Every frame the ingest provider emits satisfies the same rules a third-party
provider is held to by the conformance suite:

- **Budget honesty (§B3).** `token_cost == ceil(utf8_len(content) / 4)` over the
  bytes *this representation* inlines. A `full` frame and a `compact` frame of the
  same artifact have different content, so they compute different costs and
  different inline `content_digest`s. A cost is never carried across a
  representation flip.
- **Representation invariants.** A `compact` frame carries its
  `canonical_content_hash`, `canonical_token_cost`, `content_ref`, and
  `transform`; a `reference` frame inlines nothing and costs 0.
- **Citations.** Every frame has a non-empty `title` and `citation_label`
  ("pasted log", "pasted stack trace", …), so the host can cite what it used
  without falling back to a bare id.
- **Provenance is `derivation`, never `file`.** Pasted text has no URI a host can
  independently re-read, so a `file` digest (§F5) would be a lie and would trip
  `provenance_with_unusable_digests`. The real hash lives in
  `canonical_content_hash`; provenance records only that the frame was *derived
  from a paste*.
- **Temporal bounds are F4 or absent.** A log's `valid_from`/`valid_to` come from
  normalizing its own timestamps into the [§F4 profile](./protocol-surface.md);
  a shape that cannot be spelled in F4 without inventing a year or an offset
  yields *no* window rather than a fabricated one.

## Content addressing, dedup, and cheap revalidation

Every artifact is stored under the SHA-256 of its full source bytes, and the frame
id is derived from that hash. Three consequences fall out for free:

- **The same paste twice is one frame.** Identical bytes hash identically, so a
  re-paste deduplicates rather than duplicating — within a turn and across turns.
- **Composition is byte-stable.** Because ids and digests are content-derived,
  `compose_context` renders an unchanged paste to identical bytes on every turn,
  which is exactly what keeps a provider's prompt cache warm
  ([context-reuse §1](./context-reuse.md#1-deterministic-composition)).
- **`verify` is exact and free.** Artifacts are immutable, so
  [`context/verify`](./context-reuse.md#4-context-verification) answers `valid`
  when a held digest matches one the provider served, `stale` (with the current
  digest) when the id is known but the digest differs, and `gone` when the id is
  unknown. The store is authoritative-complete for the session: **a paste never
  has to re-travel.**

## Local-only, egress-free

`IngestProvider` declares `DataFlow { reads: true, egress: false }` with an
`EgressScope::LocalOnly` scope, so it is auto-permitted — the
[consent gate](./context-reuse.md#3-consent-scopes-and-receipts) exists to stop
content leaving the machine, and the whole point of this provider is that a paste
the user typed never does.

## Pills: nothing is transformed invisibly

Segmentation is deterministic and heuristic, which means it is sometimes wrong.
The mitigation is not a better heuristic — it is **visibility**. `ingest_paste`
returns a `SegmentReport` per segment, in paste order, so a host UI renders
correctable chips above the composer rather than silently reshaping the input:

```text
[Log]        log · 115 lines · 1834 → 379 tokens   → frame frm_63cc71157fd1
[Table]      table · 34 lines · 260 → 95 tokens    → frame frm_466166b63bb8
[StackTrace] stack trace · 14 lines · 176 → 120    → frame frm_bca787198a3e
[PathRef]    anchor · ./src/net                    → anchor ./src/net
[Prose]      note · 16 tokens                      → frame frm_4f0bca1d9d62
```

(That is the example's rendering, elided to fit; the report itself carries the
kind, the summary, and a `SegmentOutcome` naming the frame id, the representation
it will be served as, and both token counts.)

Each pill names the classification, the shape, and the exact cost of what will be
inlined versus what the source cost — the numbers a user needs to notice that
their table was read as prose, and the affordance to say so.

## Segmentation and distillation are provider policy

Which lines of a log are salient, how many rows of a table to sample, how deep to
cut a stack — all of it is **provider policy**, exactly as ranking and compaction
are elsewhere in the protocol. It is not standardized, improving it never touches
the wire, and a host that wants different policy writes a different provider.

Classification walks a ladder from the least ambiguous shape to the most, and the
order is load-bearing: a fenced block is code; a lone path token is an anchor; an
exception header plus stack frames is a trace; a **timestamped** log outranks
table detection (so `ts | LEVEL | msg` is an `episode`, not a `fact`); explicit
delimiters (`|`, tab, comma) make a table; a weaker log comes next; and
whitespace-aligned columns are tried last, because the padding after a
fixed-width `INFO ` is indistinguishable from a column break.

Heuristics this cheap misread things, and the known misreads are documented in the
[module docs](https://docs.rs/contextgraph-host/latest/contextgraph_host/ingest/index.html)
rather than papered over. None of them can produce a *dishonest* frame — cost,
digest, and provenance are computed from the bytes actually emitted, whatever the
kind was decided to be — and every one of them is visible in the pill.

## Try it

The `ingest_paste` example walks the whole path end to end: it prints the pills,
registers the provider in a real `Host`, fans the query out, prints the composed
block and the usage report, proves a second turn composes to identical bytes, and
pulls one frame back in `full`.

```bash
cargo run -p contextgraph-host --example ingest_paste
# …or against your own paste:
cargo run -p contextgraph-host --example ingest_paste -- ./my-paste.txt
```

On the built-in sample — a 115-line log ending in a 25× repeated retry warning, a
33-row CSV with currency and percent columns, a 13-frame Java traceback, a
directory, and a note — a 2 288-token paste is served as 610 tokens of compact
frames (27 %), with every source byte still addressable and one re-query away.

## See also

- [Implementing a provider](./implementing-a-provider.md) — the contract this
  provider satisfies like any other.
- [Context reuse](./context-reuse.md) — frame identity, canonical composition,
  usage reports, consent receipts, and `context/verify`.
- [ADR 0006](./adr/0006-prompt-ingestion-as-a-local-provider.md) — why the paste
  is modeled as a provider, and the schema fix that building it uncovered.
