// The label decision behind .github/workflows/triage-guard.yml (SCR-005,
// https://github.com/macanderson/oxagen/blob/main/.oxagen/rules/ctx.scr.005-triage-separation.toml).
//
// Issue creators apply only `triage`. Priority and size come from the triage
// identity alone, which assigns one priority, optionally a size, and removes
// `triage`. Invariant: every open issue carries a priority or `triage`, never
// neither.
//
// This file is pure — no GitHub calls — so the workflow does the I/O and
// .github/scripts/tests/triage-guard.test.cjs can exercise every decision
// (#137). The workflow loads it with `require` after a sparse checkout.
//
// The reserved families are matched as families, not as tier lists:
//
//   priority  /^P\d+$/      P0 … P4 today
//   size      /^size\/.+$/  size/XS … size/XL today
//
// The guard once matched `/^P[0-3]$/` while the record had grown a `P4`
// tier, and it never matched `size/*` at all (#137). A tier list in the
// guard is a second copy of a fact the record owns, and it drifts the day a
// tier is added. Matching the family means a new tier is guarded the moment
// the label exists, with no edit here.
'use strict';

const PRIORITY = /^P\d+$/;
const SIZE = /^size\/.+$/;
const TRIAGE = 'triage';

/** True for a label only the triage identity may apply. */
function isReserved(name) {
  return PRIORITY.test(name) || SIZE.test(name);
}

/** True for a priority label (`P0`, `P1`, …). */
function isPriority(name) {
  return PRIORITY.test(name);
}

/**
 * Decide what the guard does for one `issues` event.
 *
 * @param {object} event
 * @param {'opened'|'labeled'|string} event.action  the webhook action
 * @param {string} event.sender     login that caused the event
 * @param {string[]} event.allowed  logins that may apply reserved labels
 * @param {string[]} event.labels   labels on the issue now; for `labeled`,
 *                                  read fresh, since the payload is a snapshot
 * @param {string} [event.label]    the label just added (`labeled` only)
 * @returns {{remove: string[], addTriage: boolean, comment: string|null}}
 */
function decide({ action, sender, allowed, labels, label }) {
  const trusted = allowed.includes(sender);
  const none = { remove: [], addTriage: false, comment: null };

  if (action === 'opened') {
    // A priority counts only if the triage identity opened the issue. A
    // reserved label a creator applied at open time arrives as its own
    // `labeled` event from the same sender, and that event strips it, so
    // this branch only has to make sure the issue is queued.
    const heldPriority = trusted && labels.some(isPriority);
    return {
      ...none,
      addTriage: !labels.includes(TRIAGE) && !heldPriority,
    };
  }

  if (action === 'labeled' && label !== undefined && isReserved(label) && !trusted) {
    const remaining = labels.filter((n) => n !== label);
    // Re-queue only when no priority is left. A priority the triage
    // identity already set stands, and adding `triage` beside it would
    // leave the issue both prioritised and unsorted.
    const addTriage = !remaining.some(isPriority) && !remaining.includes(TRIAGE);
    const requeued = !remaining.some(isPriority);
    return {
      remove: [label],
      addTriage,
      comment:
        `Priority and size labels are assigned only by the triage identity (SCR-005). ` +
        `Removed \`${label}\`` +
        (requeued ? '; re-queued as `triage`.' : '; the priority triage set stands.'),
    };
  }

  return none;
}

module.exports = { decide, isReserved, isPriority, TRIAGE };
