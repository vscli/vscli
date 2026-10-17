'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');

function fixture(handle = () => { throw new Error('Unexpected native request'); }, nativeUri) {
  const calls = [], root = '/tmp/cache-workspace';
  const uri = nativeUri ?? Uri.file(`${root}/main.cpp`).toString();
  const snapshot = { id: 7, uri, version: 1, text: '猫🙂\r\noriginal', languageId: 'cpp', isDirty: false };
  let generation = 0;
  const runtime = createApi((method, params) => { calls.push({method, params}); return handle(method, params); }, () => {}, { session: 8 });
  runtime.configure(root, [], [{}, {}]);
  const sync = (documents = [snapshot]) => runtime.sync({ generation: ++generation, documents, active: null, selections: [] });
  sync();
  const api = runtime.forExtension('cache.owner');
  return { runtime, api, root, uri, snapshot, sync, calls, doc: api.workspace.textDocuments[0] };
}

test('absolute/relative paths and file URIs return the same hidden model across extension owners', async () => {
  const f = fixture();
  let opened = 0, closed = 0;
  f.api.workspace.onDidOpenTextDocument(() => opened++);
  f.api.workspace.onDidCloseTextDocument(() => closed++);
  for (const input of ['main.cpp', './folder/../main.cpp', `${f.root}/main.cpp`, Uri.file(`${f.root}/main.cpp`)]) {
    assert.equal(await f.api.workspace.openTextDocument(input, {encoding: 'utf8'}), f.doc);
  }
  assert.equal(await f.runtime.forExtension('other.owner').workspace.openTextDocument('main.cpp'), f.doc);
  assert.deepEqual([opened, closed, f.calls.length], [0, 0, 0]);
  assert.equal(f.api.window.activeTextEditor, undefined);
  assert.deepEqual(f.api.window.visibleTextEditors, []);
});

test('cached opens retain current dirty Unicode CRLF text without asking the filesystem to reload', async () => {
  const f = fixture();
  f.sync([{...f.snapshot, version: 2, text: '猫🙂\r\nunsaved', isDirty: true}]);
  const doc = await f.api.workspace.openTextDocument('main.cpp');
  assert.equal(doc, f.doc);
  assert.equal(doc.getText(), '猫🙂\r\nunsaved');
  assert.equal(doc.version, 2);
  assert.equal(doc.isDirty, true);
  assert.equal(f.calls.length, 0);
});

test('public document metadata and later URI factory overrides cannot redirect the private cache', async () => {
  const f = fixture(), file = Uri.file;
  f.doc.uri.path = '/forged.cpp';
  f.doc._snapshot = {id: 99, uri: 'file:///forged.cpp', version: 99, text: 'forged', isClosed: true};
  Object.defineProperty(f.doc, 'fileName', {value: '/forged.cpp'});
  try {
    Uri.file = () => { throw new Error('Untrusted public URI factory'); };
    assert.equal(await f.api.workspace.openTextDocument('main.cpp'), f.doc);
    assert.equal(f.calls.length, 0);
  } finally { Uri.file = file; }
});

test('cached file opens do not occupy a native service slot when all eight slots are busy', async () => {
  const held = [], f = fixture(() => new Promise((resolve, reject) => held.push({resolve, reject})));
  const settling = Promise.allSettled(Array.from({length: 8}, (_, i) => f.api.workspace.openTextDocument(`missing-${i}.cpp`)));
  try {
    assert.equal(await f.api.workspace.openTextDocument('main.cpp'), f.doc);
    await assert.rejects(f.api.workspace.openTextDocument('ninth.cpp'), /queue limit/);
    assert.equal(f.calls.length, 8);
  } finally { for (const entry of held) entry.reject(new Error('Canceled fixture load')); }
  assert.equal((await settling).filter(result => result.status === 'rejected').length, 8);
});

test('cached opens validate encoding once and cannot revive a disposed caller through argument getters', async () => {
  const f = fixture();
  let reads = 0;
  assert.equal(await f.api.workspace.openTextDocument('main.cpp', {get encoding() {reads++; return 'utf8';}}), f.doc);
  assert.equal(reads, 1);
  await assert.rejects(f.api.workspace.openTextDocument('main.cpp', {encoding: 'utf16le'}), /encoding/);
  await assert.rejects(f.api.workspace.openTextDocument('main.cpp', {get encoding() {
    f.runtime.disposeOwner('cache.owner'); f.runtime.forExtension('cache.owner'); return 'utf8';
  }}), /retired/);
  assert.equal(f.calls.length, 0);
});

test('same-URI model replacement during options capture cannot satisfy the original cached identity', async () => {
  const f = fixture();
  await assert.rejects(f.api.workspace.openTextDocument('main.cpp', {get encoding() {
    f.sync([]); f.sync(); return 'utf8';
  }}), /Cached native document mirror was retired/);
  assert.notEqual(f.api.workspace.textDocuments[0], f.doc);
  assert.equal(f.doc.isClosed, true);
  assert.equal(f.calls.length, 0);
});

test('file URI getters are captured once and a legitimate text advance retains the original model', async () => {
  const f = fixture(), reads = {};
  const fields = {scheme: 'file', authority: '', path: `${f.root}/main.cpp`, query: '', fragment: ''};
  const input = {toString() {throw new Error('Untrusted formatter');}, get fsPath() {throw new Error('Untrusted path');}};
  for (const [key, value] of Object.entries(fields)) Object.defineProperty(input, key, {get() {
    reads[key] = (reads[key] || 0) + 1;
    if (key === 'path') f.sync([{...f.snapshot, version: 2, text: 'advanced猫🙂\r\n', isDirty: true}]);
    return value;
  }});
  assert.equal(await f.api.workspace.openTextDocument(input), f.doc);
  assert.equal(f.doc.getText(), 'advanced猫🙂\r\n');
  assert.deepEqual(reads, {scheme: 1, authority: 1, path: 1, query: 1, fragment: 1});
  assert.equal(f.calls.length, 0);
});

test('canonical aliases and query/fragment-qualified URIs are not guessed equivalent by the cache', async () => {
  const f = fixture(() => ({document: 7}));
  for (const input of [`${f.root}/alias.cpp`, Uri.file(`${f.root}/main.cpp`).with({query: 'q'}), Uri.file(`${f.root}/main.cpp`).with({fragment: 'f'})]) {
    assert.equal(await f.api.workspace.openTextDocument(input), f.doc);
  }
  assert.equal(f.calls.length, 3);
  assert(f.calls.every(call => call.method === 'nativeDocumentOpen'));
});


test('native file URI spelling and upstream drive/percent formatting share the original cache without new acquisitions', async () => {
  for (const uri of ['file:///C:/work/source.cpp', 'file:///tmp/cache-workspace/a:b@c.cpp', 'file:///tmp/cache-workspace/a%20b.cpp']) {
    const f = fixture(undefined, uri);
    const canonical = Uri.parse(uri);
    assert.equal(await f.api.workspace.openTextDocument(canonical), f.doc);
    assert.equal(f.calls.length, 0);
    assert.equal(f.api.workspace.textDocuments.length, 1);
    assert.equal(f.doc.isClosed, false);
  }
});
