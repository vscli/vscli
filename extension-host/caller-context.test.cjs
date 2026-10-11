'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { AsyncLocalStorage } = require('node:async_hooks');
const { createCallerContext } = require('./caller-context.cjs');
const { createApi } = require('./api.cjs');
const { EventEmitter, Disposable } = require('./api-types.cjs');
const { CancellationTokenSource } = require('./cancellation.cjs');

function fixture(sendRequest = async () => ({})) {
  const active = new Set(['test.a', 'test.b']), messages = [];
  const runtime = createApi(sendRequest, (method, params) => messages.push({ method, params }), { session: 17, languageProviders: true });
  runtime.setActivation({ accepts: owner => active.has(owner), extension: owner => ({ id: owner, isActive: active.has(owner) }), facade: () => ({}) });
  runtime.sync({ generation: 1, documents: [{ id: 1, uri: 'untitled:test', text: 'x\n', version: 1, languageId: 'plaintext', isDirty: true }], active: 1,
    selections: [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 0 } }] });
  return { runtime, active, messages, a: runtime.forExtension('test.a'), b: runtime.forExtension('test.b') };
}
const timerTurn = () => new Promise(resolve => setTimeout(resolve, 0));

test('cross-extension command uses registered caller while original active command scope remains unchanged', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), command = new AsyncLocalStorage(), receiver = { base: 9 };
  b.commands.registerCommand('b.inner', function (n) {
    assert.equal(this, receiver); assert.equal(runtime.currentCallerOwner(), 'test.b');
    assert.deepEqual(command.getStore(), { id: 61, owner: 'test.a', active: true }); return this.base + n;
  }, receiver);
  a.commands.registerCommand('a.outer', async () => {
    assert.equal(runtime.currentCallerOwner(), 'test.a');
    const result = await a.commands.executeCommand('b.inner', 3);
    assert.equal(runtime.currentCallerOwner(), 'test.a'); return result;
  });
  const origin = { id: 61, owner: 'test.a', active: true };
  assert.equal(await command.run(origin, () => a.commands.executeCommand('a.outer')), 12);
  assert.throws(() => runtime.currentCallerOwner(), /missing or retired/);
});

test('timer created by actual command keeps caller after command correlation closes', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), command = new AsyncLocalStorage();
  let observed, finish;
  const completion = new Promise(resolve => { finish = resolve; });
  a.commands.registerCommand('a.timer', () => {
    setTimeout(() => { observed = [runtime.currentCallerOwner(), command.getStore().active]; finish(); }, 0);
  });
  const origin = { id: 62, owner: 'test.a', active: true };
  await command.run(origin, () => a.commands.executeCommand('a.timer'));
  origin.active = false;
  await completion;
  assert.deepEqual(observed, ['test.a', false]);
});

test('native document event scope retains thisArg and async descendants without cloning shared documents', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), seen = [], receiver = { marker: 'receiver' };
  a.workspace.onDidChangeTextDocument(function (event) {
    assert.equal(this, receiver); assert.equal(event.document, a.workspace.textDocuments[0]);
    assert.equal(event.document, b.workspace.textDocuments[0]);
    seen.push(runtime.currentCallerOwner());
    Promise.resolve().then(() => seen.push(runtime.currentCallerOwner()));
  }, receiver);
  runtime.sync({ generation: 2, documents: [{ id: 1, uri: 'untitled:test', text: 'xy\n', version: 2, languageId: 'plaintext', isDirty: true }], active: 1,
    selections: [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 0 } }] });
  await Promise.resolve(); assert.deepEqual(seen, ['test.a', 'test.a']);
});

test('public constructors remain identical and emitter listeners capture independent registering owners', { timeout: 8000 }, () => {
  const { runtime, a, b } = fixture(), errors = [], seen = [], receiver = { marker: 7 };
  assert.equal(a.EventEmitter, EventEmitter); assert.equal(b.EventEmitter, EventEmitter);
  assert.equal(a.CancellationTokenSource, CancellationTokenSource); assert.equal(b.CancellationTokenSource, CancellationTokenSource);
  const emitter = new a.EventEmitter(error => errors.push(error));
  runtime.runWithOwner('test.a', () => emitter.event(function (value) { seen.push([runtime.currentCallerOwner(), this.marker, value]); }, receiver));
  runtime.runWithOwner('test.b', () => emitter.event(value => seen.push([runtime.currentCallerOwner(), 0, value])));
  emitter.fire(5); assert.deepEqual(errors, []); assert.deepEqual(seen, [['test.a', 7, 5], ['test.b', 0, 5]]);
});

