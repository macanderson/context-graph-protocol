# Context Receipt Impact Trace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an accessible interactive Context Receipt to the Context Graph Protocol microsite that traces exact context items to observable decisions, actions, and outcomes through a realistic Stella brand-kit example.

**Architecture:** Keep the existing page as the composition root and add one focused client component with static fixture data and local selection/simulation state. Render receipt statuses as derived explanatory labels, preserve `context_use` and `context_use_feedback` terminology, and clearly label the section as a future lifecycle prototype rather than current 1.0 retrieval behavior.

**Tech Stack:** React 19, TypeScript 5.9, Vinext/Vite, plain CSS, Node test runner, Cloudflare Workers-compatible Sites deployment.

## Global Constraints

- Preserve the existing monochrome, editorial, electric-green visual system.
- Use lowercase snake_case for serialized protocol property names.
- `context_use` records `selected`, `rendered`, and `cited`; influence remains an assessment in `context_use_feedback`.
- Display statuses are projections, never canonical record values or a global usefulness score.
- Agent self-report is inferred evidence, never proof, hidden chain-of-thought, or pruning authority.
- Negative attribution requires a real opportunity and observable evidence.
- Missing context is an independent `missing_context` observation and never negative feedback about selected context.
- Counterfactual content is visibly labeled simulated and not evidence.
- Use native buttons, native document order, visible focus, text-plus-mark status encoding, 44px minimum touch targets, reduced-motion handling, and no horizontal overflow at 320 CSS pixels.
- Preserve the current package manager, lockfile, Cloudflare/Vinext architecture, social metadata, GitHub links, and private Sites deployment.
- Add no dependency, persistence, account requirement, external request, or background animation.

---

### Task 1: Add the failing Context Receipt contract tests

**Files:**

- Modify: `tests/rendered-html.test.mjs`
- Test: `tests/rendered-html.test.mjs`

**Interfaces:**

- Consumes: the existing `render()` helper and production worker output.
- Produces: a red test contract for the receipt content, source semantics, and CSS accessibility behavior.

- [ ] **Step 1: Extend the rendered-output test**

Add assertions after the governed-learning-loop assertion:

```js
assert.match(html, /See which context changed the work/);
assert.match(html, /Context Receipt/);
assert.match(html, /Impact trace/);
assert.match(html, /Lifecycle prototype/);
assert.match(html, /not in the 1\.0 retrieval wire/i);
assert.match(html, /Brand-kit delivery contract/);
assert.match(html, /Terminal-native identity/);
assert.match(html, /Moved the work/);
assert.match(html, /Simulated counterfactual/);
assert.match(html, /not observed evidence/i);
```

- [ ] **Step 2: Extend the source-contract test**

Read `app/_components/ContextReceiptDemo.tsx` and `app/globals.css`, then add:

```js
assert.match(page, /href="#receipt"/);
assert.match(receipt, /aria-label="Context Receipt items"/);
assert.match(receipt, /aria-label="Impact trace"/);
assert.match(receipt, /<button[\s\S]*?aria-pressed=[\s\S]*?aria-controls="context-impact-detail"/);
assert.match(receipt, /context_use_id/);
assert.match(receipt, /influence_stage/);
assert.match(receipt, /influence_statement/);
assert.match(receipt, /had_opportunity/);
assert.match(receipt, /observable_effect_refs/);
assert.match(receipt, /outcome_relation/);
assert.match(receipt, /evaluation_method/);
assert.match(receipt, /attribution_confidence/);
assert.match(receipt, /source_kind:\s*"missing_context"/);
assert.match(receipt, /context_use_id:\s*null/);
assert.match(receipt, /evaluation:\s*"not_helpful"/);
assert.match(receipt, /had_opportunity:\s*true/);
assert.match(receipt, /observable_effect_refs:\s*\[[^\]]+\]/);
assert.match(receipt, /derived statuses[\s\S]*not a usefulness score/i);
assert.match(receipt, /self-report[\s\S]*not proof/i);
assert.match(receipt, /Simulated counterfactual[\s\S]{0,1200}not observed evidence/i);
assert.match(receipt, /No chain-of-thought is stored/);
assert.match(css, /:focus-visible/);
assert.match(css, /\.receipt-trace/);
assert.match(css, /@media \(max-width: 760px\)/);
assert.match(css, /prefers-reduced-motion: reduce/);
```

Update the `Promise.all` bindings so `receipt` and `css` contain the new
component and stylesheet source.

- [ ] **Step 3: Run the focused test and verify RED**

