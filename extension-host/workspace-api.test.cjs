'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { createApi } = require('./api.cjs');
test('workspace facade retains getters and folder identity while exposing authoritative single-folder metadata', () => {
  const runtime = createApi(() => {}, () => {}), v = runtime.forExtension('test');
  assert.equal(v.workspace.workspaceFolders, undefined);
  const root = path.resolve('workspace with space');
  runtime.configure(root, [], []);
  const first = v.workspace.workspaceFolders[0];
  const inside = v.Uri.joinPath(first.uri, 'source', '猫#.cpp');
  assert.equal(v.workspace.name, path.basename(root)); assert.equal(v.workspace.rootPath, root);
  assert.equal(v.workspace.workspaceFile, undefined);
  assert.equal(v.workspace.getWorkspaceFolder(inside), first);
  assert.equal(runtime.api.workspace.getWorkspaceFolder(inside), first);
  assert.equal(v.workspace.asRelativePath(inside), 'source/猫#.cpp');
  assert.equal(v.workspace.getWorkspaceFolder(v.Uri.file(root + '-sibling/a.cpp')), undefined);
  const array = v.workspace.workspaceFolders; array.length = 0;
  assert.equal(v.workspace.workspaceFolders.length, 1);
  runtime.configure(root, [], []); assert.equal(v.workspace.workspaceFolders[0], first);
  const replacement = path.resolve('replacement'); runtime.configure(replacement, [], []);
  assert.equal(v.workspace.name, 'replacement'); assert.equal(v.workspace.rootPath, replacement);
  assert.equal(v.workspace.getWorkspaceFolder(inside), undefined);
});
test('unimplemented workspace trust, mutation and native folder events remain explicit', () => {
  const runtime = createApi(() => {}, () => {}), v = runtime.forExtension('test');
  for (const key of ['isTrusted', 'onDidGrantWorkspaceTrust', 'onDidChangeWorkspaceFolders', 'updateWorkspaceFolders']) {
    assert.throws(() => v.workspace[key], new RegExp(`not implemented: workspace.${key}`));
  }
});
test('extension mutations of public folder URI objects cannot redirect native relative document requests', async () => {
  const root = path.resolve('native-root'), calls = [];
  const runtime = createApi(async (method, params) => { calls.push([method, params]); return { document: 1 }; }, () => {});
  runtime.configure(root, [], []);
  const v = runtime.forExtension('test'), uri = v.Uri.file(path.join(root, 'file.txt'));
  runtime.sync({ generation: 1, documents: [{ id: 1, uri: uri.toString(), version: 1, text: 'preserved', languageId: 'plaintext', isDirty: true }], active: null, selections: [] });
  v.workspace.workspaceFolders[0].uri.path = '/extension-rewrite';
  const doc = await v.workspace.openTextDocument('file.txt');
  assert.equal(calls[0][0], 'nativeDocumentOpen'); assert.equal(calls[0][1].args.path, path.join(root, 'file.txt'));
  assert.equal(doc.uri.toString(), uri.toString()); assert.equal(doc.getText(), 'preserved'); assert.equal(doc.isDirty, true);
});