test('live and already-canceled tokens retain owner, receiver and individual late timer disposal', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), seen = [], receiver = { marker: 8 }, source = new a.CancellationTokenSource();
  runtime.runWithOwner('test.a', () => source.token.onCancellationRequested(function () { seen.push([runtime.currentCallerOwner(), this.marker]); }, receiver));
  source.cancel(); assert.deepEqual(seen, [['test.a', 8]]);
  runtime.runWithOwner('test.a', () => {
    source.token.onCancellationRequested(function () { seen.push([runtime.currentCallerOwner(), this.marker]); }, receiver);
    source.token.onCancellationRequested(() => seen.push('disposed')).dispose();
  });
  await timerTurn(); assert.deepEqual(seen, [['test.a', 8], ['test.a', 8]]);
  source.dispose();
});

test('owner retirement and reacquisition cannot revive stale event, cancellation or timer caller leases', { timeout: 8000 }, async () => {
  const { runtime, active } = fixture(), seen = [], emitter = new EventEmitter(), source = new CancellationTokenSource();
  let finish, error;
  const completion = new Promise(resolve => { finish = resolve; });
  runtime.runWithOwner('test.a', () => {
    emitter.event(() => seen.push('old event'));
    source.cancel(); source.token.onCancellationRequested(() => seen.push('old late token'));
    setTimeout(() => { try { runtime.currentCallerOwner(); } catch (value) { error = value; } finish(); }, 0);
  });
  active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
  runtime.runWithOwner('test.a', () => emitter.event(() => seen.push(runtime.currentCallerOwner())));
  emitter.fire(); await completion; await timerTurn();
  assert.deepEqual(seen, ['test.a']); assert.match(error.message, /missing or retired/);
});

test('one runtime cannot borrow another runtime owner scope', { timeout: 8000 }, () => {
  const first = fixture(), second = fixture();
  first.runtime.runWithOwner('test.a', () => {
    assert.equal(first.runtime.currentCallerOwner(), 'test.a');
    assert.throws(() => second.runtime.currentCallerOwner(), /missing or retired/);
  });
  const forged = createCallerContext(() => {});
  forged.run('test.a', () => assert.throws(() => first.runtime.currentCallerOwner(), /missing or retired/));
});

test('frozen language provider gets original receiver, owner-scoped cancellation, and retained actual symbol lane', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), seen = [];
  let entered, release;
  const started = new Promise(resolve => { entered = resolve; });
  const held = new Promise(resolve => { release = resolve; });
  const provider = Object.freeze({ async provideDocumentSymbols(document, token) {
    assert.equal(this, provider); assert.equal(runtime.currentCallerOwner(), 'test.a');
    assert.equal(document, a.workspace.textDocuments[0]);
    token.onCancellationRequested(() => seen.push(runtime.currentCallerOwner())); entered();
    await held; return [];
  } });
  a.languages.registerDocumentSymbolProvider('plaintext', provider);
  const entry = runtime.providerSnapshot()[0];
  const params = { session: 17, owner: 'test.a', provider: entry.id, document: 1, version: 1, request: 71, symbolRequest: true };
  const work = runtime.provideLanguage(params);
  const rejected = assert.rejects(work, /canceled/);
  await started; runtime.cancelLanguageProvider({ session: 17, owner: 'test.a', request: 71 });
  await rejected; assert.deepEqual(seen, ['test.a']);
  await assert.rejects(runtime.provideLanguage({ ...params, request: 72 }), /callback|pending|running|slot|Symbols/);
  release(); await timerTurn();
  assert.deepEqual(await runtime.provideLanguage({ ...params, request: 73 }), []);
});