Run: `npm test`

Expected: FAIL because `app/_components/ContextReceiptDemo.tsx` is absent or the
new receipt assertions do not match the current site.

- [ ] **Step 4: Commit the red test**

```bash
git add tests/rendered-html.test.mjs
git commit -m "test: specify Context Receipt microsite behavior"
```

### Task 2: Implement the Context Receipt component and page integration

**Files:**

- Create: `app/_components/ContextReceiptDemo.tsx`
- Create: `app/_components/contextReceiptProjector.mjs`
- Modify: `app/page.tsx`
- Test: `tests/rendered-html.test.mjs`

**Interfaces:**

- Consumes: no props and no external state.
- Produces: `export default function ContextReceiptDemo(): JSX.Element` and the `#receipt` navigation target.

- [ ] **Step 1: Define the exact fixture contract**

Use these TypeScript shapes:

```ts
type ReceiptStatus =
  | "moved"
  | "confirmed"
  | "unused"
  | "obstructed"
  | "missing"
  | "unknown";

type TraceStage = {
  label: "Context item" | "Decision" | "Observable action" | "Verified outcome";
  value: string;
  evidence_ref: string | null;
};

type ReceiptBase = {
  id: string;
  title: string;
  kind: string;
  record_revision: string;
  attribution_basis: string;
  trace: readonly TraceStage[];
};

type ContextUse = {
  context_use_id: string;
  use_trace_id: string;
  context_record_id: string;
  use_stage: "selected" | "rendered" | "cited";
  selected: boolean;
  rendered: boolean;
  cited: boolean;
  telemetry_complete: boolean;
  selection_reason: string;
};

type ContextUseFeedback = {
  feedback_id: string;
  context_use_id: string;
  use_trace_id: string;
  evaluation: "helpful" | "not_helpful" | "neutral";
  evaluation_method: string;
  attribution_confidence: number;
  influence_stage: "planning" | "execution" | "verification" | "final_response" | "none";
  influence_statement: string;
  had_opportunity: boolean;
  observable_effect_refs: string[];
  outcome_relation: "supported" | "contradicted" | "unrelated" | "unknown";
  outcome_assessment_id: string | null;
};

type UsedReceiptItem = ReceiptBase & {
  source_kind: "context_use";
  context_use: ContextUse;
  context_use_feedback_events: ContextUseFeedback[];
};

type MissingReceiptItem = ReceiptBase & {
  source_kind: "missing_context";
  observation_kind: "missing_context";
  missing_context_kind: "not_retrieved" | "not_selected" | "not_rendered" | "unavailable" | "unknown";
  detection_method: string;
  expected_context_record_id: string | null;
  evidence_refs: string[];
  selected_context_use_id?: string | null;
  selected_use_trace_id?: string | null;
};

type ReceiptItem = UsedReceiptItem | MissingReceiptItem;
```

Create six items with these exact titles and labels:

```text
Brand-kit delivery contract -> Moved the work
Terminal-native identity -> Confirmed the work
Photography treatment -> Selected, no observed use
Generic constellation mark -> Got in the way
Reduced-motion behavior -> Missing from frame
Legacy spacing note -> Unknown
```

Use this exact observable story:

| Item | Context | Decision | Action | Outcome | Basis |
| --- | --- | --- | --- | --- | --- |
| Brand-kit delivery contract | `Artifact contract · revision 03` | Create a required-file checklist before generation. | Produced the wordmark, mark, theme variants, manifest, and social preview. | All required deliverables passed the bound artifact contract. | Direct artifact and validation references · confidence 94/100 |
| Terminal-native identity | `Directive · revision 03` | Keep the `> Stella` terminal lockup already present in the plan. | Applied the terminal lockup consistently across the brand sheet and exports. | User-confirmed direction remained intact in accepted artifacts. | Explicit user feedback · confidence 91/100 |
| Photography treatment | `Memory · episode 07` | No linked decision. | No observable action. | Not evaluated. | Adequate telemetry found no effect references |
| Generic constellation mark | `Directive · revision 01` | Start from a constellation-shaped symbol. | Generated a generic first draft that was later replaced. | User correction and accepted diff contradicted the direction. | Explicit correction plus accepted repository state · confidence 88/100 |
| Reduced-motion behavior | `Missing context · not_selected` | No linked planning requirement. | Animated spinner was initially generated without a reduced-motion state. | Deterministic validation found the omission before delivery. | Missing-context observation, not negative use feedback |
| Legacy spacing note | `Knowledge · revision 04` | Trace unavailable. | Trace unavailable. | Evidence incomplete. | Incomplete execution telemetry; no impact claim |

