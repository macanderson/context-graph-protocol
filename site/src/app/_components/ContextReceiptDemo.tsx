"use client";

import { useState } from "react";
import {
  projectReceipt,
  RECEIPT_STATUS_LABELS,
} from "./contextReceiptProjector.mjs";

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

type ReceiptBase = {
  id: string;
  title: string;
  kind: string;
  record_revision: string;
  attribution_basis: string;
  trace: readonly TraceStage[];
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
  expected_requirement?: string;
  evidence_refs: string[];
  selected_context_use_id?: string | null;
  selected_use_trace_id?: string | null;
};

type ReceiptItem = UsedReceiptItem | MissingReceiptItem;

const receiptItems: ReceiptItem[] = [
  {
    id: "brand-kit-contract",
    title: "Brand-kit delivery contract",
    kind: "Artifact contract",
    record_revision: "revision 03",
    attribution_basis: "Direct artifact and validation references · confidence 94/100",
    trace: [
      { label: "Context item", value: "Artifact contract · revision 03", evidence_ref: "plan-checklist-04" },
      { label: "Decision", value: "Create a required-file checklist before generation.", evidence_ref: "plan-checklist-04" },
      { label: "Observable action", value: "Produced the wordmark, mark, theme variants, manifest, and social preview.", evidence_ref: "brand-manifest.json" },
      { label: "Verified outcome", value: "All required deliverables passed the bound artifact contract.", evidence_ref: "validation-07" },
    ],
    source_kind: "context_use",
    context_use: {
      context_use_id: "context-use-brand-kit-03",
      use_trace_id: "use-trace-brand-kit-03",
      context_record_id: "brand-kit-contract",
      use_stage: "cited",
      selected: true,
      rendered: true,
      cited: true,
      telemetry_complete: true,
      selection_reason: "Required deliverables applied to the brand-kit task.",
    },
    context_use_feedback_events: [{
      feedback_id: "feedback-brand-kit-03",
      context_use_id: "context-use-brand-kit-03",
      use_trace_id: "use-trace-brand-kit-03",
      evaluation: "helpful",
      evaluation_method: "deterministic_validation",
      attribution_confidence: 94,
      influence_stage: "planning",
      influence_statement: "The contract made the required deliverables explicit before generation began.",
      had_opportunity: true,
      observable_effect_refs: ["brand-manifest.json", "validation-07"],
      outcome_relation: "supported",
      outcome_assessment_id: "outcome-brand-kit-07",
    }],
  },
  {
    id: "terminal-native-identity",
    title: "Terminal-native identity",
    kind: "Directive",
    record_revision: "revision 03",
    attribution_basis: "Explicit user feedback · confidence 91/100",
    trace: [
      { label: "Context item", value: "Directive · revision 03", evidence_ref: "decision-terminal-03" },
      { label: "Decision", value: "Keep the `> Stella` terminal lockup already present in the plan.", evidence_ref: "decision-terminal-03" },
      { label: "Observable action", value: "Applied the terminal lockup consistently across the brand sheet and exports.", evidence_ref: "brand-sheet.svg" },
      { label: "Verified outcome", value: "User-confirmed direction remained intact in accepted artifacts.", evidence_ref: "accepted-diff-08" },
    ],
    source_kind: "context_use",
    context_use: {
      context_use_id: "context-use-terminal-03",
      use_trace_id: "use-trace-terminal-03",
      context_record_id: "terminal-native-identity",
      use_stage: "cited",
      selected: true,
      rendered: true,
      cited: true,
      telemetry_complete: true,
      selection_reason: "Confirmed direction applied to visual identity decisions.",
    },
    context_use_feedback_events: [{
      feedback_id: "feedback-terminal-03",
      context_use_id: "context-use-terminal-03",
      use_trace_id: "use-trace-terminal-03",
      evaluation: "helpful",
      evaluation_method: "explicit_user_feedback",
      attribution_confidence: 91,
      influence_stage: "verification",
      influence_statement: "The directive preserved the accepted terminal-native lockup across outputs.",
      had_opportunity: true,
      observable_effect_refs: ["brand-sheet.svg", "accepted-diff-08"],
      outcome_relation: "supported",
      outcome_assessment_id: "outcome-terminal-08",
    }],
  },
  {
    id: "photography-treatment",
    title: "Photography treatment",
    kind: "Memory",
    record_revision: "episode 07",
    attribution_basis: "Complete telemetry found no citation or observable effect",
    trace: [
      { label: "Context item", value: "Memory · episode 07", evidence_ref: null },
      { label: "Decision", value: "No linked decision", evidence_ref: null },
      { label: "Observable action", value: "No observable action", evidence_ref: null },
      { label: "Verified outcome", value: "Not evaluated", evidence_ref: null },
    ],
    source_kind: "context_use",
    context_use: {
      context_use_id: "context-use-photography-07",
      use_trace_id: "use-trace-photography-07",
      context_record_id: "photography-treatment",
      use_stage: "rendered",
      selected: true,
      rendered: true,
      cited: false,
      telemetry_complete: true,
      selection_reason: "Historical treatment was available in the task frame.",
    },
    context_use_feedback_events: [],
  },
  {
    id: "generic-constellation-mark",
    title: "Generic constellation mark",
    kind: "Directive",
    record_revision: "revision 01",
    attribution_basis: "Explicit correction plus accepted repository state · confidence 88/100",
    trace: [
      { label: "Context item", value: "Directive · revision 01", evidence_ref: "correction-02" },
      { label: "Decision", value: "Start from a constellation-shaped symbol.", evidence_ref: "correction-02" },
      { label: "Observable action", value: "Generated a generic first draft that was later replaced.", evidence_ref: "accepted-diff-08" },
      { label: "Verified outcome", value: "User correction and accepted diff contradicted the direction.", evidence_ref: "accepted-diff-08" },
    ],
    source_kind: "context_use",
    context_use: {
      context_use_id: "context-use-constellation-01",
      use_trace_id: "use-trace-constellation-01",
      context_record_id: "generic-constellation-mark",
      use_stage: "cited",
      selected: true,
      rendered: true,
      cited: true,
      telemetry_complete: true,
      selection_reason: "An earlier visual directive was selected for the first draft.",
    },
    context_use_feedback_events: [{
      feedback_id: "feedback-constellation-01",
      context_use_id: "context-use-constellation-01",
      use_trace_id: "use-trace-constellation-01",
      evaluation: "not_helpful",
      evaluation_method: "explicit_user_feedback",
      attribution_confidence: 88,
      influence_stage: "planning",
      influence_statement: "The earlier directive shaped an initial draft that the accepted correction replaced.",
      had_opportunity: true,
      observable_effect_refs: ["correction-02", "accepted-diff-08"],
      outcome_relation: "contradicted",
      outcome_assessment_id: "outcome-constellation-08",
    }],
  },
  {
    id: "reduced-motion",
    title: "Reduced-motion behavior",
    kind: "Missing context",
    record_revision: "not_selected",
    attribution_basis: "Missing-context observation, not negative use feedback",
    trace: [
      { label: "Context item", value: "Missing context · not_selected", evidence_ref: null },
      { label: "Decision", value: "No linked planning requirement.", evidence_ref: null },
      { label: "Observable action", value: "Animated spinner was initially generated without a reduced-motion state.", evidence_ref: "validation-reduced-motion-05" },
      { label: "Verified outcome", value: "Deterministic validation found the omission before delivery.", evidence_ref: "validation-reduced-motion-05" },
    ],
    source_kind: "missing_context",
    observation_kind: "missing_context",
    missing_context_kind: "not_selected",
    detection_method: "deterministic_validation",
    expected_context_record_id: "reduced-motion-behavior",
    evidence_refs: ["validation-reduced-motion-05"],
  },
  {
    id: "legacy-spacing-note",
    title: "Legacy spacing note",
    kind: "Knowledge",
    record_revision: "revision 04",
    attribution_basis: "Incomplete execution telemetry; no impact claim",
    trace: [
      { label: "Context item", value: "Knowledge · revision 04", evidence_ref: null },
      { label: "Decision", value: "Trace unavailable", evidence_ref: null },
      { label: "Observable action", value: "Trace unavailable", evidence_ref: null },
      { label: "Verified outcome", value: "Evidence incomplete", evidence_ref: null },
    ],
    source_kind: "context_use",
    context_use: {
      context_use_id: "context-use-spacing-04",
      use_trace_id: "use-trace-spacing-04",
      context_record_id: "legacy-spacing-note",
      use_stage: "rendered",
      selected: true,
      rendered: true,
      cited: false,
      telemetry_complete: false,
      selection_reason: "A legacy spacing note was included with incomplete execution telemetry.",
    },
    context_use_feedback_events: [],
  },
];