test('filesystem provider callbacks use original receiver and registering owner when another owner requests them', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), events = new EventEmitter(), seen = [];
  const provider = Object.freeze({ onDidChangeFile: events.event, watch: () => new Disposable(() => {}),
    stat() { assert.equal(this, provider); seen.push(runtime.currentCallerOwner()); return { type: a.FileType.File, ctime: 1, mtime: 1, size: 1 }; },
    readDirectory() { return []; }, createDirectory() {}, readFile() { return new Uint8Array([7]); },
    writeFile() {}, delete() {}, rename() {},
  });
  a.workspace.registerFileSystemProvider('owned', provider);
  const stat = await b.workspace.fs.stat(a.Uri.parse('owned:/value'));
  assert.equal(stat.size, 1); assert.deepEqual(seen, ['test.a']);
});

test('tree provider callbacks and visibility events bind original owner and receiver', { timeout: 8000 }, async () => {
  const { runtime, a, messages } = fixture(), seen = [], receiver = { marker: 4 };
  runtime.configureSurfaces([{ id: 'test.a', manifest: { contributes: { views: { explorer: [{ id: 'owned.tree', name: 'Owned' }] } } } }]);
  const provider = Object.freeze({ getChildren() { assert.equal(this, provider); seen.push(runtime.currentCallerOwner()); return []; }, getTreeItem() { throw new Error('No nodes'); } });
  const view = a.window.createTreeView('owned.tree', { treeDataProvider: provider });
  view.onDidChangeVisibility(function () { assert.equal(this, receiver); seen.push(runtime.currentCallerOwner()); }, receiver);
  const params = { session: 17, owner: 'test.a', id: 'owned.tree', generation: messages.at(-1).params.generation };
  await runtime.treeChildren(params);
  runtime.treeEvent({ ...params, event: 'visibility', visible: true });
  assert.deepEqual(seen, ['test.a', 'test.a']); view.dispose();
});


test('stale command timer cannot register a callback using a renewed same-owner facade', { timeout: 8000 }, async () => {
  const { runtime, active, a } = fixture();
  let finish, error;
  const completion = new Promise(resolve => { finish = resolve; });
  runtime.runWithOwner('test.a', () => setTimeout(() => {
    try { a.commands.registerCommand('a.stale', () => 'old'); } catch (value) { error = value; }
    finish();
  }, 0));
  active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
  const fresh = runtime.forExtension('test.a');
  fresh.commands.registerCommand('a.fresh', () => runtime.currentCallerOwner());
  await completion;
  assert.match(error.message, /missing or retired/);
  assert(!runtime.commandSnapshot().some(entry => entry.id === 'a.stale'));
  assert.equal(await fresh.commands.executeCommand('a.fresh'), 'test.a');
});

test('cross-owner registration of facade events does not borrow the invoking owner lifetime', { timeout: 8000 }, () => {
  const { runtime, active, a } = fixture(), seen = [];
  runtime.runWithOwner('test.b', () => a.workspace.onDidSaveTextDocument(() => seen.push(runtime.currentCallerOwner())));
  active.delete('test.b'); runtime.disposeOwner('test.b');
  runtime.sync({ generation: 2, documents: [{ id: 1, uri: 'untitled:test', text: 'x\n', version: 1, languageId: 'plaintext', isDirty: false, savedGeneration: 1 }], active: 1,
    selections: [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 0 } }] });
  assert.deepEqual(seen, ['test.a']);
});


function providerParams(runtime, request, extra = {}) {
  const entry = runtime.providerSnapshot().at(-1);
  return { session: 17, owner: entry.owner, provider: entry.id, document: 1, version: 1, request, ...extra };
}
function deferred() { let resolve; const promise = new Promise(value => { resolve = value; }); return { promise, resolve }; }

