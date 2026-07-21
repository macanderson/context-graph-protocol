# Context Receipt Impact Trace Design

**Status:** Approved for implementation
**Approved:** 2026-07-21
**Product surface:** Context Graph Protocol microsite
**Future host surface:** Stella Observatory and task completion UI

## Purpose

The Context Receipt makes the usefulness of a compiled context frame visible at
the level of one completed task. It answers six questions without presenting
agent self-report as proof:

1. What context entered the frame?
2. Why was each item selected?
3. Where was an item observed influencing the work?
4. Which decision, action, or verification result was connected to it?
5. What outcome evidence supports or challenges the attribution?
6. What lifecycle action, if any, should follow?

The microsite demonstrates the concept through a concrete brand-kit task. The
production Stella feature is intentionally outside this site change.

## Architectural boundary

The proposed optional Context Graph Protocol lifecycle capability can exchange
immutable `context_use` and `context_use_feedback` events when a provider
advertises that capability. These events are not part of the currently shipped
1.0 retrieval wire. The protocol does not calculate usefulness or prescribe
product UI.

Stella owns:

- frame compilation and exact use instrumentation;
- structured post-task reflection;
- outcome attribution and trust weighting;
- retrieval-gap detection;
- the derived Context Receipt view;
- user correction and lifecycle policy.

Agent self-reflection is an evidence source, not an authority. It records a
short observable influence claim and never requests or stores hidden
chain-of-thought.

## Canonical data contract

`context_use` continues to record the independently observable stages
`selected`, `rendered`, and `cited`. An influence claim does not become a new
`use_kind`; it belongs to `context_use_feedback` because influence is an
assessment.

The microsite fixture preserves that separation: a used receipt item contains
one nested `context_use` record and a collection of immutable
`context_use_feedback` events. An unevaluated use has an empty event collection
rather than a feedback-shaped object with null fields. A missing-context item
is a separate observation and contains neither record. Display status and label
are never fixture fields; one pure ordered projector derives them.

Method trust, confidence thresholds, and the known-use registry are host-owned
projection inputs, not producer-authored feedback properties. Contradiction is
derived across eligible feedback history: helpful and not-helpful events tied
to the same use trace and outcome assessment make the display contested and
therefore `Unknown`. A feedback producer cannot mark its own method trusted or
assert that no contradictory feedback exists.

The projector returns a structured display projection: one status, an optional
detail, and at most one eligible assessment. Contested history returns
`Unknown` plus `Contested` and no selected assessment. Self-report-only history
returns `Unknown` plus `Reported influence` and no verified assessment. The UI
must not bypass this result by independently selecting the last raw event.

Each feedback event includes:

```json
{
  "context_use_id": "use_brand_contract_01",
  "outcome_assessment_id": "outcome_brand_kit_07",
  "influence_stage": "planning",
  "influence_statement": "Created a required-file checklist from the brand-kit contract.",
  "had_opportunity": true,
  "observable_effect_refs": [
    "plan_step_04",
    "validation_brand_kit_07"
  ],
  "outcome_relation": "supported",
  "evaluation": "helpful",
  "evaluation_method": "deterministic_validation",
  "attribution_confidence": 94
}
```

`influence_stage` values are:

- `planning`;
- `execution`;
- `verification`;
- `final_response`;
- `none`.

`outcome_relation` values are:

- `supported`;
- `contradicted`;
- `unrelated`;
- `unknown`.

`evaluation` remains `helpful`, `not_helpful`, or `neutral`. An unevaluated use
has no feedback event; `unknown` is not added as a fourth evaluation value.

`influence_statement` is a post-task summary of an observable effect, limited
to 500 Unicode scalar values. A producer must not request hidden reasoning or
chain-of-thought to populate it. Validators enforce the length and record shape;
they do not claim to detect chain-of-thought semantically.

Core `evaluation_method` values are extensible and initially include:

- `deterministic_validation`;
- `explicit_user_feedback`;
- `external_outcome`;
- `accepted_repository_state`;
- `controlled_comparison`;
- `trace_correlation`;
- `agent_self_report`.

