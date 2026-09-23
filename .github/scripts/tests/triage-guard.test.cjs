// Tests for .github/scripts/triage-guard.cjs, the decision behind
// .github/workflows/triage-guard.yml (SCR-005, #137).
//
// Run: node --test .github/scripts/tests/triage-guard.test.cjs
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { decide, isReserved } = require('../triage-guard.cjs');

const allowed = ['triage-bot', 'macanderson'];

test('every priority tier and every size is reserved, other labels are not', () => {
  for (const name of ['P0', 'P1', 'P2', 'P3', 'P4', 'P5', 'size/XS', 'size/M', 'size/XL']) {
    assert.equal(isReserved(name), true, name);
  }
  for (const name of ['triage', 'bug', 'Priority', 'P', 'PX', 'P1a', 'xP1', 'size', 'sizes/M']) {
    assert.equal(isReserved(name), false, name);
  }
});

test('a creator-applied P4 is removed and the issue is re-queued as triage', () => {
  const d = decide({ action: 'labeled', sender: 'someone', allowed, labels: ['P4'], label: 'P4' });
  assert.deepEqual(d.remove, ['P4']);
  assert.equal(d.addTriage, true);
  assert.match(d.comment, /Removed `P4`; re-queued as `triage`/);
});

test('a creator-applied size/M is removed and the issue is re-queued as triage', () => {
  const d = decide({
    action: 'labeled', sender: 'someone', allowed, labels: ['bug', 'size/M'], label: 'size/M',
  });
  assert.deepEqual(d.remove, ['size/M']);
  assert.equal(d.addTriage, true);
});

test('a creator size under a priority triage set is removed without re-queueing', () => {
  const d = decide({
    action: 'labeled', sender: 'someone', allowed, labels: ['P2', 'size/M'], label: 'size/M',
  });
  assert.deepEqual(d.remove, ['size/M']);
  assert.equal(d.addTriage, false, 'triage beside P2 would break the invariant');
  assert.match(d.comment, /the priority triage set stands/);
});

test('an issue already queued is not labelled triage twice', () => {
  const d = decide({
    action: 'labeled', sender: 'someone', allowed, labels: ['triage', 'P0'], label: 'P0',
  });
  assert.deepEqual(d.remove, ['P0']);
  assert.equal(d.addTriage, false);
});

test("the triage identity's priority and size labels survive", () => {
  for (const sender of allowed) {
    for (const label of ['P4', 'size/M']) {
      const d = decide({ action: 'labeled', sender, allowed, labels: [label], label });
      assert.deepEqual(d, { remove: [], addTriage: false, comment: null }, `${sender} ${label}`);
    }
  }
});

test('an unreserved label from anyone is left alone', () => {
  const d = decide({ action: 'labeled', sender: 'someone', allowed, labels: ['bug'], label: 'bug' });
  assert.deepEqual(d, { remove: [], addTriage: false, comment: null });
});

test('an issue opened with no labels receives triage', () => {
  const d = decide({ action: 'opened', sender: 'someone', allowed, labels: [] });
  assert.equal(d.addTriage, true);
  assert.deepEqual(d.remove, []);
});

test('an issue a creator opened with only P4 receives triage', () => {
  const d = decide({ action: 'opened', sender: 'someone', allowed, labels: ['P4'] });
  assert.equal(d.addTriage, true);
});

test('an issue the triage identity opened with a priority is not queued', () => {
  const d = decide({ action: 'opened', sender: 'triage-bot', allowed, labels: ['P1'] });
  assert.equal(d.addTriage, false);
});

test('an issue opened already carrying triage is not labelled again', () => {
  const d = decide({ action: 'opened', sender: 'someone', allowed, labels: ['triage'] });
  assert.equal(d.addTriage, false);
});

test('other issue actions do nothing', () => {
  const d = decide({ action: 'edited', sender: 'someone', allowed, labels: ['P1'] });
  assert.deepEqual(d, { remove: [], addTriage: false, comment: null });
});