Stable example references are `plan-checklist-04`, `brand-manifest.json`,
`validation-07`, `decision-terminal-03`, `brand-sheet.svg`,
`correction-02`, `accepted-diff-08`, and `validation-reduced-motion-05`.

Define `simulatedTrace` with these four values and no evidence references:

```text
Context item -> Required-file contract absent
Decision -> No explicit deliverable checklist
Observable action -> Wordmark and social preview omitted
Verified outcome -> Illustrative validator failure
```

The first item is the default. The missing item uses `source_kind:
"missing_context"`, `observation_kind: "missing_context"`,
`missing_context_kind: "not_selected"`, `detection_method:
"deterministic_validation"`, and missing-context evidence rather than a
ContextUse or ContextUseFeedback.

Use `evaluation_method: "deterministic_validation"` for the contract,
`evaluation_method: "explicit_user_feedback"` for the confirmed terminal
directive and obstructing constellation directive. Use an empty
`context_use_feedback_events` collection when the use was not evaluated. The
confirmed terminal directive uses `influence_stage: "verification"`. Every
supported or contradicted feedback event supplies `outcome_assessment_id`.

Feedback producers do not author method trust or contradiction booleans. Pass
trusted evaluation methods and the display-confidence floor as host-owned
projection policy. Project over the immutable feedback-event collection and
derive contested outcomes when eligible helpful and not-helpful events refer to
the same use trace and outcome assessment. A later eligible contradiction
therefore resolves to `Unknown`.

Do not put `status` or `status_label` in fixture data. Implement one pure,
unit-tested `projectReceipt` function in
`contextReceiptProjector.mjs`. Apply the ordered status rules and the confidence,
method trust, opportunity, evidence, outcome-identity, contradiction, and
telemetry requirements from the approved design. Return structured `status`,
`detail`, and eligible `assessment` fields so the component never selects raw
feedback independently. The label map is a separate display projection.

Inside the component, derive interaction state exactly once:

```ts
const statusMarks: Record<ReceiptStatus, string> = {
  moved: "↗",
  confirmed: "✓",
  unused: "—",
  obstructed: "↶",
  missing: "⊘",
  unknown: "?",
};

const [activeId, setActiveId] = useState(receiptItems[0].id);
const [showCounterfactual, setShowCounterfactual] = useState(false);
const [showEvidence, setShowEvidence] = useState(false);
const selected = receiptItems.find((item) => item.id === activeId) ?? receiptItems[0];
const selectedProjection = receiptProjection(selected);
const selectedLabel = selectedProjection.label;
const displayTrace = showCounterfactual && selected.id === "brand-kit-contract"
  ? simulatedTrace
  : selected.trace;
const structuredReceipt = showEvidence ? JSON.stringify(selected, null, 2) : null;
```

- [ ] **Step 2: Implement the semantic receipt shell**

Render:

