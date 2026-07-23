# Design sketch — host execution trace (`contextgraph-trace`)

> **Status: sketch, not specification.** Nothing here is part of the
> `contextgraph/1.0` surface. The `contextgraph-trace` crate implements this
> sketch as an **unpublished** workspace member so the shape can be exercised
> against real journals before any of it is proposed for the spec. Nothing here
> is normative.

## Why a trace exists at all

The conformance suite (`SPEC.md` §11) holds a *provider* honest: budget, frames,
verify, consent. Nothing holds the **host-side agent loop** honest. A harness
can pass every provider-level check and still:

- render a frame it was told is stale three turns ago (citing dead evidence);
- assemble a prompt whose declared frame costs exceed the budget it announced;
- issue the next model request while tool calls from the previous response are
  still unresolved;
- execute a tool call the model never asked for;
- replay a side effect after a crash-resume (the double-`git push` class of
  bug);
- resume from an earlier state than its own durable record and never notice.

Outcome-graded benchmarks (Harbor-style: run the agent in a container, check
the final state) cannot see any of these — a run can produce the right answer
*and* have done all six. They are **invariants over the execution trace**, so
the missing piece is a trace: an append-only NDJSON journal the harness (or an
adapter observing it) emits while it works, and a set of pure replay oracles
that hold the journal to account after the fact.

The oracles never talk to the harness. They read the journal. That split is
what makes an eventual benchmark runner agent-agnostic: one thin,
Harbor-adapter-style shim per harness maps its native logs/hooks onto this
vocabulary, and every check downstream is shared.

## The shape

One JSON object per line. Every line carries a dense sequence number, an
RFC 3339 UTC timestamp (`SPEC.md` §F4 profile), the session id, and — inside a
turn — the turn number. The event vocabulary reuses the protocol's own types:
frames are named by [`FrameId`](../../contextgraph-types/src/identity.rs)
(provider id, frame id, content digest), verify observations carry the wire
[`Verdict`](../../contextgraph-types/src/verify.rs), rendered representations
use [`Representation`](../../contextgraph-types/src/frame.rs). No frame body
ever travels in the journal — identities and costs only, the same economy
`context/verify` runs on.

```jsonc
{"seq":1,"at":"2026-07-23T09:00:00Z","session":"sess_1","event":"session_start","agent":"example-agent","harness":"stella/0.9","trace_format":"contextgraph-trace/0.1-sketch"}
{"seq":2,"at":"2026-07-23T09:00:01Z","session":"sess_1","turn":1,"event":"turn_start"}
{"seq":3,"at":"2026-07-23T09:00:02Z","session":"sess_1","turn":1,"event":"prompt_assembled",
 "budget_tokens":4096,"declared_total_tokens":180,
 "composition_digest":"sha256:2b7e…",
 "frames":[{"frame":{"provider_id":"docs","frame_id":"frm_1","content_digest":"sha256:9f2c…"},
            "token_cost":120,"citation_label":"workspace.ts L120-160"}]}
{"seq":4,"at":"2026-07-23T09:00:09Z","session":"sess_1","turn":1,"event":"model_response","tool_calls":["call_1"]}
{"seq":5,"at":"2026-07-23T09:00:10Z","session":"sess_1","turn":1,"event":"tool_call","call_id":"call_1","tool":"write_file"}
{"seq":6,"at":"2026-07-23T09:00:11Z","session":"sess_1","turn":1,"event":"side_effect","effect_id":"write:src/main.rs#1","kind":"file_write","call_id":"call_1"}
{"seq":7,"at":"2026-07-23T09:00:12Z","session":"sess_1","turn":1,"event":"tool_result","call_id":"call_1","status":"ok"}
{"seq":8,"at":"2026-07-23T09:00:13Z","session":"sess_1","turn":1,"event":"turn_end"}
{"seq":9,"at":"2026-07-23T09:00:14Z","session":"sess_1","event":"verify_observed",
 "frame":{"provider_id":"docs","frame_id":"frm_1","content_digest":"sha256:9f2c…"},"verdict":{"status":"valid"}}
{"seq":10,"at":"2026-07-23T09:00:15Z","session":"sess_1","event":"session_end","outcome":"completed"}
```

The full vocabulary: `session_start`, `session_end`, `resume`, `turn_start`,
`turn_end`, `prompt_assembled`, `model_response`, `tool_call`, `tool_result`,
`verify_observed`, `side_effect`. Deliberately minimal — each event exists
because an oracle consumes it, and nothing else is recorded.