test('retired actual command timer cannot execute fresh delegates or submit native edits and filesystem writes', { timeout: 8000 }, async () => {
  const fs = require('node:fs/promises'), os = require('node:os'), path = require('node:path');
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'vscli-caller-aba-'));
  const file = path.join(directory, 'retained.txt'), bytes = '猫🙂\r\n';
  await fs.writeFile(file, bytes);
  const sent = [], callbacks = [], { runtime, active, a, b } = fixture(async (method, params) => { sent.push({ method, params }); return {}; });
  const entered = deferred(), gate = deferred(), finished = deferred();
  const editor = a.window.activeTextEditor;
  a.commands.registerCommand('a.old.timer', () => {
    setTimeout(async () => {
      entered.resolve(); await gate.promise;
      try {
        await assert.rejects(a.commands.executeCommand('a.fresh'), /missing or retired/);
        await assert.rejects(a.commands.executeCommand('b.fresh'), /missing or retired/);
        await assert.rejects(a.commands.executeCommand('undo'), /missing or retired/);
        await assert.rejects(a.workspace.openTextDocument(file), /missing or retired/);
        await assert.rejects(editor.edit(() => callbacks.push('edit builder')), /missing or retired/);
        await assert.rejects(a.workspace.fs.writeFile(a.Uri.file(file), new Uint8Array([1, 2])), /missing or retired/);
        finished.resolve();
      } catch (error) { finished.resolve(error); }
    }, 0);
  });
  try {
    await a.commands.executeCommand('a.old.timer'); await entered.promise;
    active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
    const fresh = runtime.forExtension('test.a');
    fresh.commands.registerCommand('a.fresh', () => callbacks.push('a'));
    b.commands.registerCommand('b.fresh', () => callbacks.push('b'));
    gate.resolve(); const error = await finished.promise; if (error) throw error;
    assert.deepEqual(callbacks, []); assert.deepEqual(sent, []);
    assert.equal(await fs.readFile(file, 'utf8'), bytes);
    await fresh.commands.executeCommand('b.fresh'); assert.deepEqual(callbacks, ['b']);
  } finally { gate.resolve(); await fs.rm(directory, { recursive: true, force: true }); }
});

test('queued builtin filesystem work rechecks inherited exact lease before any mutation', { timeout: 8000 }, async () => {
  const fs = require('node:fs/promises'), os = require('node:os'), path = require('node:path');
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'vscli-caller-queued-'));
  const file = path.join(directory, 'retained.txt'); await fs.writeFile(file, 'original\r\n');
  const { runtime, active, a } = fixture(); let pending;
  try {
    runtime.runWithOwner('test.a', () => {
      pending = a.workspace.fs.writeFile(a.Uri.file(file), new Uint8Array([7]));
      active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
    });
    await assert.rejects(pending, /missing or retired/);
    assert.equal(await fs.readFile(file, 'utf8'), 'original\r\n');
  } finally { await fs.rm(directory, { recursive: true, force: true }); }
});

test('registered command returned Thenable getters and assimilation retain owner and receiver', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), seen = [];
  const value = { get then() {
    assert.equal(this, value); seen.push(runtime.currentCallerOwner());
    return function (resolve) {
      assert.equal(this, value); seen.push(runtime.currentCallerOwner());
      Promise.resolve().then(() => { seen.push(runtime.currentCallerOwner()); resolve(41); });
    };
  } };
  a.commands.registerCommand('a.thenable', () => value);
  assert.equal(await a.commands.executeCommand('a.thenable'), 41);
  assert.deepEqual(seen, ['test.a', 'test.a', 'test.a']);
});

test('provider returned Thenable and symbol metadata getters retain exact owner and original receivers', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), seen = [];
  const symbol = { get name() { assert.equal(this, symbol); seen.push(runtime.currentCallerOwner()); return 'owned'; },
    kind: a.SymbolKind.Function, range: new a.Range(0, 0, 0, 1), selectionRange: new a.Range(0, 0, 0, 1), children: [] };
  const result = { get then() {
    assert.equal(this, result); seen.push(runtime.currentCallerOwner());
    return function (resolve) {
      assert.equal(this, result); seen.push(runtime.currentCallerOwner());
      Promise.resolve().then(() => { seen.push(runtime.currentCallerOwner()); resolve([symbol]); });
    };
  } };
  const provider = Object.freeze({ provideDocumentSymbols() { assert.equal(this, provider); return result; } });
  a.languages.registerDocumentSymbolProvider('plaintext', provider);
  const rows = await runtime.provideLanguage(providerParams(runtime, 81, { symbolRequest: true }));
  assert.equal(rows[0].name, 'owned'); assert.deepEqual(seen, ['test.a', 'test.a', 'test.a', 'test.a']);
});

