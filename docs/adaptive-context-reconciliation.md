# Adaptive-context ↔ protocol reconciliation (issue #27)

**Status:** delta table for [#27](https://github.com/macanderson/context-graph-protocol/issues/27).
Anchored on [ADR 0007 — the protocol/product boundary](./adr/0007-protocol-product-boundary.md).

## What this reconciles

An "adaptive-context" spec bundle was merged into **stella**
(`docs/design/context-frame-spec.md`, `directive-schema.md`,
`adaptive-context-lifecycle.md`, and a build prompt addressed at this repo) and
**oxagen-platform** (`docs/specs/adaptive-context/`, PR #1074). Those documents
restate frame semantics this repo owns. This table classifies every material
delta as one of:

- **adopt-upstream** — belongs in CGP; lands as a spec change now or is folded
  into a normative-track issue.
- **push-downstream** — a host/product concern; stays in stella (or an optional
  provider), which references CGP rather than restating it.
- **reject** — contradicts a settled CGP decision; recorded with rationale so it
  does not resurface.

The single reframe that resolves most of the drift: the bundle's `ContextFrame`
is the **task-wide aggregate** (`CompiledContextFrame`, host-owned), not the
protocol's **atomic** `ContextFrame`. See ADR 0007 §1.

## Method

Three source sets were extracted claim-by-claim and diffed: the stella bundle
(`context-frame-spec.md` = FS, `directive-schema.md` = DS, the CGP build prompt
= BP); the current CGP normative surface (`SPEC.md`, `docs/protocol-surface.md`,
`docs/adr/0005`, `schema/contextgraph-envelope.schema.json`, `contextgraph-types`);
and oxagen's `docs/specs/adaptive-context/` (PR #1074). oxagen's authoritative
position already defers upstream ("mechanism in the protocol; policy in the
host") and already marks its 6-kind drafts superseded; its remaining drift is
holding editable local copies plus CGEP naming.

## A. Frame model & identity

| # | Item | Bundle says | CGP position | Class | Destination |
|---|---|---|---|---|---|
| A1 | **`ContextFrame` identity** | Task-wide aggregate: task, entity_bindings, 6 directive buckets, bi_temporal_memories, observations, contracts, code_map, excluded, citations, FrameMetadata. | Atomic retrieval envelope; the aggregate is host-owned `CompiledContextFrame`. | **push-downstream** | ADR 0007 §1. stella keeps the aggregate under its own name; it already imports the atomic `ContextFrame` from `contextgraph-types`. |
| A2 | **"Six questions a frame answers"** (FS §1) | Normative framing of the frame. | CGP frames a *five-guarantee* posture ("what it is, where it came from, what it costs, when it was true, how to cite it"); no "six questions." | **push-downstream** | Host/product narrative; stays in stella docs. Not a wire contract. |
| A3 | **8 design principles** (bounded/typed/provenance-native/time-aware/policy-first/inspectable/self-correcting/privacy-bounded) | Frame properties. | Overlap CGP's seven guarantees but include host concerns (policy-first, self-correcting, inspectable = host compilation). | **push-downstream** | stella narrative; the wire-relevant subset (provenance, budget, temporal) is already CGP's guarantees. |
| A4 | **9-source system model** (task/state/contract/directives/memory/observations/code-map/evidence/entities) | How the frame is assembled. | Assembly is host compilation. | **push-downstream** | stella. |
| A5 | **`FrameMetadata` policy versions** (selection/authorization/promotion policy versions, governance_mode) | Required frame metadata. | Policy versioning is a host compilation artifact. | **push-downstream** | stella (`CompiledContextFrame` metadata). |
| A6 | **`selection_reasons`, `excluded[]` with reason enum, `citations[]`** | Frame fields. | Selection/exclusion/citation bookkeeping is host compilation output. | **push-downstream** | stella. (CGP has usage reports U1 + citation *labels* per atomic frame; different layer.) |
| A7 | **Frame representations** full/compact/reference, `content_fidelity`, `canonical_content_hash`, `content_ref`, `transform`, `minimum_content_fidelity`, `inline_content_requirement` | Protocol should add these (BP). | **Already implemented** in CGP (ADR 0005, `frame.rs`, schema `allOf`, conformance). Bundle's model matches. | **adopt-upstream (already landed)** | Reconcile naming: BP's `content_hash` = CGP's `content_digest` (ADR 0005 §"two names"). Document in `SPEC.md`: issue **#49**. |
| A8 | **Frame identity triple** | BP: `record_id`/`lineage_id`/`record_hash` for records. | CGP: `FrameId {provider_id, frame_id, content_digest}`, opaque + provider-declared. | **adopt-upstream (already landed)** for frames; record identity is a #28-profile concern. | Frame identity is done (`identity.rs`, D1–D4). Record identity → **#28**. |

## B. Directives

| # | Item | Bundle says | CGP position | Class | Destination |
|---|---|---|---|---|---|
| B1 | **Directive as the core engine unit** (DS/FS) | The single typed unit of the context engine. | Not a frame concept; frame content is evidence, not a directive (SPEC §6/R3). | **push-downstream** | stella runtime (`stella-core::context_record`, live `DirectiveKind`). |
| B2 | **Six directive kinds** `memory\|fact\|rule\|preference\|constraint\|procedure` (DS/FS drafts) | Portable taxonomy. | Four portable kinds `preference\|rule\|constraint\|procedure`; `memory`/`fact` are separate record kinds. oxagen's own lifecycle spec already says "Memory is not a directive kind." | **reject** (as *portable* taxonomy) | ADR 0007 §4. Six-kind version is a host convenience; the portable directive record (if #28 defines one) is four kinds. |
| B3 | **Directive-as-record, 4 kinds** (BP) + subtype fields (`constraint_effect: require\|forbid`, ordered `procedure` steps, `enforcement`, `origin`) | Protocol should define a `directive` record. | No directive record yet; the atomic frame has none. Not foreclosed. | **adopt-upstream (issue)** | **#28** Context Exchange Provider profile — as one immutable record kind. Not the frozen 1.0 core. |
| B4 | **Directive lifecycle** (citation-stat pruning thresholds, precedence layers, promotion_status) | Engine behavior. | Pruning/promotion/precedence = host policy; protocol "carries the value, does not authorize it." | **push-downstream** | stella. `promotion_stage` explicitly stays out of any portable `Directive` (BP's own rule). |
| B5 | **Directive `status` enum** `active\|stale\|superseded\|archived` (FS) vs `active\|superseded\|archived` (DS) | Stored status. | Record status is `active\|retracted\|archived` (superseded is derived from lineage). | **adopt-upstream (issue)** / reconcile | **#28**. Host may keep richer internal statuses; wire status is the three-value record status. |

## C. Temporal, tokens, provenance

| # | Item | Bundle says | CGP position | Class | Destination |
|---|---|---|---|---|---|
| C1 | **Temporal field names** `as_of_valid_at` / `as_of_observed_at` (FS) | Frame/query temporal fields. | CGP uses `valid_from` / `valid_to` / `recorded_at` on frames + `as_of` on queries. BP *also rejects* the `as_of_*` names. | **reject** (the bundle's field names) | CGP names win. stella maps its internal bi-temporal store to CGP names on the wire. |
| C2 | **Half-open `[from, until)` intervals + `known_at`/`valid_at` point queries** (BP) | Protocol temporal semantics. | CGP temporal fields exist but are free-form strings (no RFC 3339 validation, no `as_of` probe). | **adopt-upstream (issue)** | **#10** (validate temporal fields as RFC 3339, probe `as_of`). Decide half-open + `known_at`/`valid_at` naming there. |
| C3 | **`token_cost` / `canonical_token_cost` / `tokenizer_ref`** (BP) | Protocol token fields; wire cost optional, host computes. | `token_cost` **already normative & required** (B3: `ceil(utf8_bytes/4)`, ADR 0003). `canonical_token_cost`/`tokenizer_ref` already in the type. | **adopt-upstream (already landed)** | B3 for compact/reference frames (reference frame cost) is open: **#50**. |
| C4 | **`token_budget` / `token_estimate`** (FS) | Frame budgeting fields. | Budgeting/allocation is a host concern; CGP carries per-frame `token_cost`, not a budget. | **push-downstream** | stella (`CompiledContextFrame` budgeting). |
| C5 | **Provenance schema, `content_hash` vs `canonical_content_hash` golden vectors, `RecordAttestation`** (BP) | Protocol provenance + detached attestation. | Provenance digest format is normative-*grammar* only (`sha256:<64 hex>`); no byte-match verification; no attestation. | **adopt-upstream (issue)** | **#12** (digest format + host-side provenance verification). Attestation → **#28** profile. |

## D. Lifecycle / records / operations (the exchange layer)

| # | Item | Bundle says (BP) | CGP position | Class | Destination |
|---|---|---|---|---|---|
| D1 | **`ContextRecord` 12-kind taxonomy** (observation, knowledge, memory, directive, record_proposal, evidence, artifact_contract, contract_validation, outcome_assessment, promotion_event, context_use, context_use_feedback) + canonical envelope | Add to the protocol. | Not in CGP; CGP is frame-retrieval-only today. This is a whole new layer. | **adopt-upstream (issue)** | **#28** profile. The *record schemas* are portable; their *execution/promotion/validation* is host (push-down). |
| D2 | **`context/records/append`** (batch, idempotency ledger, retention negotiation) | Write path. | `Capabilities.upsert` is a dead bool (no envelope/API). #5 recommends drop-and-defer. | **adopt-upstream (issue)** | **#5** — BP's append is the concrete write-path design that unblocks #5's "specify or drop." |
| D3 | **`context/records/get`** (by exact `record_id`) & **`context/resolve`** (opaque `content_ref`, verify canonical hash, typed resolve failures) | Read/resolve path. | `Capabilities.resolve` advertised but no envelope/API; reference frames un-rehydratable. #50 open. | **adopt-upstream (issue)** | **#50** — BP's resolve + failure taxonomy is the design #50 asks for. |
| D4 | **Capability negotiation** under `cgep/lifecycle/1.0-draft` (representations, `known_at`, resolve, record kinds, operations, limits, retention, consent) | Add capabilities. | CGP has handshake capabilities; no lifecycle profile. | **adopt-upstream (issue)** — **naming normalized** to `contextgraph/lifecycle/1.0-draft`. | **#28**. |
| D5 | **28 typed error codes** (unsupported_capability, invalid_record, idempotency_conflict, retention_rejected, partial_failure, …) | Add. | CGP §9 has a 6-code table + open vocab (X1/X2). | **adopt-upstream (issue)** | Frame/query errors → **#49** (add `unsupported_representation`, version-mismatch code). Record/append/resolve errors → **#28**/#5/#50. |
| D6 | **`ArtifactContract` + `ContractValidation` records** (10-kind requirement validator, `command` needs `execution_approval_ref`) | Portable records. | Absent. Execution/judging is explicitly host. | **adopt-upstream (schema, issue)** / **push-downstream (execution)** | Record *schemas* → **#28**; contract *execution* + semantic judging stay in the host. |
| D7 | **`OutcomeAssessment`, `PromotionEvent`, `ContextUse`, `ContextUseFeedback`, `RecordProposal`** | Portable records. | `ContextUse`/feedback overlap CGP's usage reports (U1). Promotion/proposal are host decisions recorded as immutable events. | **adopt-upstream (schema, issue)** | **#28**. Keep policy (when to promote, thresholds) host-side. |
| D8 | **`subscribe` / staleness push** | (BP is pull-based; leans on verify.) | `Capabilities.subscribe` is a dead bool; #6 recommends drop-and-defer; freshness in 1.0 is pull `context/verify`. | **push (defer)** | **#6** — no change to the recommendation; noted for completeness. |

## E. Scope / naming / rejections

| # | Item | Bundle says | CGP position | Class | Destination |
|---|---|---|---|---|---|
| E1 | **Rename to "Context Graph Exchange Protocol / CGEP"**, `cgep/1.0-draft` namespace, `context-graph-exchange-protocol` repo (BP §naming; oxagen lifecycle §23; rationale: "AgentSpeak uses Context Graph Protocol") | Rename the protocol. | Canonical name is **Context Graph Protocol (CGP)**; wire `contextgraph/1.0-draft`; stem `contextgraph`. Owner confirmed 2026-07-23. | **reject** | ADR 0007 §5. Every adopted BP item is normalized to CGP naming. |
| E2 | **Portable `project_id` in scope** (DS draft) | Add to portable scope. | BP itself forbids it ("do not add `project_id` to the portable core until there is a cross-provider registry contract"); oxagen marks the draft superseded. | **reject (defer)** | Not portable until a registry contract exists. Host may key on project internally. |
| E3 | **9-key `Scope`** (tenant/org/workspace/project/repo/env/session/task/user) (FS) vs BP's 7-key portable scope + `sharing_scope` | Frame scope. | CGP query scope differs; portable record scope belongs to the profile. | **adopt-upstream (issue)** / reconcile | **#28** defines the portable scope (7-key + `sharing_scope`, conjunctive); drop `tenant_id`/`project_id` from portable core. |
| E4 | **`context/propose`, `context/promote`, `context/validate` operations** | (BP explicitly says do **not** expose these.) | Agree — policy-executing operations are host-only; the protocol records decisions after the host makes them. | **reject** | Recorded as a boundary invariant (ADR 0007 §3). |

## Disposition summary

- **Adopt now (this PR):** ADR 0007 (boundary) + this delta table, posted to #27.
  Confirms A7/A8/C3 are *already upstream*.
- **Fold into normative-track issues** (design input = the build prompt; each
  issue updated with a pointer to this table): **#49** (SPEC.md completeness:
  representations, verify, identity, `unsupported_representation`), **#28**
  (exchange-provider profile: record taxonomy, capabilities, scope,
  attestation, artifact/outcome/promotion/use records), **#5** (append write
  path), **#50** (resolve + B3 for reference frames), **#12** (digest
  format + provenance verification), **#10** (RFC 3339 temporal + `as_of`
  probe). **#6** unchanged (defer subscribe).
- **Push downstream** (host owns; downstream docs reference CGP): A1–A6, B1, B4,
  C4, and the compilation/learning/governance machinery.
- **Reject** (with rationale, so it does not resurface): E1 (CGEP rename), B2
  (six portable directive kinds), C1 (`as_of_*` field names), E2 (portable
  `project_id`), E4 (policy-executing operations).

## Enforcement (so it does not re-drift)

The structural guarantee is that the normative frame text lives in exactly one
place (`SPEC.md` + schema); downstream docs hold only a pinned pointer
(`NORMATIVE-HOME:` header naming this repo + the pinned rev they consume). The
downstream **canary CI** (issue #29) builds stella and the oxagen copy against
this repo's HEAD, catching code-level drift before the freeze.
