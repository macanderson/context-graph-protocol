import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

test("keeps the interactive narrative, receipt contract, and social asset wired", async () => {
  const [page, layout, receipt, css, packageJson] = await Promise.all([
    readFile(new URL("../src/app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/layout.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/_components/ContextReceiptDemo.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/marketing.css", import.meta.url), "utf8"),
    readFile(new URL("../package.json", import.meta.url), "utf8"),
    access(new URL("../public/og.png", import.meta.url)),
  ]);

  assert.match(page, /^"use client";/);
  assert.match(page, /useState\(0\)/);
  assert.match(page, /aria-label="Interactive context graph"/);
  assert.match(page, /aria-label="Historical query date"/);
  assert.match(page, /aria-label="Adaptive context lifecycle"/);
  assert.match(page, /href="#receipt"/);
  assert.doesNotMatch(page, /role="tab(?:list|panel)?"/);
  assert.match(page, /role="group"/);
  assert.match(page, /className="skip-link"/);
  assert.match(
    page,
    /<div className="marketing-page">[\s\S]*?<header[\s\S]*?<main id="main-content">[\s\S]*?<\/main>\s*<footer>/,
  );
  assert.match(page, /repository publication/);
  assert.doesNotMatch(page, /workspace publication|repository or workspace scope/);
  assert.match(layout, /export const metadata: Metadata/);
  assert.match(layout, /NEXT_PUBLIC_SITE_URL/);
  assert.doesNotMatch(layout, /generateMetadata|next\/headers|x-forwarded-host/);
  assert.match(layout, /\/og\.png/);
  assert.match(receipt, /aria-label="Context Receipt items"/);
  assert.match(receipt, /id="impact-trace-description"/);
  assert.match(receipt, /aria-describedby="impact-trace-description"/);
  assert.match(receipt, /connect the receipt entry/i);
  assert.match(receipt, /<ol\b(?=[^>]*className="receipt-trace")(?=[^>]*role="list")/);
  assert.match(
    receipt,
    /<button\b(?=[^>]*aria-pressed=)(?=[^>]*aria-controls="context-impact-detail")[^>]*>/,
  );
  assert.match(receipt, /context_use_id/);
  assert.match(receipt, /influence_stage/);
  assert.match(receipt, /influence_statement/);
  assert.match(receipt, /had_opportunity/);
  assert.match(receipt, /observable_effect_refs/);
  assert.match(receipt, /outcome_relation/);
  assert.match(receipt, /evaluation_method/);
  assert.match(receipt, /attribution_confidence/);
  assert.match(receipt, /No chain-of-thought is stored/);
  assert.match(receipt, /source_kind:\s*"context_use"[\s\S]*context_use:\s*\{/);
  assert.match(receipt, /context_use_feedback_events:\s*\[\{/);
  assert.match(receipt, /context_use_feedback_events:\s*\[\]/);
  assert.match(receipt, /outcome_assessment_id:\s*"[^"\n]+"/);
  assert.match(receipt, /source_kind:\s*"missing_context"/);
  assert.doesNotMatch(receipt, /source_kind:\s*"missing_context"[\s\S]{0,500}context_use_feedback_events/);
  assert.doesNotMatch(receipt, /method_trusted\s*:/);
  assert.doesNotMatch(receipt, /has_contradictory_feedback\s*:/);
  assert.match(receipt, /trusted_evaluation_methods/);
  assert.match(receipt, /projectReceipt\(item, receiptProjectionPolicy\)/);
  assert.doesNotMatch(receipt, /context_use_feedback_events\.at/);
  assert.match(receipt, /evaluation:\s*"not_helpful"/);
  assert.match(receipt, /had_opportunity:\s*true/);
  assert.match(receipt, /observable_effect_refs:\s*\[\s*"[^"]+"/);
  assert.match(receipt, /statuses are derived[\s\S]{0,120}not a usefulness score/i);
  assert.match(receipt, /self-report[\s\S]{0,120}not proof/i);
  assert.match(receipt, /Simulated counterfactual[\s\S]{0,500}not observed evidence/);
  assert.match(receipt, /Impact assessment/);
  assert.doesNotMatch(receipt, /Observed impact/);
  assert.doesNotMatch(receipt, /\bstatus:\s*"(?:moved|confirmed|unused|obstructed|missing|unknown)"/);
  assert.doesNotMatch(receipt, /status_label\s*:/);
  assert.match(
    css,
    /\.receipt-workspace\s*\{[^}]*grid-template-columns:\s*minmax\(260px, 0\.4fr\) minmax\(0, 0\.6fr\);/,
  );
  assert.match(css, /\.receipt-items\s*\{[^}]*list-style:\s*none/);
  assert.match(
    css,
    /\.receipt-trace\s*\{[^}]*grid-template-columns:\s*repeat\(4, minmax\(0, 1fr\)\);/,
  );
  assert.match(
    css,
    /:where\(a, button, summary\):focus-visible\s*\{[^}]*outline:\s*3px solid var\(--electric\);[^}]*box-shadow:\s*0 0 0 6px var\(--ink\);/,
  );
  assert.match(
    css,
    /\.receipt-evidence summary\s*\{[^}]*display:\s*inline-flex;[^}]*align-items:\s*center;[^}]*min-height:\s*44px;/,
  );
  assert.match(css, /\.receipt-counterfactual\s*\{[^}]*min-height:\s*44px;/);
  assert.match(
    css,
    /@media \(max-width: 760px\)\s*\{[\s\S]*?\.receipt-workspace, \.receipt-trace, \.receipt-evaluation\s*\{[^}]*grid-template-columns:\s*1fr;/,
  );
  assert.match(
    css,
    /@media \(prefers-reduced-motion: reduce\)\s*\{[\s\S]*?\.receipt-item-button,\s*\.receipt-counterfactual\s*\{[^}]*transition:\s*none;/,
  );
  assert.doesNotMatch(packageJson, /react-loading-skeleton/);
  await assert.rejects(access(new URL("../src/app/_sites-preview", import.meta.url)));
});

test("uses explicit buttons and scoped reduced-motion styles", async () => {
  const [page, receipt, css] = await Promise.all([
    readFile(new URL("../src/app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/_components/ContextReceiptDemo.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/marketing.css", import.meta.url), "utf8"),
  ]);

  for (const source of [page, receipt]) {
    const buttonCount = source.match(/<button\b/g)?.length ?? 0;
    const explicitButtonCount = source.match(/<button\b[^\n]*\btype="button"|<button\s+type="button"/g)?.length ?? 0;
    assert.ok(buttonCount > 0);
    assert.equal(explicitButtonCount, buttonCount);
  }

  assert.doesNotMatch(css, /^\.marketing-page,\s*\n\.marketing-page \* \{/m);
  assert.doesNotMatch(css, /transition-duration:\s*0\.01ms/);
  assert.doesNotMatch(css, /^\s*\*,\s*\*::before,\s*\*::after/m);
  assert.match(css, /:where\(\.marketing-page\) a\s*\{[^}]*text-decoration:\s*none;/);
  assert.doesNotMatch(css, /^\.marketing-page a\s*\{/m);
});

test("projects one deterministic receipt status from canonical evidence", async () => {
  const { projectReceiptStatus } = await import(
    new URL("../src/app/_components/contextReceiptProjector.mjs", import.meta.url),
  );

  const use = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    context_record_id: "rule-1",
    selected: true,
    rendered: true,
    cited: false,
    telemetry_complete: true,
  };
  const contextUse = {
    source_kind: "context_use",
    context_use: use,
    context_use_feedback_events: [],
  };
  const policy = {
    display_confidence_floor: 80,
    trusted_evaluation_methods: ["deterministic_validation"],
    known_context_uses: [use],
  };
  const eligible = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    evaluation: "helpful",
    evaluation_method: "deterministic_validation",
    attribution_confidence: 94,
    influence_stage: "planning",
    influence_statement: "The context changed an observable planning decision.",
    had_opportunity: true,
    observable_effect_refs: ["effect-1"],
    outcome_relation: "supported",
    outcome_assessment_id: "outcome-1",
  };

  const cases = [
    ["missing", {
      source_kind: "missing_context",
      observation_kind: "missing_context",
      missing_context_kind: "not_selected",
      expected_context_record_id: "required-rule-1",
      evidence_refs: ["validator-1"],
    }],
    ["obstructed", { ...contextUse, context_use_feedback_events: [{ ...eligible, evaluation: "not_helpful", outcome_relation: "contradicted" }] }],
    ["confirmed", { ...contextUse, context_use_feedback_events: [{ ...eligible, influence_stage: "verification" }] }],
    ["moved", { ...contextUse, context_use_feedback_events: [eligible] }],
    ["unused", contextUse],
    ["unknown", { ...contextUse, context_use: { ...use, telemetry_complete: false } }],
  ];

  for (const [expected, item] of cases) {
    assert.equal(projectReceiptStatus(item, policy), expected);
  }
});

test("keeps trust host-owned and derives contested feedback from history", async () => {
  const { projectReceipt, projectReceiptStatus } = await import(
    new URL("../src/app/_components/contextReceiptProjector.mjs", import.meta.url),
  );
  const use = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    context_record_id: "rule-1",
    selected: true,
    rendered: true,
    cited: true,
    telemetry_complete: true,
  };
  const base = { source_kind: "context_use", context_use: use };
  const feedback = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    evaluation: "helpful",
    evaluation_method: "deterministic_validation",
    attribution_confidence: 94,
    influence_stage: "planning",
    influence_statement: "The context changed an observable planning decision.",
    had_opportunity: true,
    observable_effect_refs: ["effect-1"],
    outcome_relation: "supported",
    outcome_assessment_id: "outcome-1",
  };
  const policy = {
    display_confidence_floor: 80,
    trusted_evaluation_methods: ["deterministic_validation"],
    known_context_uses: [use],
  };

  assert.equal(
    projectReceiptStatus({ ...base, context_use_feedback_events: [{ ...feedback, method_trusted: true }] }),
    "unknown",
  );

  const laterContradiction = {
    ...feedback,
    evaluation: "not_helpful",
    outcome_relation: "contradicted",
  };
  const contested = projectReceipt(
    { ...base, context_use_feedback_events: [feedback, laterContradiction] },
    policy,
  );
  assert.deepEqual(contested, {
    status: "unknown",
    detail: "Contested",
    assessment: null,
  });

  const selfReport = projectReceipt({
    ...base,
    context_use_feedback_events: [{
      ...feedback,
      evaluation_method: "agent_self_report",
    }],
  }, policy);
  assert.deepEqual(selfReport, {
    status: "unknown",
    detail: "Reported influence",
    assessment: null,
  });
});

test("rejects ineligible attribution and incomplete cross-field evidence", async () => {
  const { projectReceiptDetail, projectReceiptStatus } = await import(
    new URL("../src/app/_components/contextReceiptProjector.mjs", import.meta.url),
  );
  const use = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    context_record_id: "rule-1",
    selected: true,
    rendered: true,
    cited: true,
    telemetry_complete: true,
  };
  const base = { source_kind: "context_use", context_use: use };
  const policy = {
    display_confidence_floor: 80,
    trusted_evaluation_methods: ["deterministic_validation"],
    known_context_uses: [use],
  };
  const feedback = {
    context_use_id: "use-1",
    use_trace_id: "trace-1",
    evaluation: "helpful",
    evaluation_method: "deterministic_validation",
    attribution_confidence: 94,
    influence_stage: "planning",
    influence_statement: "The context changed an observable planning decision.",
    had_opportunity: true,
    observable_effect_refs: ["effect-1"],
    outcome_relation: "supported",
    outcome_assessment_id: "outcome-1",
  };

  const ineligibleFeedback = [
    { ...feedback, evaluation_method: "unregistered_method" },
    { ...feedback, evaluation_method: "agent_self_report" },
    { ...feedback, attribution_confidence: 79 },
    { ...feedback, observable_effect_refs: [] },
    { ...feedback, outcome_assessment_id: null },
    { ...feedback, context_use_id: "different-use" },
    { ...feedback, use_trace_id: "different-trace" },
    { ...feedback, attribution_confidence: 101 },
    { ...feedback, influence_statement: "" },
    { ...feedback, influence_statement: "x".repeat(501) },
    { ...feedback, outcome_relation: "unknown" },
  ];

  for (const event of ineligibleFeedback) {
    assert.equal(
      projectReceiptStatus({ ...base, context_use_feedback_events: [event] }, policy),
      "unknown",
    );
  }

  const validNotRendered = {
    source_kind: "missing_context",
    observation_kind: "missing_context",
    missing_context_kind: "not_rendered",
    expected_context_record_id: "rule-1",
    evidence_refs: ["renderer-1"],
    selected_context_use_id: "use-1",
    selected_use_trace_id: "trace-1",
  };
  assert.equal(projectReceiptStatus(validNotRendered, policy), "missing");
  assert.equal(projectReceiptDetail(validNotRendered, policy), "Selected, not rendered");

  const invalidMissingContext = [
    { ...validNotRendered, selected_context_use_id: null, selected_use_trace_id: null },
    { ...validNotRendered, selected_context_use_id: "different-use" },
    { ...validNotRendered, evidence_refs: [] },
    {
      source_kind: "missing_context",
      observation_kind: "missing_context",
      missing_context_kind: "not_selected",
      expected_context_record_id: null,
      evidence_refs: ["validator-1"],
    },
    {
      source_kind: "missing_context",
      observation_kind: "missing_context",
      missing_context_kind: "unavailable",
      expected_context_record_id: "rule-1",
      expected_requirement: "A reduced-motion rule",
      evidence_refs: ["validator-1"],
    },
  ];

  for (const item of invalidMissingContext) {
    assert.equal(projectReceiptStatus(item, policy), "unknown");
  }
});