```tsx
<section className="section receipt-section" id="receipt">
  <div className="section-shell">
    <div className="section-intro wide-intro dark-text">
      <div className="eyebrow">07 · Interactive lifecycle concept</div>
      <h2>See which context changed the work—and which didn&apos;t.</h2>
      <p>A Context Receipt links what entered a task to observable decisions, actions, and outcomes—not private reasoning or causal certainty.</p>
      <span className="prototype-label">Lifecycle prototype · not in the 1.0 retrieval wire</span>
    </div>
    <div className="receipt-shell">
      <dl className="receipt-task-summary" aria-label="Illustrative task outcome">
        <div><dt>Task</dt><dd>Create the adaptive Stella brand kit</dd></div>
        <div><dt>Outcome</dt><dd>Delivered · artifact contract passed</dd></div>
        <div><dt>Frame / Receipt</dt><dd>5 selected context items · 1 missing-context observation · evidence varies by entry</dd></div>
      </dl>
      <div className="receipt-workspace">
        <ul className="receipt-items" aria-label="Context Receipt items">
          {receiptItems.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="receipt-item-button"
                aria-pressed={selected.id === item.id}
                aria-controls="context-impact-detail"
                onClick={() => {
                  setActiveId(item.id);
                  setShowCounterfactual(false);
                  setShowEvidence(false);
                }}
              >
                <span className="receipt-status-mark" aria-hidden="true">{statusMarks[receiptProjection(item).status]}</span>
                <span><strong>{item.title}</strong><small>{receiptProjection(item).label}</small></span>
              </button>
            </li>
          ))}
        </ul>
        <div className="receipt-impact" id="context-impact-detail">
          <p className="receipt-selection-announcement" aria-live="polite">Selected context: {selected.title}. {selectedLabel}.</p>
          <div className="receipt-impact-heading">
            <div><span>Impact assessment</span><h3>{selected.title}</h3></div>
            <strong>{selectedLabel}</strong>
          </div>
          <p id="impact-trace-description">Four stages connect the receipt entry to only the decisions, actions, and outcomes supported by observable evidence.</p>
          <ol className="receipt-trace" aria-label="Impact trace" aria-describedby="impact-trace-description" role="list">
            {displayTrace.map((stage) => (
              <li className={stage.evidence_ref ? "receipt-trace-step" : "receipt-trace-step is-gap"} key={stage.label}>
                <span>{stage.label}</span><strong>{stage.value}</strong>
                <code>{stage.evidence_ref ?? "no_evidence_ref"}</code>
              </li>
            ))}
          </ol>
          <p className="receipt-attribution">{selected.attribution_basis}</p>
          <p>No chain-of-thought is stored. This receipt records a bounded influence claim and observable references.</p>
          {selected.id === "brand-kit-contract" && (
            <div>
              <button type="button" className="receipt-counterfactual" aria-pressed={showCounterfactual} onClick={() => setShowCounterfactual((value) => !value)}>
                Simulated counterfactual
              </button>
              {showCounterfactual && <p>Illustrative simulation · not observed evidence</p>}
            </div>
          )}
          <details className="receipt-evidence" key={selected.id} onToggle={(event) => setShowEvidence(event.currentTarget.open)}>
            <summary>View evidence</summary>
            {structuredReceipt && <pre><code>{structuredReceipt}</code></pre>}
          </details>
        </div>
      </div>
    </div>
  </div>
</section>
```

Use a `<ul aria-label="Context Receipt items">` whose rows contain native
`<button type="button">` elements. Each button sets the selected item, exposes
`aria-pressed`, and points to `aria-controls="context-impact-detail"`. Do not
add tab roles or custom arrow-key behavior.

- [ ] **Step 3: Implement the Impact Trace**