Three contracts the vocabulary leans on:

- **`seq` is dense.** `1, 2, 3, …` with no gaps, continuing across a
  crash-resume. Density is what makes "this journal is complete" checkable at
  all; a lost tail is then visible as the delta between the last recorded `seq`
  and what a `resume` event says it recovered.
- **A turn does not survive a crash.** A `resume` implicitly closes any open
  turn and orphans any unresolved tool calls; resumed work starts a new turn.
  The oracles treat dangling calls before a `resume` as expected, and the
  *replay* of an already-performed effect after one as the defect.
- **`effect_id` names an intended-once effect.** The adapter assigns a stable
  id when the harness performs an externally visible action (file write,
  network call, command). A deliberate re-execution is a new id; the same id
  twice is the crash-replay bug, by construction.

## The oracles

Pure functions over a parsed journal, in the conformance suite's vocabulary —
named checks, pass/fail/skip, an evidence string that says exactly what broke
and at which `seq`:

| Check | Holds the harness to |
| --- | --- |
| `sequence-integrity` | dense monotonic `seq`, one session, well-formed timestamps, balanced turn markers, nothing after `session_end` |
| `turn-loop-pairing` | every model-requested call resolved exactly once before the next prompt; no phantom executions; no orphan or duplicate results |
| `assembly-budget-honesty` | declared frame costs sum to the declared total and fit the announced budget; a `reference` frame inlines nothing so costs 0 (§B1/§B3 at the point of assembly) |
| `staleness-at-use` | no frame rendered after its exact identity was last verified `stale`/`gone` (`docs/context-reuse.md` §4 V2, at the point of use) |
| `citation-at-use` | every rendered frame carries a non-empty citation label (§F3 at the point of use, not just at the provider boundary) |
| `deterministic-composition` | an unchanged frame set composes to an unchanged `composition_digest` (`docs/context-reuse.md` §1 prefix stability, finally checkable) |
| `effect-exactly-once` | no `effect_id` performed twice — the crash-replay double-side-effect bug, whether across a `resume` or within one live run |
| `resume-integrity` | a `resume` recovered exactly what the journal records — a shortfall is quantified work loss, an excess is a corrupt recovery |

The suite is deliberately adversarial in the same way `contextgraph-conformance`
is: the crate ships a golden journal that passes everything, and one fixture
per check that trips exactly that check — proving the oracle catches the broken
harness it exists for.

## What this is *not*

The trace and oracles are the protocol-side substrate. The benchmark **runner**
— chaos scheduling (killing a harness mid-turn on purpose), scripted-model
cassettes, task generators, planted-fact retention probes, memory-ablation arms
for self-improvement curves — is a separate tool that *produces* interesting
journals for these oracles to judge. It is deliberately not in this repository
today; whether it lands as an internal tool or a public agent-agnostic runner
(one adapter per harness, Harbor-style) is an open product question, and
nothing in this sketch depends on the answer.

## Relationship to `context_use` receipts

The [context-receipt design](../future/context-receipt-impact-trace/context-receipt-impact-trace-design.md)
records the observable stages `selected` → `rendered` → `cited` per frame to
answer "was this context *useful*?". The journal's `prompt_assembled` entries
are the `rendered` stage observed host-side, so a receipt's use records are
derivable from a journal — but the two answer different questions. Receipts
grade *usefulness per task*; the trace grades *invariants per session*. Neither
replaces the other, and the shared `FrameId` spine is what keeps them joinable.

## Open questions

- **Runner location and shape** — internal tool vs. public agent-agnostic
  runner with per-harness adapters. The journal contract is designed so either
  works.
- **Probe vocabulary** — planted-fact retention probes (insert a fact early,
  require it late, measure survival across compaction) need `compaction` and
  probe events. Left out until the runner exists to plant them; additive when
  it does.
- **Resume shortfall severity** — a `resume` that recovered less than the
  journal records currently *fails* `resume-integrity`. An argument exists for
  a measured-warning tier instead (the harness recovered honestly, just
  lossily); the oracle reports the exact loss either way.
- **JSON Schema** — `schema/contextgraph-envelope.schema.json` is precedent for
  machine-checking examples; the trace envelope should get the same treatment
  before any adapter outside this repo emits it.
- **Publication** — the crate stays unpublished while the shape settles.
  Publishing it is what would turn "one adapter per harness" from an internal
  recipe into an ecosystem contract.