test('completion lazy resolve getters preserve original registration lease and item identity', { timeout: 8000 }, async () => {
  const { runtime, a } = fixture(), seen = [];
  const item = { label: 'owned' };
  const provider = Object.freeze({ provideCompletionItems() { return [item]; }, resolveCompletionItem(original) {
    assert.equal(this, provider); assert.equal(original, item);
    Object.defineProperty(item, 'detail', { get() { assert.equal(this, item); seen.push(runtime.currentCallerOwner()); return 'detail'; } });
    return item;
  } });
  a.languages.registerCompletionItemProvider('plaintext', provider);
  const original = providerParams(runtime, 82, { position: { line: 0, character: 0 } });
  const rows = await runtime.provideLanguage(original);
  const resolved = await runtime.resolveLanguageCompletion({ ...original, request: 83, origin: 82, handle: rows.items[0]._vscliCompletionHandle });
  assert.equal(resolved.detail, 'detail'); assert(seen.length > 0); assert(seen.every(owner => owner === 'test.a'));
});

test('tree returned Thenables and item metadata getters run under original provider lease', { timeout: 8000 }, async () => {
  const { runtime, a, messages } = fixture(), seen = [], element = {};
  runtime.configureSurfaces([{ id: 'test.a', manifest: { contributes: { views: { explorer: [{ id: 'owned.values', name: 'Owned' }] } } } }]);
  const item = { get label() { assert.equal(this, item); seen.push(runtime.currentCallerOwner()); return 'owned'; } };
  const result = { get then() { seen.push(runtime.currentCallerOwner()); return function (resolve) { assert.equal(this, result); resolve([element]); }; } };
  const provider = Object.freeze({ getChildren() { assert.equal(this, provider); return result; }, getTreeItem(value) { assert.equal(value, element); return item; } });
  const view = a.window.createTreeView('owned.values', { treeDataProvider: provider });
  try {
    const rows = await runtime.treeChildren({ session: 17, owner: 'test.a', id: 'owned.values', generation: messages.at(-1).params.generation });
    assert.equal(rows.items[0].label, 'owned'); assert(seen.length > 1); assert(seen.every(owner => owner === 'test.a'));
  } finally { view.dispose(); }
});

test('retiring owner rejects held wire work immediately while actual symbol slot and core release survive', { timeout: 8000 }, async () => {
  const { runtime, active, a, messages } = fixture(), entered = deferred(), gate = deferred();
  a.languages.registerDocumentSymbolProvider('plaintext', { async provideDocumentSymbols() { entered.resolve(); await gate.promise; return []; } });
  const original = providerParams(runtime, 84, { symbolRequest: true });
  const held = runtime.runWithOwner('test.a', () => runtime.provideLanguage(original));
  const rejection = assert.rejects(held, /canceled/);
  try {
    await entered.promise; active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
    const fresh = runtime.forExtension('test.a'); fresh.languages.registerDocumentSymbolProvider('plaintext', { provideDocumentSymbols() { return []; } });
    await rejection;
    assert(!messages.some(message => message.method === 'symbolsReleased' && message.params.request === 84));
    await assert.rejects(runtime.provideLanguage(providerParams(runtime, 85, { symbolRequest: true })), /callback|pending|running|slot|Symbols/);
    gate.resolve(); await timerTurn();
    assert.equal(messages.filter(message => message.method === 'symbolsReleased' && message.params.request === 84).length, 1);
    assert.deepEqual(await runtime.provideLanguage(providerParams(runtime, 86, { symbolRequest: true })), []);
  } finally { gate.resolve(); }
});

test('owner disposal inside returned metadata getter refuses publication and leaves core cleanup functional', { timeout: 8000 }, async () => {
  const { runtime, a, messages } = fixture(); let cleanup = 0;
  const raw = new a.Disposable(() => { cleanup++; });
  const symbol = { get name() { assert.equal(runtime.currentCallerOwner(), 'test.a'); runtime.disposeOwner('test.a'); raw.dispose(); return 'stale'; },
    kind: a.SymbolKind.Function, range: new a.Range(0, 0, 0, 1), selectionRange: new a.Range(0, 0, 0, 1), children: [] };
  a.languages.registerDocumentSymbolProvider('plaintext', { provideDocumentSymbols() { return [symbol]; } });
  const original = providerParams(runtime, 87, { symbolRequest: true });
  await assert.rejects(runtime.provideLanguage(original), /retired|canceled|changed|Stale/);
  await timerTurn(); assert.equal(cleanup, 1);
  assert.equal(messages.filter(message => message.method === 'symbolsReleased' && message.params.request === 87).length, 1);
});


