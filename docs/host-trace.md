# Host execution trace (`contextgraph-trace`)

> **Status: sketch stage — not part of `contextgraph/1.0`.** This page is the
> specification the published [`contextgraph-trace`](https://crates.io/crates/contextgraph-trace)
> crate implements: the journal wire format (`TRACE_FORMAT =
> "contextgraph-trace/0.1-sketch"`) and the replay oracles that judge it.
> Nothing here is normative for providers or hosts that implement the
> protocol, and the journal format may change in any `0.x` release of the
> crate. Gate on the `TRACE_FORMAT` a journal declares, never on the crate
> version ([`stability.md`](./stability.md) explains what `0.x` does and does
> not promise). Where this page and the crate disagree, the crate is right and
> this page has a bug.

## Why a trace exists at all

The conformance suite (`SPEC.md` §11) holds a *provider* honest: budget,
frames, verify, consent. Nothing holds the **host-side agent loop** honest. A
harness can pass every provider-level check and still:

- render a frame it was told is stale three turns ago (citing dead evidence);
- assemble a prompt whose declared frame costs exceed the budget it announced;
- issue the next model request while tool calls from the previous response are
  still unresolved;
- execute a tool call the model never asked for;
- replay a side effect after a crash-resume (the double-`git push` class of
  bug);
- resume from an earlier state than its own durable record and never notice.

Outcome-graded benchmarks — run the agent in a container, check the final
state — cannot see any of these: a run can produce the right answer *and* have
done all six. They are **invariants over the execution trace**. So the crate
defines a trace — an append-only NDJSON journal a harness (or an adapter
observing it) emits while it works — and a set of pure replay oracles that
hold the journal to account after the fact.

The oracles never talk to the harness; they read the journal. That split is
what makes an eventual benchmark runner agent-agnostic: one thin adapter per
harness maps its native logs or hooks onto this vocabulary, and every check
downstream is shared.

## The journal

One JSON object per line (`Journal::from_ndjson`). Blank lines are permitted;
any other line that does not parse as an event is an error naming its line
number. Parsing is strict on purpose: a lenient parser would let a harness
escape the oracles by garbling exactly the events that would convict it. An
empty journal is an error, not a vacuous pass.

Every line carries the same envelope (`TraceEvent`), with the event body
flattened beside it and tagged by `event`:

| Field | Type | Meaning |
| --- | --- | --- |
| `seq` | integer | Dense and strictly increasing from 1, continuing across a crash-resume. |
| `at` | string | RFC 3339 UTC timestamp, the protocol's `SPEC.md` §F4 profile. |
| `session` | string | The session this recording belongs to. One journal records one session, including its resumes. |
| `turn` | integer, optional | The open turn. Present on `turn_start`/`turn_end` and on every event inside a turn; absent between turns. |
| `event` | string | The event kind, below. |

The vocabulary reuses the protocol's own types rather than inventing a parallel
identity: frames are named by `FrameId` (`provider_id`, `frame_id`, optional
`content_digest`), verify observations carry the wire `Verdict`, and rendered
representations use `Representation`. **No frame body ever travels in the
journal** — identities and costs only, the same economy `context/verify` runs
on.

### Events

The vocabulary is deliberately minimal. Each event exists because an oracle
consumes it, and nothing else is recorded.

| `event` | Fields | Records |
| --- | --- | --- |
| `session_start` | `agent`, `harness`, `model`?, `trace_format`? | The recording opens. `trace_format` SHOULD be `TRACE_FORMAT`, so an oracle suite can refuse a vocabulary it does not understand. |
| `session_end` | `outcome`: `completed` \| `aborted` | The session tore down deliberately. A journal without one records a crash. `aborted` permits unresolved tool calls. |
| `resume` | `last_seq_seen` | The harness came back after a crash, having recovered events up to `last_seq_seen`. |
| `turn_start` | — | A turn opens; the envelope `turn` names it. |
| `turn_end` | — | The open turn closes; the envelope `turn` must match. |
| `prompt_assembled` | `budget_tokens`, `declared_total_tokens`, `composition_digest`?, `frames` | The harness composed a prompt and sent it. Recording assembly *as* the model request is deliberate: every context guarantee is checked at the point of use. |
| `model_response` | `tool_calls` (ids, may be empty) | The model answered. Each requested id must be resolved exactly once before the next `prompt_assembled`. |
| `tool_call` | `call_id`, `tool` | The harness began executing a model-requested call. |
| `tool_result` | `call_id`, `status`: `ok` \| `error` \| `rejected` | A requested call was resolved. `rejected` (declined by a permission gate or policy) needs no `tool_call`: declining is a resolution, not an execution. |
| `verify_observed` | `frame`, `verdict` | The host observed a `context/verify` answer for a frame it holds. The verdict is copied straight through from the wire. |
| `side_effect` | `effect_id`, `kind`, `call_id`? | The harness performed an externally visible action (`file_write`, `network`, `command`, …). |

Each entry of `prompt_assembled.frames` is a `RenderedFrame`: `frame` (a
`FrameId`), `representation` (omitted means `full`, as on the frame wire
shape), `token_cost` (the inline cost the harness accounted — a `reference`
frame inlines nothing, so it costs 0), and `citation_label`.

### Three contracts the vocabulary leans on

- **`seq` is dense.** `1, 2, 3, …` with no gaps, continuing across a
  crash-resume. Density is what makes "this journal is complete" checkable at
  all: a lost tail becomes the measurable delta between the last recorded `seq`
  and what a `resume` says it recovered.
- **A turn does not survive a crash.** A `resume` implicitly closes any open
  turn and orphans any unresolved tool calls; resumed work starts a new turn.
  Dangling calls before a `resume` are expected; *replaying* an
  already-performed effect after one is the defect.
- **`effect_id` names an intended-once effect.** The adapter assigns a stable
  id when the harness performs an externally visible action. A deliberate
  re-execution is a new id, so the same id twice is the crash-replay bug by
  construction.

### Example

The crate's golden journal, `contextgraph-trace/fixtures/golden.ndjson`, is the
canonical example: two turns, a `reference` frame, a verify observation, and a
side effect, passing every oracle. Its first lines:

```jsonc
{"seq":1,"at":"2026-07-23T09:00:00Z","session":"sess_golden","event":"session_start","agent":"example-agent","harness":"stella/0.9","model":"claude-opus-4-8","trace_format":"contextgraph-trace/0.1-sketch"}
{"seq":2,"at":"2026-07-23T09:00:01Z","session":"sess_golden","turn":1,"event":"turn_start"}
{"seq":3,"at":"2026-07-23T09:00:02Z","session":"sess_golden","turn":1,"event":"prompt_assembled","budget_tokens":4096,"declared_total_tokens":120,"composition_digest":"sha256:2b7e2b7e","frames":[{"frame":{"provider_id":"docs","frame_id":"frm_1","content_digest":"sha256:9f2c9f2c"},"token_cost":120,"citation_label":"workspace.ts L120-160"},{"frame":{"provider_id":"docs","frame_id":"frm_2"},"representation":"reference","token_cost":0,"citation_label":"Deployment runbook"}]}
```

`golden-resume.ndjson` beside it is the same shape across a crash and resume.

## The oracles

`run_oracles` runs every check below over a parsed journal and returns a
`TraceReport` in the conformance suite's vocabulary: each check is named,
passes, fails, or is skipped (`CheckStatus` — a check the journal does not
exercise, such as `resume-integrity` on a run that never crashed, is declared
skipped rather than counted as a pass), and carries evidence naming the exact
`seq` numbers involved. The oracles are pure and independent — each walks the journal
itself, so a failure in one cannot mask a failure in another — and they never
panic: every defect becomes a failing check. The names are stable constants
(`ALL_CHECKS`).

| Check | Holds the harness to |
| --- | --- |
| `sequence-integrity` | `seq` dense from 1; the journal opens with `session_start` and holds one session; timestamps in the §F4 profile; turn markers balanced; nothing after `session_end`. Every other oracle leans on this one. |
| `turn-loop-pairing` | Every model-requested call resolved exactly once before the next prompt; no phantom executions (a call the model never requested); no result for a call never made or already resolved. Calls orphaned by a crash are expected; only a session that ends `completed` with dangling calls fails. |
| `assembly-budget-honesty` | Declared frame costs sum to `declared_total_tokens`, the sum fits `budget_tokens`, and a `reference` frame costs 0 — `SPEC.md` §B1/§B3 held at the point of assembly. |
| `staleness-at-use` | No frame rendered after its exact identity was last verified `stale` or `gone` ([`context-reuse.md`](./context-reuse.md) §4, V2, at the point of use). A later `valid` clears it; `unknown` does not convict. An honest refresh carries a new digest, so a new identity, and is invisible here. |
| `citation-at-use` | Every rendered frame carries a non-empty citation label (`SPEC.md` §F3 at the point of use, not only at the provider boundary). |
| `deterministic-composition` | An identical rendered frame set (identities, representations, order) composes to an identical `composition_digest` ([`context-reuse.md`](./context-reuse.md) §1 prefix stability). Skipped when the journal records no digests. |
| `effect-exactly-once` | No `effect_id` performed twice, with the evidence saying whether it was replayed across a `resume` or duplicated within one live run. |
| `resume-integrity` | A `resume` recovered exactly what the journal records: a `last_seq_seen` below the recorded prefix is quantified work loss, above it is a corrupt recovery. |

The suite is adversarial in the same way `contextgraph-conformance` is: the
crate ships a golden journal that passes everything and one
`fixtures/trip-<check>.ndjson` per check that trips **exactly** that check
while leaving every other one green (`contextgraph-trace/tests/fixture_suite.rs`).
An oracle that can only pass a healthy harness proves nothing.

## What this is not

The trace and oracles are the protocol-side substrate. A benchmark **runner** —
chaos scheduling that kills a harness mid-turn on purpose, scripted-model
cassettes, task generators, planted-fact retention probes — is a separate tool
that *produces* interesting journals for these oracles to judge. It is not in
this repository, and nothing on this page depends on whether it lands as an
internal tool or a public agent-agnostic runner.

The trace is also not attribution. `SPEC.md` §14 records the observable
stages `selected` → `rendered` → `cited` per frame to answer "was this context
*useful*?"; a journal's `prompt_assembled` entries are the `rendered` stage
observed host-side, so the two are joinable on the shared `FrameId`. But
attribution grades *usefulness per task* and the trace grades *invariants per
session*. Neither replaces the other.

## Open questions

These are why the crate is still `0.x` and still `-sketch`:

- **Probe vocabulary.** Planted-fact retention probes (insert a fact early,
  require it late, measure survival across compaction) need `compaction` and
  probe events. They are left out until a runner exists to plant them, and are
  additive when it does.
- **Resume shortfall severity.** A `resume` that recovered less than the
  journal records currently *fails* `resume-integrity`. There is an argument
  for a measured-warning tier instead — the harness recovered honestly, just
  lossily. The oracle reports the exact loss either way.
- **A JSON Schema for the journal.** `schema/contextgraph-envelope.schema.json`
  is the precedent for machine-checking examples; the trace envelope should get
  the same treatment before a stable `TRACE_FORMAT` is declared.
