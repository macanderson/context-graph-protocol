export const RECEIPT_STATUS_LABELS = Object.freeze({
  moved: "Moved the work",
  confirmed: "Confirmed the work",
  unused: "Selected, no observed use",
  obstructed: "Got in the way",
  missing: "Missing from frame",
  unknown: "Unknown",
});

function trustedMethods(policy) {
  return new Set(policy?.trusted_evaluation_methods ?? []);
}

function confidenceFloor(policy) {
  return Number.isInteger(policy?.display_confidence_floor)
    ? policy.display_confidence_floor
    : 100;
}

function hasOutcomeIdentity(feedback) {
  return !["supported", "contradicted"].includes(feedback.outcome_relation)
    || (typeof feedback.outcome_assessment_id === "string"
      && feedback.outcome_assessment_id.length > 0);
}

function hasMatchingUseIdentity(use, feedback) {
  return typeof use.context_use_id === "string"
    && use.context_use_id.length > 0
    && feedback.context_use_id === use.context_use_id
    && typeof use.use_trace_id === "string"
    && use.use_trace_id.length > 0
    && feedback.use_trace_id === use.use_trace_id;
}

function hasBoundedInfluenceStatement(feedback) {
  return typeof feedback.influence_statement === "string"
    && feedback.influence_statement.length > 0
    && Array.from(feedback.influence_statement).length <= 500;
}

function hasValidEvaluationRelation(feedback) {
  if (feedback.evaluation === "helpful") return feedback.outcome_relation === "supported";
  if (feedback.evaluation === "not_helpful") return feedback.outcome_relation === "contradicted";
  return feedback.evaluation === "neutral"
    && ["unrelated", "unknown"].includes(feedback.outcome_relation);
}

function isEligible(use, feedback, policy) {
  const methods = trustedMethods(policy);
  const floor = confidenceFloor(policy);
  return Boolean(
    feedback
      && hasMatchingUseIdentity(use, feedback)
      && methods.has(feedback.evaluation_method)
      && feedback.evaluation_method !== "agent_self_report"
      && feedback.attribution_confidence >= floor
      && feedback.attribution_confidence <= 100
      && feedback.had_opportunity === true
      && Array.isArray(feedback.observable_effect_refs)
      && feedback.observable_effect_refs.length > 0
      && feedback.observable_effect_refs.every((ref) => typeof ref === "string" && ref.length > 0)
      && hasBoundedInfluenceStatement(feedback)
      && hasValidEvaluationRelation(feedback)
      && hasOutcomeIdentity(feedback),
  );
}

function hasContestedOutcome(feedbackEvents) {
  const evaluationsByOutcome = new Map();
  for (const feedback of feedbackEvents) {
    if (typeof feedback.outcome_assessment_id !== "string") continue;
    const evaluations = evaluationsByOutcome.get(feedback.outcome_assessment_id) ?? new Set();
    evaluations.add(feedback.evaluation);
    evaluationsByOutcome.set(feedback.outcome_assessment_id, evaluations);
  }
  return [...evaluationsByOutcome.values()].some(
    (evaluations) => evaluations.has("helpful") && evaluations.has("not_helpful"),
  );
}

function resolvesSelectedUse(item, policy) {
  const hasUseId = typeof item.selected_context_use_id === "string"
    && item.selected_context_use_id.length > 0;
  const hasTraceId = typeof item.selected_use_trace_id === "string"
    && item.selected_use_trace_id.length > 0;

  const knownUses = (policy?.known_context_uses ?? []).filter((use) => (
    use.selected === true
      && use.context_record_id === item.expected_context_record_id
  ));
  if (knownUses.length > 0 && !hasUseId && !hasTraceId) {
    return false;
  }
  if (!hasUseId && !hasTraceId) return true;

  return knownUses.some((use) => (
    (!hasUseId || use.context_use_id === item.selected_context_use_id)
      && (!hasTraceId || use.use_trace_id === item.selected_use_trace_id)
  ));
}

function isValidMissingContext(item, policy) {
  if (
    item.observation_kind !== "missing_context"
    || !Array.isArray(item.evidence_refs)
    || item.evidence_refs.length === 0
  ) {
    return false;
  }

  const hasRecordId = typeof item.expected_context_record_id === "string"
    && item.expected_context_record_id.length > 0;
  const hasRequirement = typeof item.expected_requirement === "string"
    && item.expected_requirement.length > 0;

  if (["not_retrieved", "not_selected"].includes(item.missing_context_kind)) {
    return hasRecordId;
  }
  if (item.missing_context_kind === "not_rendered") {
    return hasRecordId && resolvesSelectedUse(item, policy);
  }
  if (item.missing_context_kind === "unavailable") {
    return !hasRecordId && hasRequirement;
  }
  if (item.missing_context_kind === "unknown") {
    return hasRecordId || hasRequirement;
  }
  return false;
}

function projectContextUse(item, policy) {
  const use = item.context_use;
  const feedbackHistory = Array.isArray(item.context_use_feedback_events)
    ? item.context_use_feedback_events
    : [];
  const eligibleFeedback = feedbackHistory.filter((feedback) => isEligible(use, feedback, policy));

  if (hasContestedOutcome(eligibleFeedback)) {
    return { status: "unknown", detail: "Contested", assessment: null };
  }

  const feedback = eligibleFeedback.at(-1);
  if (
    feedback?.evaluation === "not_helpful"
    && feedback.outcome_relation === "contradicted"
  ) {
    return { status: "obstructed", detail: null, assessment: feedback };
  }
  if (
    feedback?.evaluation === "helpful"
    && feedback.outcome_relation === "supported"
    && feedback.influence_stage === "verification"
  ) {
    return { status: "confirmed", detail: null, assessment: feedback };
  }
  if (
    feedback?.evaluation === "helpful"
    && feedback.outcome_relation === "supported"
    && ["planning", "execution", "final_response"].includes(feedback.influence_stage)
  ) {
    return { status: "moved", detail: null, assessment: feedback };
  }
  if (
    feedbackHistory.length === 0
    && use.telemetry_complete === true
    && (use.selected === true || use.rendered === true)
    && use.cited === false
  ) {
    return { status: "unused", detail: null, assessment: null };
  }
  if (
    feedbackHistory.length > 0
    && feedbackHistory.every((event) => event.evaluation_method === "agent_self_report")
  ) {
    return { status: "unknown", detail: "Reported influence", assessment: null };
  }
  return { status: "unknown", detail: null, assessment: null };
}

/**
 * Ordered display-only projection from canonical use, feedback history, gap
 * observations, and host-owned projection policy.
 * @param {Record<string, any>} item
 * @param {Record<string, any>} policy
 * @returns {{status: keyof typeof RECEIPT_STATUS_LABELS, detail: string | null, assessment: Record<string, any> | null}}
 */
export function projectReceipt(item, policy) {
  if (item?.source_kind === "missing_context") {
    const valid = isValidMissingContext(item, policy);
    return {
      status: valid ? "missing" : "unknown",
      detail: valid && item.missing_context_kind === "not_rendered"
        ? "Selected, not rendered"
        : null,
      assessment: null,
    };
  }
  if (item?.source_kind !== "context_use" || !item.context_use) {
    return { status: "unknown", detail: null, assessment: null };
  }
  return projectContextUse(item, policy);
}

export function projectReceiptStatus(item, policy) {
  return projectReceipt(item, policy).status;
}

export function projectReceiptDetail(item, policy) {
  return projectReceipt(item, policy).detail;
}