function resultFileSystem(a, events, overrides) {
  return Object.freeze({ onDidChangeFile: events.event, watch() { return new a.Disposable(() => {}); },
    stat() { return { type: a.FileType.File, ctime: 0, mtime: 0, size: 0 }; },
    readDirectory() { return []; }, createDirectory() {}, readFile() { return new Uint8Array(); },
    writeFile() {}, delete() {}, rename() {}, ...overrides });
}

test('cross-owner filesystem returned stat Thenable and directory tuple getters use provider lease while input getters stay requester-owned', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), events = new a.EventEmitter(), seen = [];
  const uri = a.Uri.parse('owned-results:/file');
  Object.defineProperty(uri, 'scheme', { get() { assert.equal(runtime.currentCallerOwner(), 'test.b'); return 'owned-results'; } });
  const stat = { get type() { seen.push(['stat', runtime.currentCallerOwner()]); return a.FileType.File; }, ctime: 1, mtime: 2, size: 3 };
  const thenable = { get then() { seen.push(['then', runtime.currentCallerOwner()]); return function (resolve) { assert.equal(this, thenable); resolve(stat); }; } };
  const tuple = { get 0() { seen.push(['name', runtime.currentCallerOwner()]); return '猫'; }, get 1() { seen.push(['kind', runtime.currentCallerOwner()]); return a.FileType.File; } };
  const rows = [];
  Object.defineProperty(rows, 0, { get() { seen.push(['row', runtime.currentCallerOwner()]); return tuple; } });
  const provider = resultFileSystem(a, events, { stat() { assert.equal(this, provider); return thenable; }, readDirectory() { return rows; } });
  a.workspace.registerFileSystemProvider('owned-results', provider);
  const actual = await runtime.runWithOwner('test.b', () => b.workspace.fs.stat(uri));
  assert.deepEqual(actual, { type: a.FileType.File, ctime: 1, mtime: 2, size: 3, permissions: undefined });
  const directory = await runtime.runWithOwner('test.b', () => b.workspace.fs.readDirectory(uri));
  assert.deepEqual(directory, [['猫', a.FileType.File]]);
  assert.deepEqual(seen, [['then', 'test.a'], ['stat', 'test.a'], ['row', 'test.a'], ['name', 'test.a'], ['kind', 'test.a']]);
});

test('provider stat getter retiring and reacquiring its owner cannot publish under the renewed lease', { timeout: 8000 }, async () => {
  const { runtime, active, a, b } = fixture(), events = new a.EventEmitter();
  const provider = resultFileSystem(a, events, { stat() { return { get type() {
    assert.equal(runtime.currentCallerOwner(), 'test.a'); active.delete('test.a'); runtime.disposeOwner('test.a'); active.add('test.a');
    runtime.forExtension('test.a'); return a.FileType.File;
  }, ctime: 1, mtime: 1, size: 1 }; } });
  a.workspace.registerFileSystemProvider('owned-retirement', provider);
  await assert.rejects(b.workspace.fs.stat(a.Uri.parse('owned-retirement:/file')), /missing or retired/);
});

test('directory normalization rejects oversized and getter-grown outputs without publishing a partial array', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), events = new a.EventEmitter();
  let rows = new Array(65537);
  a.workspace.registerFileSystemProvider('owned-bounds', resultFileSystem(a, events, { readDirectory() { return rows; } }));
  const uri = a.Uri.parse('owned-bounds:/file');
  await assert.rejects(b.workspace.fs.readDirectory(uri), /65536/);
  rows = [];
  Object.defineProperty(rows, 0, { get() { rows.length = 65537; return ['name', a.FileType.File]; } });
  await assert.rejects(b.workspace.fs.readDirectory(uri), /changed during normalization/);
  rows = [['猫', a.FileType.File]];
  assert.deepEqual(await b.workspace.fs.readDirectory(uri), [['猫', a.FileType.File]]);
});


