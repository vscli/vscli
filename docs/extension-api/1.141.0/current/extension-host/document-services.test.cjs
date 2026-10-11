'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const { Uri, Range, Position } = require('./api-types.cjs');
function initial() { return { generation: 1, documents: [], active: null, selections: [] }; }
test('hidden open mirrors before events and promise; show reuses identity and native command mirrors', async () => {
  const calls = [];
  let runtime;
  runtime = createApi(async (method, params) => {
    calls.push([method, params]);
    if (method === 'nativeDocumentOpen') {
      runtime.sync({ generation: 2, documents: [{ id: 7, uri: Uri.file('/tmp/file.txt').toString(), version: 1, text: 'α😀\r\nnext', languageId: 'plaintext', isDirty: false }], active: null, selections: [] });
      return { document: 7 };
    }
    if (method === 'nativeDocumentShow') {
      runtime.sync({ generation: 3, documents: [{ id: 7, uri: Uri.file('/tmp/file.txt').toString(), version: 1, languageId: 'plaintext', isDirty: false }], active: 7, selections: [{ anchor: { line: 0, character: 1 }, active: { line: 0, character: 3 } }] });
      return { document: 7 };
    }
    return null;
  }, () => {}, { session: 9 });
  runtime.sync(initial());
  const api = runtime.forExtension('test.owner');
  let opened;
  api.workspace.onDidOpenTextDocument(doc => { opened = doc; assert.equal(api.window.activeTextEditor, undefined); });
  const doc = await api.workspace.openTextDocument('/tmp/file.txt');
  assert.equal(doc, opened); assert.equal(doc.getText(), 'α😀\r\nnext');
  assert.equal(api.window.activeTextEditor, undefined); assert.deepEqual(api.window.visibleTextEditors, []);
  const editor = await api.window.showTextDocument(doc, { preview: false, selection: new Range(new Position(0, 1), new Position(0, 3)) });
  assert.equal(editor.document, doc); assert.equal(editor, api.window.activeTextEditor);
  assert.equal(editor.selection.end.character, 3);
  await api.commands.executeCommand('undo');
  assert.equal(calls[2][1].generation, 3); assert.equal(calls[2][1].owner, 'test.owner');
  const count = calls.length;
  await assert.rejects(api.window.showTextDocument('/new/file', { preserveFocus: true }), /preserveFocus/);
  await assert.rejects(api.window.showTextDocument('/new/file', { viewColumn: api.ViewColumn.One }), /active editor group/);
  await assert.rejects(api.workspace.openTextDocument({ content: 'no' }), /untitled/);
  await assert.rejects(api.commands.executeCommand('undo', 1), /arguments/);
  await assert.rejects(api.commands.executeCommand('workbench.action.tasks.runTask'), /unregistered/);
  assert.equal(calls.length, count);
});
test('document service queue is bounded and malformed input never requests native work', async () => {
  const held = [];
  const runtime = createApi(() => new Promise((resolve,reject) => held.push({resolve,reject})), () => {}, {session:1});
  runtime.sync(initial());
  const api = runtime.forExtension('test.owner');
  const requests = Array.from({length:8},()=>api.workspace.openTextDocument('/tmp/file'));
  const settling = Promise.allSettled(requests);
  await assert.rejects(api.workspace.openTextDocument('/tmp/ninth'), /limit/);
  for (const promise of held) promise.reject(new Error('cancelled'));
  assert.equal((await settling).length, 8);
});
