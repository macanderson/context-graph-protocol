# The Context Graph Protocol, explained simply

This is the one doc to read before touching this codebase. It has three parts:

1. **[Principles](#1-principles)** — the rules we follow when we write code here.
2. **[Schema](#2-schema)** — the shapes of data the protocol sends around, and the Rust types that match them.
3. **[ADRs](#3-decision-log-adrs)** — the decisions that got us here, indexed and summarized.

If you only read one section, read Principles. Everything else follows from it.

---

## 0. What is this thing, in one paragraph?

Imagine a chat app (the "host") wants to answer a question using your code files,
your notes, and your memory. Instead of dumping raw text into the prompt, it asks
a small helper program (a "provider") for **frames**: little labeled packages of
context that say what they are, where they came from, how much they cost, and
whether you're allowed to send them elsewhere. The **Context Graph Protocol (CGP)**
is the shared language hosts and providers use to talk. It does **not** handle
running tools or taking actions — that's [MCP](https://modelcontextprotocol.io/)'s
job. CGP only handles fetching context, honestly.

---

## 1. Principles

These are the rules that shape every line of code and every design decision here.
Each one is enforced by a real type or a real test — not just a sentence in a
style guide.

1. **Every frame tells the truth about itself.** A frame always says: what kind of
   thing it is, where it came from, what it costs, when it was true, and how to
   cite it. No bare blobs of text.

2. **Budgets are checked, not trusted.** A provider says "this costs N tokens."
   The host recomputes that cost from the real bytes of content
   (`tokens = ceil(bytes / 4)`, exact match, no wiggle room). If the numbers
   don't match, the frames get dropped. See [ADR 0003](#adr-0003).

3. **Data doesn't leave the machine without a human saying yes.** A provider that
   sends data somewhere else (a cloud API, a third-party index) cannot be used
   until a person explicitly consents. That consent is recorded, not assumed.

4. **"Conformant" is a test you run, not a claim you make.** We ship a test suite
   (`contextgraph-inspect`) that actively tries to break each rule above. A
   provider is only conformant if it survives all of them, on every run.

5. **Every frame has a human-readable citation.** Never just an ID. A person
   should be able to look at a frame and know where it came from without
   decoding anything.

6. **Old and new versions keep working together.** A version string looks like
   `contextgraph/MAJOR.MINOR`. Two sides only need to agree on the `MAJOR` part
   to talk. Adding an optional field is a small (`MINOR`) change. Removing or
   renaming a field is a big (`MAJOR`) change, because it can break someone.

7. **Unknown fields are ignored, never rejected.** If a message has a field you
   don't recognize, skip it — don't error out. This is what lets the protocol
   grow without forcing everyone to upgrade on the same day.

8. **A frame's content is evidence, not a command.** The `content` field is
   untrusted data from somewhere else. Code that builds a prompt must fence or
   quote it — the same way an email client keeps the message body separate from
   the headers. Never treat frame content as instructions to execute.

9. **Providers never crash on bad input.** They reply with a structured `error`
   message (a machine-readable `code` plus a human `message`). If a code is
   unrecognized, treat it as `internal` — the safe, conservative default, not
   "safe to retry."

10. **The protocol stays small.** CGP only does context retrieval. It does not
    grow into tool-calling, task orchestration, or app-specific features. See
    [ADR 0007](#adr-0007) for why "the big app-specific bundle" and "the small
    protocol frame" are kept strictly separate.

### Contribution conventions (the short version)

- Commit messages: [Conventional Commits](https://www.conventionalcommits.org/),
  scoped to the crate you touched (e.g. `feat(contextgraph-types): ...`).
- Sign off commits (`git commit -s`) — this is DCO, not a CLA. You keep your
  copyright.
- One logical change per PR. CI must be green (`fmt`, `clippy -D warnings`,
  `test`). Include a test that proves the change (a "witness"), or say why one
  isn't possible.
- Update docs in the same PR as the code change.
- Dual-licensed MIT OR Apache-2.0.
- This repo does not host a public website. Don't invent new hosted URLs — only
  link to `raw.githubusercontent.com` or the paths the public site actually
  syncs from this repo. See [ADR 0008](#adr-0008).

---

## 2. Schema

CGP has two layers: the **core wire protocol** (frozen at `contextgraph/1.0`) and
an **optional profile** for longer-lived memory (`context-exchange-provider`,
still evolving). The machine-readable versions of both live in
[`schema/`](../schema/):

- [`schema/contextgraph-envelope.schema.json`](../schema/contextgraph-envelope.schema.json) — the core wire messages.
- [`schema/contextgraph-lifecycle-record.schema.json`](../schema/contextgraph-lifecycle-record.schema.json) — the optional memory profile.

The full normative text is [`SPEC.md`](../SPEC.md). What follows is the plain-English map.

### 2.1 Core wire messages (the envelope)

Every message on the wire is one JSON object with a `type` field:

| `type` | Sent by | What it means |
|---|---|---|
| `handshake` | host → provider | "Here's the protocol version I speak." |
| `handshake_ack` | provider → host | "Here's who I am and what I can do." |
| `query` | host → provider | "I need context: here's my goal, my budget, and what kinds of frames I want." |
| `frames` | provider → host | "Here are the frames, and whether I had to cut anything for budget." |
| `verify` | host → provider | "Are these frames I'm holding onto still true?" |
| `verified` | provider → host | "Here's the verdict for each: valid / stale / gone / unknown." |
| `shutdown` | either | "Close down cleanly." |
| `error` | either | "Something went wrong" — a machine `code` plus a human `message`. |

### 2.2 The frame — the core unit of exchange

A **frame** ("ContextFrame") is one piece of context. It always has:

- **`kind`** — one of: `snippet`, `symbol`, `fact`, `doc`, `memory`, `episode`, `graph`.
- **`title`** — short human label.
- **content or a reference to fetch content** — see representations below.
- **relevance score** — 0 to 1, how well it matches the query.
- **token cost** — honest, checked against real bytes.
- **provenance** — where it came from (file, line range, hash).
- **temporal validity** — when this was true (so a query can "rewind time").
- **citation label** — human-readable, always present.
- **relations** — optional graph edges to other frames.

A frame can carry its content three ways (see [ADR 0005](#adr-0005)):

- **full** — the whole thing, inline.
- **compact** — a shrunk/summarized version inline, plus a way to fetch the original.
- **reference** — just a pointer, no content at all, fetched only if needed.

### 2.3 Rust types (`contextgraph-types` crate)

These are the Rust types that implement the schema above. If you're reading
code, start here.

**Frames and content**
- `FrameKind` — the 7 kinds of frame (snippet, symbol, fact, doc, memory, episode, graph).
- `ContextFrame` — the main frame type; the unit of exchange.
- `FrameId` — a frame's stable identity (provider id + frame id + content hash) — used for dedup and stable ordering.
- `Representation` — full / compact / reference (see 2.2).
- `ContentFidelity` — how faithful the content is: exact / normalized / summarized / omitted.
- `InlineContentRequirement` — whether a use case needs content inline or can accept a reference.
- `ContentRef` — a handle for fetching a compact/reference frame's real content later.
- `Transform` — records what shrinking/summarizing was done to make a compact frame.
- `Provenance` — one "where this came from" entry.
- `Relation` — one graph edge from a frame to another thing.
- `FrameEmbedding` — optional vector-embedding info on a frame.

**Providers and handshake**
- `ProviderInfo` — a provider's name, version, and data-flow declaration.
- `DataFlow` — what a provider does with data it receives (reads / writes / egress) — the security-relevant consent input.
- `Capabilities` — what a provider says it can do at handshake time.
- `QueryCapability` — which frame kinds a provider serves.
- `EgressScope` — where a provider's data may travel: local-only, org-tenant, third-party-index, third-party-model, or custom.
- `Grantor` — who granted a consent receipt (a human, etc.).
- `ConsentReceipt` — the audit record proving a person agreed to let content leave the machine.

**Queries and errors**
- `ContextQuery` — a request for frames (goal, keywords, kinds wanted, budget, as-of time).
- `ContextQueryResult` — the response wrapper (frames + whether truncated).
- `ErrorCode` — machine-readable error codes (`bad_request`, `unsupported_kind`, `budget_unsatisfiable`, ...).
- `HostReaction` — what a host should do in response to a given error code (retry, drop provider, etc.).

**Verification and accounting**
- `VerifyRequest`, `VerifyResponse`, `Verdict`, `FrameVerdict` — the "is this frame still true?" exchange.
- `ServedFrame`, `ProviderUsage`, `UsageReport` — a per-request roll-up of which frames were served at what cost, so a bill traces back to exact frames.
- `ContextUse`, `AttributionReport` (in `attribution.rs`) — did a frame get selected, actually shown to the model, and actually cited? Separate from cost — this answers "did it matter?"
- `token.rs` — the token-cost formula (`ceil(bytes / 4)`).

**The optional lifecycle-record profile** (durable memory, not part of frozen 1.0 core — see [ADR 0006](#adr-0006) and the [profile doc](./profiles/context-exchange-provider.md)):
- `ContextRecord`, `RecordBody` — an immutable, hash-addressed note: one of 12 kinds
  (`observation`, `knowledge`, `memory`, `directive`, `record_proposal`,
  `evidence`, `artifact_contract`, `contract_validation`, `outcome_assessment`,
  `promotion_event`, `context_use`, `context_use_feedback`).
- `RecordStatus` — active / retracted / archived.
- `SharingScope`, `RecordScope` — who a record applies to, and who it's shared with.
- `OriginClass`, `RecordProvenance`, `RecordLink`, `RecordAttestation` — where a record came from and how it's tied to other records.
- `KnowledgeKind` (fact / assumption / decision), `DirectiveKind` (preference / rule / constraint / procedure), `ConstraintEffect`, `Enforcement`.
- `ValidationOutcome`, `ContractRequirement`, `RequirementResult` — pass/fail results for a validated artifact.

---

## 3. Decision log (ADRs)

Full text lives in [`docs/adr/`](./adr/). Every entry below is **Accepted**.
Read the full ADR before changing anything it covers.

| # | Title | One-line takeaway |
|---|---|---|
| [0002](./adr/0002-request-correlation-and-the-json-rpc-question.md) | Request correlation, and the JSON-RPC question | We were never JSON-RPC — added an optional `id` field so replies can be matched to requests, and fixed the docs to stop claiming otherwise. |
| [0003](./adr/0003-canonical-token-accounting.md) | Canonical token accounting | Token cost is now a fixed formula checked against real bytes, so a provider can't just claim a low number and get away with it. |
| [0004](./adr/0004-dead-capability-surface.md) | Dead capability surface | Removed `upsert`, `subscribe`, and `filters` from the handshake — they had no code behind them. Kept `writes`, which answers a real, separate consent question. |
| [0005](./adr/0005-frame-representations.md) | Frame representations | Frames can now be `compact` (shrunk, with a way to fetch the original) or `reference` (pointer only), not just `full` — old-style full frames still work unchanged. |
| [0006](./adr/0006-prompt-ingestion-as-a-local-provider.md) | Prompt ingestion as a local provider | Text a user pastes into chat is now treated like any other provider's output: split, classified, budgeted, and hashed — no more free pass around the rules. |
| [0007](./adr/0007-protocol-product-boundary.md) | The protocol/product boundary | Drew a hard line between the protocol's small atomic frame and one downstream app's much bigger task-specific bundle — they were both sloppily called "ContextFrame" before this. |
| [0008](./adr/0008-deploy-topology-and-advertised-urls.md) | Deploy topology and advertised URLs | This repo does not run a website (a separate repo, `cgp-website`, does) — nailed down which host serves what, so we stop advertising broken links. |
| [0009](./adr/0009-adopt-standing-decisions-scr-corpus.md) | Standing decisions as a Steering Context Record corpus | The maintainer's recurring directives to coding agents live in `docs/scr/` as one versioned corpus, identical across the org's repos, instead of being retyped every session. |
| [0010](./adr/0010-provenance-attestation.md) | Provenance attestation | A digest only proves nothing changed since someone wrote it down. A frame's provenance now folds into a signed hash chain bound to the frame's identity, so a third party can check a citation offline. |
| [0011](./adr/0011-open-frame-kind-vocabulary.md) | `FrameKind` is an open vocabulary | A provider can name a frame kind this crate has never heard of and the frame still parses, so a new kind is an additive change rather than a wire break. |
| [0017](./adr/0017-record-hash-and-record-attestation.md) | `record_hash` and `RecordAttestation` | The record layer's identity, implemented: RFC 8785 canonicalization with the record's own hash removed from the preimage, and a domain-separated Ed25519 signature over it. Says why JCS is right here and wrong at the frame layer. |

<a id="adr-0002"></a><a id="adr-0003"></a><a id="adr-0004"></a><a id="adr-0005"></a><a id="adr-0006"></a><a id="adr-0007"></a><a id="adr-0008"></a><a id="adr-0009"></a><a id="adr-0010"></a><a id="adr-0011"></a><a id="adr-0017"></a>

---

## Where to go next

- Building a provider? → [`implementing-a-provider.md`](./implementing-a-provider.md)
- Want to see it work? → [`reference-providers.md`](./reference-providers.md)
- Proving conformance? → [`running-conformance.md`](./running-conformance.md)
- The full normative spec → [`../SPEC.md`](../SPEC.md)
- How the protocol is maintained → [`../GOVERNANCE.md`](../GOVERNANCE.md)