const receiptProjectionPolicy = {
  display_confidence_floor: 80,
  trusted_evaluation_methods: [
    "deterministic_validation",
    "explicit_user_feedback",
    "external_outcome",
    "accepted_repository_state",
    "controlled_comparison",
    "trace_correlation",
  ],
  known_context_uses: receiptItems.flatMap((item) => (
    item.source_kind === "context_use" ? [item.context_use] : []
  )),
};

const simulatedTrace: readonly TraceStage[] = [
  { label: "Context item", value: "Required-file contract absent", evidence_ref: null },
  { label: "Decision", value: "No explicit deliverable checklist", evidence_ref: null },
  { label: "Observable action", value: "Wordmark and social preview omitted", evidence_ref: null },
  { label: "Verified outcome", value: "Illustrative validator failure", evidence_ref: null },
];

const statusMarks: Record<ReceiptStatus, string> = {
  moved: "↗",
  confirmed: "✓",
  unused: "—",
  obstructed: "↶",
  missing: "⊘",
  unknown: "?",
};

function receiptProjection(item: ReceiptItem) {
  const projection = projectReceipt(item, receiptProjectionPolicy);
  const status = projection.status as ReceiptStatus;
  return {
    ...projection,
    status,
    label: projection.detail
      ? `${RECEIPT_STATUS_LABELS[status]} · ${projection.detail}`
      : RECEIPT_STATUS_LABELS[status],
  };
}

