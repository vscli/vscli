'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const {createApi} = require('./api.cjs');
const {Uri, Position, Range} = require('./api-types.cjs');

function held() {
  let resolve;
  const promise = new Promise(value => { resolve = value; });
  return {promise, resolve};
}

function fixture() {
  const receipt = held(), calls = [];
  const document = {id: 17, uri: Uri.file('/tmp/marker-semantics.cpp').toString(),
    text: '猫🙂alpha\r\nsecond\r\n', version: 7, languageId: 'cpp', languageEpoch: 2,
    eol: 2, isDirty: true};
  const runtime = createApi((method, params) => {
    calls.push({method, params});
    return receipt.promise;
  }, () => {}, {session: 31});
  let generation = 0;
  const sync = () => runtime.sync({generation: ++generation, documents: [document],
    editors: [{id: 'original-pane', document: 17, viewColumn: 1,
      selections: [{anchor: {line: 0, character: 3}, active: {line: 0, character: 3}}]}],
    activeEditor: 'original-pane', active: 17});
  sync();
  const api = runtime.forExtension('marker.owner');
  return {runtime, document, sync, calls, receipt, editor: api.window.activeTextEditor};
}

test('public edit insert/delete force markers while replace omits flag, awaiting original native ACK', async () => {
  const f = fixture(), editor = f.editor;
  // Public metadata cannot supply the target or version used by the edit builder.
  editor.document._snapshot = {id: 99, version: 100};
  let settled = false;
  const completion = editor.edit(builder => {
    builder.insert(new Position(0, 8), 'λ');
    builder.delete(new Range(1, 0, 1, 2));
    builder.replace(new Range(0, 3, 0, 5), 'β');
  }).then(result => { settled = true; return result; });
  assert.deepEqual(f.calls, [{method: 'edit', params: {
    session: 31, owner: 'marker.owner', document: 17, version: 7, editor: 'original-pane',
    edits: [
      {range: {start: {line: 0, character: 8}, end: {line: 0, character: 8}}, newText: 'λ', forceMoveMarkers: true},
      {range: {start: {line: 1, character: 0}, end: {line: 1, character: 2}}, newText: '', forceMoveMarkers: true},
      {range: {start: {line: 0, character: 3}, end: {line: 0, character: 5}}, newText: 'β'},
    ],
  }}]);
  await Promise.resolve();
  assert.equal(settled, false);
  assert.equal(editor.document.getText(), '猫🙂alpha\r\nsecond\r\n');
  // Only the native mirror changes document text; accepting the edit has no local text simulation.
  f.document.text = '猫🙂βphaλ\r\ncond\r\n';
  f.document.version = 8;
  f.sync();
  f.receipt.resolve({applied: true});
  assert.equal(await completion, true);
  assert.equal(settled, true);
  assert.equal(editor.document.version, 8);
  assert.equal(editor.document.getText(), f.document.text);
});

test('marker edits cannot send partial work after their captured source version changes', async () => {
  const f = fixture();
  await assert.rejects(f.editor.edit(builder => {
    builder.insert(new Position(0, 3), 'λ');
    builder.delete(new Range(1, 0, 1, 2));
    f.document.version++;
    f.sync();
    builder.replace(new Range(0, 3, 0, 5), 'β');
  }), /source changed/);
  assert.deepEqual(f.calls, []);
  assert.equal(f.editor.document.getText(), '猫🙂alpha\r\nsecond\r\n');
});

test('accepted marker edit ACK cannot reacquire a retired original caller lease', async () => {
  const f = fixture();
  const completion = f.editor.edit(builder => builder.insert(new Position(0, 3), 'λ'));
  const rejected = assert.rejects(completion, /retired/);
  assert.equal(f.calls[0].params.owner, 'marker.owner');
  assert.equal(f.calls[0].params.version, 7);
  f.runtime.disposeOwner('marker.owner');
  f.runtime.forExtension('marker.owner');
  f.receipt.resolve({applied: true});
  await rejected;
  assert.equal(f.calls.length, 1);
});

test('public edits normalize all delimiters using private captured EOL before native dispatch', async () => {
  for (const eol of [1, 2]) {
    const f=fixture();f.document.eol=eol;f.document.version++;f.sync();
    // Public metadata cannot replace the original native delimiter contract.
    f.editor.document._snapshot={eol:eol===1?2:1};
    const completion=f.editor.edit(builder=>builder.insert(new Position(0,0),'α\nβ\rγ\r\n'));
    assert.equal(f.calls.length,1);assert.equal(f.calls[0].params.edits[0].newText,eol===2?'α\r\nβ\r\nγ\r\n':'α\nβ\nγ\n');
    assert.equal(f.calls[0].params.edits[0].forceMoveMarkers,true);f.receipt.resolve({applied:true});assert.equal(await completion,true);
  }
});
test('CRLF expansion exceeding the native mirror budget rejects before any native edit request',async()=>{
  const f=fixture();const original=f.editor.document.getText();
  await assert.rejects(f.editor.edit(builder=>builder.insert(new Position(0,0),'\n'.repeat(2*1024*1024+1))),/4 MiB mirror budget/);
  assert.equal(f.calls.length,0);assert.equal(f.editor.document.getText(),original);
});
