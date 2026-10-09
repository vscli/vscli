'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { collectionProjection } = require('./diagnostics-compare.cjs');
const { validateCapture } = require('./diagnostics-record.cjs');

function trace() {
  return { schema: 1, snapshots: Array.from({ length: 21 }, (_, index) => ({ name: `collection-${index}` })),
    events: [], retainedEventUris: [] };
}

test('collection projection explicitly removes only document bridge observations', () => {
  const input = trace();
  input.snapshots.push({ name: 'document-set-before-edit' }, { name: 'closed-document-local-read' });
  input.snapshots[0].aggregate = [['open-document', [{ message: 'bridge' }]], ['a.cpp', [{ message: 'collection' }]]];
  input.snapshots[0].one = { entries: [['open-document', []], ['a.cpp', []]] };
  input.events.push({ phase: 'document-close-retention', uris: ['open-document'], aggregate: [] },
    { phase: 'clear', uris: ['a.cpp'], aggregate: [['a.cpp', []]] });
  input.retainedEventUris.push({ uris: ['open-document'], arrayFrozen: true }, { uris: ['a.cpp'], arrayFrozen: true });
  const projected = collectionProjection(input);
  assert.equal(projected.snapshots.length, 21);
  assert.deepEqual(projected.snapshots[0].aggregate, [['a.cpp', [{ message: 'collection' }]]]);
  assert.deepEqual(projected.snapshots[0].one.entries, [['a.cpp', []]]);
  assert.deepEqual(projected.events, [{ phase: 'clear', uris: ['a.cpp'], aggregate: [] }]);
  assert.deepEqual(projected.retainedEventUris, [{ uris: ['a.cpp'], arrayFrozen: true }]);
  assert.equal(input.snapshots.length, 23, 'Reference raw evidence must remain unchanged');
});

test('missing collection observations and unknown schemas fail the comparison gate', () => {
  const missing = trace(); missing.snapshots.pop();
  assert.throws(() => collectionProjection(missing), /must not silently disappear/);
  assert.throws(() => collectionProjection({ ...trace(), schema: 2 }), /schema/);
  const accidentalExtra = trace(); accidentalExtra.snapshots.push({ name: 'new-unqualified-collection-step' });
  assert.throws(() => collectionProjection(accidentalExtra), /must not silently disappear/);
});

test('baseline recording rejects changed observer, trace bytes and product provenance', () => {
  const directory = path.join(__dirname, 'baselines/1.95.0/diagnostics');
  const provenance = JSON.parse(fs.readFileSync(path.join(directory, 'provenance.json'), 'utf8')).platforms.linux;
  const inventory = provenance.reference;
  // The recorded Linux fixtures use LF. Restore those fixture bytes if a
  // Windows checkout converts text; runtime capture/recording hashes remain raw.
  const bytes = Buffer.from(fs.readFileSync(path.join(directory, 'linux.json'), 'utf8').replace(/\r\n/g, '\n'));
  const observer = Buffer.from(fs.readFileSync(path.join(__dirname, 'diagnostics.cjs'), 'utf8').replace(/\r\n/g, '\n'));
  assert.equal(validateCapture(inventory, provenance, bytes, observer).snapshots.length, 21);
  assert.throws(() => validateCapture(inventory, provenance, bytes, Buffer.concat([observer, Buffer.from('\n')])), /Observer changed/);
  assert.throws(() => validateCapture(inventory, provenance, Buffer.concat([bytes, Buffer.from('\n')]), observer), /artifact changed/);
  assert.throws(() => validateCapture({ ...inventory, commit: 'unqualified' }, provenance, bytes, observer));
  assert.throws(() => validateCapture({ ...inventory, platform: 'darwin' }, provenance, bytes, observer));
});