export default function ContextReceiptDemo() {
  const [activeId, setActiveId] = useState(receiptItems[0].id);
  const [showCounterfactual, setShowCounterfactual] = useState(false);
  const [showEvidence, setShowEvidence] = useState(false);
  const selected = receiptItems.find((item) => item.id === activeId) ?? receiptItems[0];
  const selectedProjection = receiptProjection(selected);
  const selectedLabel = selectedProjection.label;
  const selectedFeedback = selectedProjection.assessment as ContextUseFeedback | null;
  const displayTrace = showCounterfactual && selected.id === "brand-kit-contract"
    ? simulatedTrace
    : selected.trace;
  const structuredReceipt = showEvidence ? JSON.stringify(selected, null, 2) : null;

  return (
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
              {receiptItems.map((item) => {
                const projection = receiptProjection(item);
                return (
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
                      <span className="receipt-status-mark" aria-hidden="true">{statusMarks[projection.status]}</span>
                      <span><strong>{item.title}</strong><small>{projection.label}</small></span>
                    </button>
                  </li>
                );
              })}
            </ul>
            <div className="receipt-impact" id="context-impact-detail">
              <p className="receipt-selection-announcement" aria-live="polite">
                Selected context: {selected.title}. {selectedLabel}.
              </p>
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
              <p className="receipt-attribution"><strong>Attribution basis</strong> {selected.attribution_basis}</p>
              {selected.source_kind === "context_use" ? (
                <dl className="receipt-evaluation">
                  <div><dt>Influence statement</dt><dd>{selectedFeedback?.influence_statement ?? "No attributable influence observed"}</dd></div>
                  <div><dt>Evaluation method</dt><dd>{selectedFeedback?.evaluation_method ?? "Not evaluated"}</dd></div>
                  <div><dt>Evaluation</dt><dd>{selectedFeedback?.evaluation ?? "Not evaluated"}</dd></div>
                  {selectedFeedback && <div><dt>Confidence</dt><dd>{selectedFeedback.attribution_confidence}/100</dd></div>}
                </dl>
              ) : (
                <dl className="receipt-evaluation">
                  <div><dt>Missing context kind</dt><dd>{selected.missing_context_kind}</dd></div>
                  <div><dt>Detection method</dt><dd>{selected.detection_method}</dd></div>
                </dl>
              )}
              <p>No chain-of-thought is stored. This receipt records a bounded influence claim and observable references.</p>
              <p>Receipt statuses are derived from bounded observations, not a usefulness score. Self-report is not proof.</p>
              {selected.id === "brand-kit-contract" && (
                <>
                  <button type="button" className="receipt-counterfactual" aria-pressed={showCounterfactual} onClick={() => setShowCounterfactual((value) => !value)}>
                    Simulated counterfactual
                  </button>
                  <p hidden={!showCounterfactual}>Illustrative simulation · not observed evidence</p>
                </>
              )}
              <details
                className="receipt-evidence"
                key={selected.id}
                onToggle={(event) => setShowEvidence(event.currentTarget.open)}
              >
                <summary>View evidence</summary>
                {structuredReceipt && <pre><code>{structuredReceipt}</code></pre>}
              </details>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
