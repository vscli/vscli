'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const { Position, Range, Uri } = require('./api-types.cjs');
const { normalize: providerResult } = require('./providers.cjs');
const { normalize: diagnosticResult } = require('./diagnostics.cjs');

const invalid = [NaN, Infinity, -Infinity, -1, 0.5, undefined, null, '1', 0x80000000];
const state = () => ({ generation: 1, documents: [{ id: 1, uri: 'untitled:coordinates',
  text: '猫🙂\r\nlast', version: 1, languageId: 'plaintext', isDirty: true }],
  active: 1, selections: [] });

test('provider locations and related diagnostics reject malformed coordinates without an open document', () => {
  const uri = Uri.file('/target.cpp');
  for (const value of invalid) {
    for (const bad of [{ line: value, character: 0 }, { line: 0, character: value }]) {
      const range = { start: bad, end: { line: 1, character: 0 } };
      assert.throws(() => providerResult('definition', [{ uri, range }]), /coordinates/);
      const diagnostic = { severity: 0, message: 'bad', range: new Range(0, 0, 0, 1),
        relatedInformation: [{ message: 'target', location: { uri, range } }] };
      assert.throws(() => diagnosticResult(diagnostic), /coordinates/);
    }
  }
});

test('diagnostic collection rejects malformed bulk atomically while retaining the original diagnostics', () => {
  const runtime = createApi(() => { throw new Error('Unexpected native request'); }, () => {},
    { session: 1, languageProviders: true });
  runtime.sync(state());
  const api = runtime.forExtension('fixture.coordinates');
  const document = api.workspace.textDocuments[0], uri = document.uri;
  const collection = api.languages.createDiagnosticCollection('coordinates');
  const original = new api.Diagnostic(new Range(0, 0, 0, 1), 'keep');
  collection.set(uri, [original]);
  for (const value of [NaN, Infinity, 0.5]) {
    const malformed = new api.Diagnostic(new Range(0, 0, 0, 1), 'bad');
    malformed.range = { start: { line: 0, character: value }, end: { line: 1, character: 0 } };
    assert.throws(() => collection.set([[uri, [malformed]]]), /coordinates/);
    assert.equal(collection.get(uri)[0], original);
    assert.equal(document.getText(), '猫🙂\r\nlast');
  }
  collection.dispose();
});

test('public Range JSON remains an array while editor edits send explicit stable native DTOs', async () => {
  let serialized;
  const runtime = createApi(async (method, params) => {
    assert.equal(method, 'edit'); serialized = JSON.parse(JSON.stringify(params));
    return { applied: true };
  }, () => {}, { session: 1 });
  runtime.sync(state());
  const range = new Range(0, 1, 0, 3);
  assert.deepEqual(JSON.parse(JSON.stringify(range)), [{ line: 0, character: 1 }, { line: 0, character: 3 }]);
  assert.equal(await runtime.api.window.activeTextEditor.edit(edit => edit.replace(range, '猫')), true);
  assert.deepEqual(serialized.edits, [{ range: {
    start: { line: 0, character: 1 }, end: { line: 0, character: 3 },
  }, newText: '猫' }]);
  assert.equal(serialized.document, 1); assert.equal(serialized.version, 1);
  assert.equal(runtime.api.workspace.textDocuments[0].getText(), '猫🙂\r\nlast');
});

test('unsupported public Position values cannot submit partial native edits', async () => {
  let calls = 0;
  const runtime = createApi(async () => { calls++; return { applied: true }; }, () => {}, { session: 1 });
  runtime.sync(state());
  const editor = runtime.api.window.activeTextEditor;
  for (const value of [NaN, Infinity, 0.5, undefined, null, '1', 0x80000000]) {
    await assert.rejects(editor.edit(edit => {
      edit.insert(new Position(0, 0), 'valid preceding edit');
      edit.insert(new Position(0, value), 'malformed');
    }), /coordinates|Invalid|argument/);
    assert.equal(calls, 0);
    assert.equal(editor.document.getText(), '猫🙂\r\nlast');
    assert.equal(editor.document.version, 1);
  }
});
