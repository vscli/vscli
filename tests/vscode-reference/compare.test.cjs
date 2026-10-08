'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { compareBindings, shimTrace } = require('./compare.cjs');

test('inventory categories retain contexts, argument presence, chords and every reference rule', () => {
  const reference = [
    { key: 'shift+ctrl+p', command: 'palette' },
    { key: 'ctrl+k ctrl+c', command: 'comment', when: 'editorTextFocus && !editorReadonly' },
    { key: 'f9', command: 'extension', args: null },
    { key: 'f1', command: 'unsupported' },
    { key: 'shift+ctrl+p', command: 'palette' },
  ];
  const native = [
    { key: 'ctrl+shift+p', command: 'palette', when: null },
    { key: 'ctrl+k ctrl+c', command: 'comment', when: 'editorTextFocus' },
    { key: 'f9', command: 'extension' },
  ];
  const report = compareBindings(reference, native);
  assert.deepEqual(report.counts, { sameRuleFields: 2, sameKeyCommandArgs: 1, sameCommandId: 1, missingCommandId: 1 });
  assert.equal(report.referenceRules, 5);
  assert.deepEqual(report.rules.map(rule => rule.index), [0, 1, 2, 3, 4]);
  assert.equal(report.rules[2].reference.args, null);
  assert.equal(report.nativeCommandIds, 3);
});

test('object arguments compare structurally while array order and chord order remain significant', () => {
  const rule = { key: 'ctrl+k ctrl+c', command: 'example', args: { b: [1, 2], a: true } };
  assert.equal(compareBindings([rule], [{ ...rule, args: { a: true, b: [1, 2] } }]).counts.sameRuleFields, 1);
  assert.equal(compareBindings([rule], [{ ...rule, args: { a: true, b: [2, 1] } }]).counts.sameCommandId, 1);
  assert.equal(compareBindings([rule], [{ ...rule, key: 'ctrl+c ctrl+k' }]).counts.sameCommandId, 1);
});

test('shared configuration fixture runs to completion with named observations', async () => {
  const trace = await shimTrace();
  assert.equal(trace.length, 25);
  assert.equal(new Set(trace.map(item => item.name)).size, trace.length);
  assert.equal(trace.find(item => item.name === 'held.get').value, true);
  assert.equal(trace.find(item => item.name === 'changed.listenerRead').value, false);
});