Render the selected item's four stages as an `<ol className="receipt-trace"
aria-label="Impact trace">`. Each stage shows its visible label, value, and
evidence reference. A missing value must say `No linked decision`,
`No observable action`, `Not evaluated`, or `Trace unavailable`; never draw an
uninterrupted evidence path through absent data.

Below the trace, render the visible status, `attribution_basis`,
`evaluation_method`, confidence when present, and the influence statement. Add
this exact disclosure:

```text
No chain-of-thought is stored. This receipt records a bounded influence claim and observable references.
```

Use native `<details>` and `<summary>View evidence</summary>` to list stable
references and the structured field names. The initial SSR response must not
contain the raw selected receipt or its private fields. Mount the structured
evidence only after the user explicitly opens the disclosure, and reset it when
selection changes. Assert the privacy boundary against the built worker HTML.

- [ ] **Step 4: Implement the simulated counterfactual**

Add one native button labeled `Simulated counterfactual` with `aria-pressed`.
When active for the default item, replace the four trace values with an
illustrative absent-contract path and visibly render:

```text
Illustrative simulation · not observed evidence
```

The simulated state never changes evaluation, confidence, or evidence data.

- [ ] **Step 5: Integrate the component into the page**

Import the component, add `<a href="#receipt">Receipt</a>` between Protocol and
Architecture, render `<ContextReceiptDemo />` between lifecycle and
architecture, and renumber Architecture, Adoption, and Future eyebrows from
07/08/09 to 08/09/10.

- [ ] **Step 6: Run the focused test toward GREEN**

Run: `npm test`

Expected: receipt content/source assertions pass; CSS assertions may remain red
until Task 3.

- [ ] **Step 7: Commit the component**

```bash
git add app/_components/ContextReceiptDemo.tsx app/_components/contextReceiptProjector.mjs app/page.tsx tests/rendered-html.test.mjs
git commit -m "feat: add Context Receipt impact trace"
```

### Task 3: Add responsive, accessible receipt styling

**Files:**

- Modify: `app/globals.css`
- Test: `tests/rendered-html.test.mjs`

**Interfaces:**

- Consumes: the `receipt-*`, `prototype-label`, and status modifier classes emitted by Task 2.
- Produces: desktop 40/60 composition, narrow-screen vertical trace, non-color state marks, focus treatment, and reduced-motion behavior.

- [ ] **Step 1: Add the desktop receipt layout**

Place receipt styles after `.governance-split` and before
`.architecture-section`. Use the existing CSS variables and borders. Required
selectors include:

```css
.receipt-section { background: var(--soft); }
.prototype-label { border: 1px solid #b8b8b8; display: inline-flex; font-family: var(--font-geist-mono), monospace; font-size: 11px; padding: 9px 12px; text-transform: uppercase; }
.receipt-shell { background: var(--paper); border: 1px solid #dcdcdc; }
.receipt-task-summary { border-bottom: 1px solid #dcdcdc; display: grid; grid-template-columns: repeat(3, 1fr); margin: 0; }
.receipt-workspace { display: grid; grid-template-columns: minmax(260px, 0.4fr) minmax(0, 0.6fr); }
.receipt-items { list-style: none; margin: 0; padding: 0; }
.receipt-item-button { align-items: center; background: transparent; border: 0; border-bottom: 1px solid #dcdcdc; display: grid; grid-template-columns: 32px 1fr; min-height: 72px; padding: 14px 16px; text-align: left; width: 100%; }
.receipt-item-button[aria-pressed="true"] { background: var(--ink); color: var(--paper); }
.receipt-status-mark { font-family: var(--font-geist-mono), monospace; }
.receipt-impact { border-left: 1px solid #dcdcdc; min-width: 0; padding: 28px; }
.receipt-trace { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); list-style: none; margin: 28px 0; padding: 0; }
.receipt-trace-step { border-top: 2px solid var(--ink); min-width: 0; padding: 16px 14px 0 0; }
.receipt-trace-step.is-gap { border-top-style: dashed; color: var(--muted); }
.receipt-attribution { border-left: 3px solid var(--electric); font-size: 14px; padding-left: 14px; }
.receipt-counterfactual { min-height: 44px; }
.receipt-evidence code { overflow-wrap: anywhere; white-space: pre-wrap; }
```

The trace is a four-column ordered list on wide screens. Use borders, symbols,
and status text in addition to color. Body and status copy in this section must
not be smaller than 14px.

- [ ] **Step 2: Add focus, touch, mobile, and reduced-motion rules**

Add a visible high-contrast focus rule for site links, buttons, and summaries:

```css
:where(a, button, summary):focus-visible {
  outline: 3px solid var(--electric);
  outline-offset: 3px;
}
```

Make selector buttons at least 44px high. In the existing
`@media (max-width: 760px)` block, stack the workspace and trace into one
column, place the selected detail directly after the selector, allow long IDs
to wrap, and prevent horizontal overflow. In the existing
`@media (prefers-reduced-motion: reduce)` block, disable receipt transitions.

- [ ] **Step 3: Run tests and verify GREEN**

Run: `npm test`

Expected: 0 failing tests and a successful production build.

- [ ] **Step 4: Run static checks**

Run: `npm run lint`

Expected: 0 lint errors.

- [ ] **Step 5: Commit the styling**

```bash
git add app/globals.css tests/rendered-html.test.mjs
git commit -m "style: clarify Context Receipt evidence states"
```

### Task 4: Verify the complete change and publish the private site

**Files:**

- Verify: `app/_components/ContextReceiptDemo.tsx`
- Verify: `app/page.tsx`
- Verify: `app/globals.css`
- Verify: `tests/rendered-html.test.mjs`
- Verify: `.openai/hosting.json`

**Interfaces:**

- Consumes: the completed branch and existing Sites project identity.
- Produces: one validated commit SHA and one private deployment of the exact validated source.

- [ ] **Step 1: Run final verification**

Run:

```bash
npm test
npm run lint
git diff --check main...HEAD
git status --short
```

Expected: tests and lint exit 0, diff check emits no errors, and status is clean.

- [ ] **Step 2: Review requirements line by line**

Confirm all six statuses, exact field names, prototype boundary, missing-context
separation, no-chain-of-thought copy, simulated-not-evidence copy, native
buttons, focus behavior, responsive trace, and reduced-motion behavior are
present. Confirm no dependency, persistence, metadata, GitHub URL, or hosting
configuration changed.

- [ ] **Step 3: Package and publish**

Package the existing successful Sites build, save one new version using the
branch-head commit SHA, deploy it privately, and poll until the deployment
reports `succeeded`.

- [ ] **Step 4: Open and report the deployed site**

Open the exact deployed URL in Codex and return it as the primary deliverable
with one sentence explaining the Context Receipt interaction.