Unregistered methods remain inspectable but never contribute to automatic
pruning. An extension becomes eligible only after it has a registered or
versioned identifier and host policy explicitly trusts it; at that point it is
a recognized method rather than an unknown method.

## Attribution rules

Evidence strength descends in this order:

1. controlled comparison or deterministic contract, test, or policy result;
2. explicit user confirmation or correction;
3. externally confirmed outcome or accepted repository state;
4. trace-supported correlation across citation, decision, action, and outcome;
5. agent self-report without corroboration.

The following invariants are mandatory:

- A failed task does not make every selected context item unhelpful.
- `not_helpful` requires `had_opportunity = true` and at least one supporting
  or challenging evidence reference, a recognized method, and valid confidence.
- `had_opportunity = false` requires `influence_stage = none`,
  `evaluation = neutral`, and `outcome_relation = unrelated` or `unknown`.
- `outcome_relation = supported` or `contradicted` requires an exact
  `outcome_assessment_id` and a nonempty observable-effect list.
- Agent self-report alone cannot create blocking steering, archive a confirmed
  directive, or count as verified causality.
- No observed use is not equivalent to no actual use and is never automatically
  negative feedback.
- Counterfactual claims are evidence only when produced by a controlled
  comparison. All other counterfactuals are explicitly labeled simulations.
- Every attribution remains correctable through a later immutable feedback
  event.

Automatic lifecycle policy uses only attributable projections. It counts at
most once per `use_trace_id` and `outcome_assessment_id` pair. The pruning
numerator contains only attributable `not_helpful` evaluations. The denominator
contains attributable `helpful`, `not_helpful`, and `neutral` evaluations after
the same trust, confidence, evidence, opportunity, and deduplication filters.
Unregistered-method, low-confidence, and agent-self-report-only evaluations are
excluded from both.

## Retrieval gaps

Missing context means required context did not reach the applicable execution
stage. Stella records the gap as an observation with `observation_kind =
missing_context`, never as `context_use_feedback`. Most gap kinds have no
`context_use`, but `not_rendered` may coexist with and reference an earlier
`selected` ContextUse because the item entered the compiled frame before it was
lost at rendering.

The observation distinguishes:

- `not_retrieved`: an applicable record existed but no provider returned it;
- `not_selected`: retrieval returned the item but ranking, conflict handling,
  or budgeting excluded it;
- `not_rendered`: the item entered the compiled frame but did not reach the
  applicable model or tool stage;
- `unavailable`: the requirement was not yet represented as durable context;
- `unknown`: evidence proves an omission but not which stage lost the item.

The observation references the compiled frame, task, outcome evidence, and the
expected record or requirement when one can be identified. This remains a
Stella observation subtype and does not expand the protocol's portable record
families.

Cross-field rules are:

- `not_retrieved`, `not_selected`, and `not_rendered` require
  `expected_context_record_id` because they assert that a durable record exists;
- `not_rendered` may carry `selected_context_use_id` and
  `selected_use_trace_id`; when selected-use telemetry is available it includes
  at least one, and every supplied reference must resolve identity-consistently
  to the earlier selected ContextUse, but it never creates ContextUseFeedback;
- `unavailable` requires a nonempty `expected_requirement` and has no
  `expected_context_record_id`;
- `unknown` requires at least one of `expected_context_record_id` or
  `expected_requirement`; and
- every missing-context observation has a nonempty evidence-reference list.

## Derived receipt statuses

Receipt statuses are product projections, not canonical event values. Stella
uses the named `receipt_display_min_attribution_confidence` policy floor, which
defaults to 80, and evaluates the rows below in order; the first matching row is
the single display status. Affirmative statuses require a trusted recognized
method, corroborating non-self-report evidence, confidence at or above the
display floor, and no contradictory eligible feedback.

| Display status | Derivation |
| --- | --- |
| Missing from frame | A `missing_context` observation identifies omitted applicable context or a new durable-context need. For `not_rendered`, retain this public umbrella label but show the required detail `Selected, not rendered`. |
| Got in the way | Attributable `not_helpful` feedback has a real opportunity, evidence, sufficient confidence, and no contradictory eligible feedback. |
| Confirmed the work | Attributable helpful feedback identifies verification-stage influence and supporting outcome evidence. |
| Moved the work | Attributable helpful feedback identifies a planning, execution, or final-response influence and an observable effect. |
| Selected, no observed use | The item was selected or rendered, no cited event or observed attribution exists by task completion, and no missing-context observation applies. |
| Unknown | Evidence is absent, contradictory, below `receipt_display_min_attribution_confidence`, or agent-self-report-only. A self-report-only claim is shown as `Reported influence` detail, not as proof. |