test('cross-provider recursive copy uses provider-owned stat and directory getters without borrowing source scope for target writes', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), seen = [], writes = [], directories = new Set(['/']);
  const source = resultFileSystem(a, new a.EventEmitter(), {
    stat(uri) { return { get type() { seen.push(['source-stat', runtime.currentCallerOwner()]); return uri.path === '/root' ? a.FileType.Directory : a.FileType.File; }, ctime: 1, mtime: 1, size: 8 }; },
    readDirectory() { return [{ get 0() { seen.push(['source-name', runtime.currentCallerOwner()]); return '猫'; }, 1: a.FileType.File }]; },
    readFile() { assert.equal(runtime.currentCallerOwner(), 'test.a'); return new Uint8Array(Buffer.from('猫🙂\r\n')); },
  });
  const target = resultFileSystem(b, new b.EventEmitter(), {
    stat(uri) { if (!directories.has(uri.path)) throw b.FileSystemError.FileNotFound(uri); return { get type() { seen.push(['target-stat', runtime.currentCallerOwner()]); return b.FileType.Directory; }, ctime: 1, mtime: 1, size: 0 }; },
    createDirectory(uri) { assert.equal(runtime.currentCallerOwner(), 'test.b'); directories.add(uri.path); },
    writeFile(uri, bytes) { assert.equal(runtime.currentCallerOwner(), 'test.b'); writes.push([uri.path, Buffer.from(bytes).toString('utf8')]); },
  });
  a.workspace.registerFileSystemProvider('owned-copy-source', source);
  b.workspace.registerFileSystemProvider('owned-copy-target', target);
  await runtime.runWithOwner('test.b', () => b.workspace.fs.copy(a.Uri.parse('owned-copy-source:/root'), b.Uri.parse('owned-copy-target:/target')));
  assert.deepEqual(writes, [['/target/猫', '猫🙂\r\n']]);
  assert(seen.some(([operation]) => operation === 'source-name'));
  assert(seen.some(([operation]) => operation === 'target-stat'));
  assert(seen.every(([operation, owner]) => owner === (operation.startsWith('source') ? 'test.a' : 'test.b')));
});


test('filesystem result DTOs reject object coercion before copy mutation and retain valid fractional negative timestamps and bit flags', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(), events = new a.EventEmitter(); let coercions = 0, writes = 0;
  const object = { valueOf() { coercions++; throw new Error('escaped provider-owned coercion'); } };
  let stat = { type: object, ctime: 0, mtime: 0, size: 1 };
  const provider = resultFileSystem(a, events, { stat() { return stat; }, readDirectory() { return [['bad', object]]; } });
  a.workspace.registerFileSystemProvider('owned-primitives', provider);
  const target = resultFileSystem(b, new b.EventEmitter(), { stat(uri) { throw b.FileSystemError.FileNotFound(uri); }, writeFile() { writes++; } });
  b.workspace.registerFileSystemProvider('owned-no-write', target);
  const uri = a.Uri.parse('owned-primitives:/file');
  await assert.rejects(b.workspace.fs.stat(uri), /Invalid filesystem result type/);
  await assert.rejects(b.workspace.fs.copy(uri, b.Uri.parse('owned-no-write:/file')), /Invalid filesystem result type/);
  await assert.rejects(b.workspace.fs.readDirectory(uri), /Invalid filesystem result type/);
  for (const key of ['ctime', 'mtime', 'size', 'permissions']) {
    stat = { type: a.FileType.File, ctime: 0, mtime: 0, size: 1, [key]: object };
    await assert.rejects(b.workspace.fs.stat(uri), /Invalid filesystem/);
  }
  assert.equal(coercions, 0); assert.equal(writes, 0);
  stat = { type: a.FileType.File | a.FileType.SymbolicLink, ctime: -1.25, mtime: -2.5, size: 7, permissions: a.FilePermission.Readonly };
  assert.deepEqual(await b.workspace.fs.stat(uri), stat);
});

test('filesystem directory name admission rejects oversized source strings and remains usable for valid Unicode output', { timeout: 8000 }, async () => {
  const { runtime, a, b } = fixture(); let rows = [['x'.repeat(8 * 1024 * 1024 + 1), a.FileType.File]];
  a.workspace.registerFileSystemProvider('owned-long-name', resultFileSystem(a, new a.EventEmitter(), { readDirectory() { return rows; } }));
  const uri = a.Uri.parse('owned-long-name:/file');
  await assert.rejects(b.workspace.fs.readDirectory(uri), /8 MiB/);
  rows = [['猫🙂', a.FileType.File | a.FileType.SymbolicLink]];
  assert.deepEqual(await b.workspace.fs.readDirectory(uri), rows);
});
