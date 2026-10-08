'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const initial = () => ({ generation: 1, documents: [{ id: 1, uri: 'untitled:test', text: 'b\na\n', version: 1, languageId: 'plaintext', isDirty: true }], active: 1, selections: [{ anchor: { line: 0, character: 0 }, active: { line: 2, character: 0 } }] });

test('native edit approval updates the existing document before resolving the edit promise', async () => {
  let call, builder;
  const after = initial(); after.generation = 2; after.documents[0].text = 'a\nb\n'; after.documents[0].version = 2;
  const runtime = createApi(async (method, params) => { call = { method, params }; runtime.sync(after); return { applied: true }; }, () => {});
  runtime.sync(initial());
  const editor = runtime.api.window.activeTextEditor;
  const document = editor.document;
  let observed;
  runtime.api.workspace.onDidChangeTextDocument(event => { observed = [event.document === document, document.getText()]; });
  assert.equal(await editor.edit(edit => { builder = edit; edit.replace(new runtime.api.Range(0, 0, 2, 0), 'a\nb\n'); }), true);
  assert.equal(call.params.version, 1);
  assert.equal(call.params.document, 1);
  assert.equal(call.method, 'edit');
  assert.deepEqual(observed, [true, 'a\nb\n']);
  assert.equal(document.version, 2);
  assert.throws(() => builder.insert(new runtime.api.Position(0, 0), 'late'), /only valid during/);
});

test('rejected edit leaves the mirror intact and callbacks cannot partially submit', async () => {
  let requests = 0;
  const runtime = createApi(async () => { requests++; return { applied: false }; }, () => {});
  runtime.sync(initial());
  const editor = runtime.api.window.activeTextEditor;
  await assert.rejects(editor.edit(edit => { edit.delete(new runtime.api.Range(0, 0, 1, 0)); throw new Error('failed'); }), /failed/);
  assert.equal(requests, 0);
  assert.equal(await editor.edit(edit => edit.delete(new runtime.api.Range(0, 0, 1, 0))), false);
  assert.equal(editor.document.getText(), 'b\na\n');
});

test('commands preserve thisArg, disposal, arguments and explicit unsupported API failures', async () => {
  const runtime = createApi(() => {}, () => {});
  const command = runtime.api.commands.registerCommand('test.add', function(n) { return this.base + n; }, { base: 4 });
  assert.equal(await runtime.api.commands.executeCommand('test.add', 3), 7);
  command.dispose();
  await assert.rejects(runtime.api.commands.executeCommand('test.add', 3), /unregistered/);
  assert.throws(() => runtime.api.window.createWebviewPanel(), /not implemented: window.createWebviewPanel/);
});

test('closed documents and active editor changes are observable after synchronized state', () => {
  const runtime = createApi(() => {}, () => {});
  runtime.sync(initial());
  const document = runtime.api.workspace.textDocuments[0];
  let observed;
  runtime.api.workspace.onDidCloseTextDocument(doc => { observed = [doc.isClosed, runtime.api.window.activeTextEditor]; });
  runtime.sync({ generation: 2, documents: [], selections: [], active: null });
  assert.equal(document.isClosed, true);
  assert.deepEqual(observed, [true, undefined]);
});

test('an older edit response cannot replace a newer document notification', async () => {
  const old = initial();
  const runtime = createApi(async () => ({ applied: true, state: old }), () => {});
  runtime.sync(old);
  const editor = runtime.api.window.activeTextEditor;
  const edit = editor.edit(() => {});
  runtime.sync({ ...old, generation: 2, documents: [{ ...old.documents[0], version: 2, text: 'newer' }] });
  await edit;
  assert.equal(editor.document.getText(), 'newer');
});

test('configuration reads preserve explicit null and isolate object defaults from mutation', () => {
  const runtime = createApi(() => {}, () => {});
  runtime.configure(process.cwd(), { properties: { 'test.null': { default: null }, 'test.object': { default: { a: 1 } } } });
  const configuration = runtime.api.workspace.getConfiguration('test');
  assert.equal(configuration.get('null', 'fallback'), null);
  configuration.get('object').a = 2;
  assert.deepEqual(configuration.get('object'), { a: 1 });
  assert.equal(configuration.get('missing', 'fallback'), 'fallback');
});

test('metadata updates retain text, offsets and identity without emitting text changes', () => {
  const runtime = createApi(() => {}, () => {});
  const state = initial();
  state.documents[0].text = '猫🙂\r\nlast';
  runtime.sync(state);
  const document = runtime.api.workspace.textDocuments[0];
  let changes = 0;
  runtime.api.workspace.onDidChangeTextDocument(() => changes++);
  const { text, ...metadata } = state.documents[0];
  runtime.sync({ ...state, generation: 2, documents: [{ ...metadata, uri: 'untitled:renamed', version: 2, isDirty: false }],
    selections: [{ anchor: { line: 1, character: 2 }, active: { line: 1, character: 2 } }] });
  assert.equal(runtime.api.window.activeTextEditor.document, document);
  assert.equal(document.getText(), text);
  assert.equal(document.offsetAt(new runtime.api.Position(1, 0)), 5);
  assert.equal(document.eol, 2);
  assert.equal(document.isDirty, false);
  assert.equal(document.uri.toString(), 'untitled:renamed');
  assert.equal(changes, 0);
  assert.equal(runtime.api.window.activeTextEditor.selection.active.line, 1);
});

test('edit acknowledgement cannot lose text before a following selection-only notification', async () => {
  let acknowledge;
  const runtime = createApi(() => new Promise(resolve => { acknowledge = resolve; }), () => {});
  const state = initial();
  runtime.sync(state);
  const editor = runtime.api.window.activeTextEditor;
  let changed;
  runtime.api.workspace.onDidChangeTextDocument(event => { changed = event.contentChanges[0]; });
  const edit = editor.edit(builder => builder.replace(new runtime.api.Range(0, 0, 2, 0), 'edited🙂'));
  runtime.sync({ ...state, generation: 2, documents: [{ ...state.documents[0], text: 'edited🙂', version: 2 }] });
  const { text, ...metadata } = state.documents[0];
  runtime.sync({ ...state, generation: 3, documents: [{ ...metadata, version: 2 }],
    selections: [{ anchor: { line: 0, character: 8 }, active: { line: 0, character: 8 } }] });
  acknowledge({ applied: true });
  assert.equal(await edit, true);
  assert.equal(editor.document.getText(), 'edited🙂');
  assert.equal(editor.selection.active.character, 8);
  assert.equal(changed.rangeLength, text.length);
  assert.equal(changed.text, 'edited🙂');
  assert.deepEqual(changed.range.end, new runtime.api.Position(2, 0));
});