The UI does not collapse these statuses into a single usefulness score.

## Microsite experience

The new section appears immediately after the governed learning-loop section
and before the ecosystem architecture section. Its headline is:

> See which context changed the work—and which didn't.

It is labeled `Lifecycle prototype · not in the 1.0 retrieval wire` so the
demonstration cannot be mistaken for a currently shipped core-protocol feature.

The section uses one dominant Impact Trace. A compact list of frame items lets
the visitor select one item. The selected item reveals four aligned stages:

```text
context item -> observable decision -> tool or artifact action -> verified outcome
```

The initial Stella brand-kit example is useful on first render and shows:

- a brand-kit artifact contract that moved the work;
- a terminal-native identity directive that confirmed the work;
- a photography-treatment memory with no observed use;
- a generic constellation directive that got in the way;
- a reduced-motion requirement missing from the frame;
- a legacy spacing note whose impact is unknown.

The detail area shows the status as text, the evaluation method, attribution
confidence, the short influence statement, and evidence references. It also
states that the receipt stores observable attribution rather than
chain-of-thought.

The default server response contains the visible trace and curated assessment,
but not the raw structured receipt or private receipt field names. Structured
evidence is mounted only after the visitor explicitly opens the evidence
disclosure. Changing the selected item closes and clears that disclosure.

One optional control shows an illustrative path in which the required-file
contract is absent. The control is visibly labeled `Simulated counterfactual`,
and the panel states that it is an explanation rather than observed evidence.

## Interaction and accessibility

- Use native buttons for item selection and the simulation control.
- Express selection with `aria-pressed`; do not implement a partial tab
  pattern.
- Keep DOM and keyboard order identical to visual order.
- Give the Impact Trace a concise accessible name and a persistent textual
  description.
- Announce selected-item changes through one polite live region.
- Pair every color state with a text label and a distinct mark.
- Preserve visible focus indicators with at least 3:1 non-text contrast.
- Reflow the four trace stages into a vertical sequence on narrow screens.
- Avoid horizontal scrolling at 320 CSS pixels and 200 percent text zoom.
- Suppress nonessential transitions under `prefers-reduced-motion: reduce`.
- Do not autoplay, loop, flash, or require pointer hover.

## Error and uncertainty handling

- Missing evidence renders `Unknown`, not `Helpful` or `Not helpful`.
- Contradictory feedback is shown as contested and remains inspectable.
- A missing expected record ID may still produce an `unavailable` gap
  when a contract failure or explicit correction names the requirement.
- If client-side interaction is unavailable, server-rendered content still
  presents the default contract trace and the complete conceptual explanation,
  without serializing the raw structured receipt.

## Testing

Automated checks must prove:

- the deployed HTML includes the Context Receipt headline and default brand-kit
  trace;
- all six examples and their derived text statuses are present in built HTML;
- unit fixtures prove the ordered projector and reject untrusted,
  low-confidence, self-report-only, contradictory, and incomplete feedback;
- built HTML excludes raw receipt field names and default private fixture data;
- item selection uses native buttons with `aria-pressed`;
- the Impact Trace and simulation control have accessible names;
- the counterfactual is explicitly labeled simulated and not evidence;
- the page states that no chain-of-thought is stored;
- CSS includes responsive trace reflow, visible focus, and reduced-motion
  handling;
- the existing protocol narrative, GitHub links, and social metadata remain
  intact;
- the production build and full test suite succeed.

## Success criteria

A first-time visitor can determine within ten seconds that the receipt traces
an exact frame item to an observable consequence. They can also distinguish
verified outcome evidence, agent self-report, no observed use, and missing
context without relying on a magic score.

The demonstration strengthens the protocol story without implying that Context
Graph Protocol itself performs attribution or governance.
