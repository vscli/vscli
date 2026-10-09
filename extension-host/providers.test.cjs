'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { TextDocument, Position, Range, Uri } = require('./api-types.cjs');
const types = require('./provider-types.cjs');
const { createProviders, selector, score, normalize } = require('./providers.cjs');
function fixture(timeoutMs = 100) {
  const document = new TextDocument({ id: 1, uri: 'file:///test.cpp', version: 3, languageId: 'cpp', text: '猫🙂x\r\nsecond', isDirty: true });
  const notifications = [], owned = new Map();
  const providers = createProviders({ session: 7, document: id => id === 1 ? document : undefined, timeoutMs,
    notify: (method, params) => notifications.push({ method, params }),
    track: (owner, disposable) => { if (!owned.has(owner)) owned.set(owner, []); owned.get(owner).push(disposable); return disposable; },
  });
  const request = id => ({ session: 7, owner: 'test.extension', provider: id, document: 1, version: 3, position: { line: 0, character: 3 } });
  return { document, providers, notifications, owned, request };
}
function deferred() { let resolve; const promise = new Promise(done => { resolve = done; }); return { resolve, promise }; }

test('provider selectors score exact languages/schemes and reject unsupported filters explicitly', () => {
  const { document } = fixture();
  assert.equal(score(selector('cpp'), document), 10);
  assert.equal(score(selector('*'), document), 5);
  assert.equal(score(selector([{ language: 'rust' }, { language: '*', scheme: 'file' }]), document), 10);
  assert.equal(score(selector({ language: 'cpp', scheme: 'untitled' }), document), 0);
  assert.equal(score(selector({}), document), 0);
  assert.throws(() => selector({ language: 'cpp', pattern: '**/*.cpp' }), /language and scheme/);
  assert.throws(() => selector(Array(33).fill('cpp')), /32 entries/);
});

test('provider dispatch preserves callback receiver, shared dirty document and VS Code UTF-16 position', async () => {
  const { document, providers, request, notifications } = fixture();
  const provider = { marker: 42, provideCompletionItems(doc, position, token, context) {
    assert.equal(this.marker, 42); assert.equal(doc, document); assert.equal(doc.isDirty, true);
    assert.deepEqual(position, new Position(0, 3)); assert.equal(token.isCancellationRequested, false); assert.equal(context.triggerKind, 0);
    const item = new types.CompletionItem('name', types.CompletionItemKind.Variable);
    item.insertText = '名字'; item.range = new Range(0, 3, 0, 4); return new types.CompletionList([item], true);
  } };
  const disposable = providers.forOwner('test.extension').registerCompletionItemProvider({ language: 'cpp', scheme: 'file' }, provider, '.');
  const id = providers.snapshot()[0].id;
  assert.equal(notifications.at(-1).method, 'languageProviders');
  assert.equal(notifications.at(-1).params.session, 7);
  const result = await providers.provide(request(id));
  assert.equal(result.items[0].kind, 6); assert.equal(result.isIncomplete, true);
  assert.equal(result.items[0].textEdit.newText, '名字'); assert.equal(providers.pendingCount(), 0);
  disposable.dispose(); disposable.dispose();
  assert.equal(providers.snapshot().length, 0);
  await assert.rejects(providers.provide(request(id)), /owner\/session/);
});

test('invalid/stale owner, session, document, selector and UTF-16 coordinates never invoke callback', async () => {
  const { providers, request } = fixture(); let invoked = 0;
  providers.forOwner('test.extension').registerHoverProvider('cpp', { provideHover() { invoked++; } });
  const params = request(providers.snapshot()[0].id);
  for (const patch of [{ owner: 'other' }, { session: 6 }, { document: 2 }, { version: 2 }, { position: { line: 0, character: 2 } }, { position: { line: 5, character: 0 } }, { position: { line: 0, character: 5 } }]) {
    await assert.rejects(providers.provide({ ...params, ...patch }));
  }
  assert.equal(invoked, 0); assert.equal(providers.pendingCount(), 0);
});

test('in-flight edit cancels its token and prevents a stale result; disposal cannot revive providers', async () => {
  const { document, providers, request } = fixture(); const wait = deferred(); let token, canceled = 0;
  const registration = providers.forOwner('test.extension').registerHoverProvider('cpp', { provideHover(_, __, value) { token = value; token.onCancellationRequested(() => canceled++); return wait.promise; } });
  const invocation = providers.provide(request(providers.snapshot()[0].id));
  await Promise.resolve();
  document._update({ ...document._snapshot, version: 4, text: 'new' }); providers.documentChanged(); providers.documentChanged();
  assert.equal(token.isCancellationRequested, true); assert.equal(canceled, 1);
  registration.dispose();
  await assert.rejects(invocation, /stale|canceled/); assert.equal(providers.pendingCount(), 1);
  wait.resolve(new types.Hover('old')); await new Promise(done => setImmediate(done)); assert.equal(providers.pendingCount(), 0);
});

test('deadline keeps uncooperative callbacks bounded until they actually settle', async () => {
  const { providers, request } = fixture(5); const waits = [];
  providers.forOwner('test.extension').registerHoverProvider('*', { provideHover(_, __, token) { const wait = deferred(); waits.push({ wait, token }); return wait.promise; } });
  const params = request(providers.snapshot()[0].id);
  for (let i = 0; i < 8; i++) await assert.rejects(providers.provide(params), /deadline/);
  assert.equal(providers.pendingCount(), 8); assert.ok(waits.every(value => value.token.isCancellationRequested));
  await assert.rejects(providers.provide(params), /invocation limit/);
  for (const { wait } of waits) wait.resolve(null);
  await new Promise(done => setImmediate(done));
  assert.equal(providers.pendingCount(), 0);
});

test('owner disposal cancels only that owner and shared registration limits stay bounded', async () => {
  const { providers, request } = fixture(); const a = deferred(), b = deferred();
  providers.forOwner('test.extension').registerHoverProvider('*', { provideHover() { return a.promise; } });
  providers.forOwner('other.extension').registerHoverProvider('*', { provideHover() { return b.promise; } });
  const first = providers.provide(request(providers.snapshot()[0].id));
  const second = providers.provide({ ...request(providers.snapshot()[1].id), owner: 'other.extension' });
  await Promise.resolve(); providers.disposeOwner('test.extension'); a.resolve(null); b.resolve(new types.Hover('ok'));
  await assert.rejects(first, /canceled/); assert.deepEqual((await second).contents, ['ok']);
  for (let i = providers.snapshot().length; i < 128; i++) providers.forOwner('other.extension').registerHoverProvider('*', { provideHover() {} });
  assert.throws(() => providers.forOwner('other.extension').registerHoverProvider('*', { provideHover() {} }), /provider limit/);
  assert.equal(providers.snapshot().length, 128);
});

test('hover, locations, formatting, hierarchical symbols and signatures convert to native LSP shapes', () => {
  const { document } = fixture(); const range = new Range(0, 0, 0, 1);
  assert.deepEqual(normalize('hover', new types.Hover(new types.MarkdownString('**猫**'), range), document).contents, [{ kind: 'markdown', value: '**猫**' }]);
  assert.equal(normalize('definition', new types.Location(Uri.file('/test.cpp'), range), document)[0].uri, 'file:///test.cpp');
  assert.equal(normalize('formatting', [types.TextEdit.replace(range, '犬')], document)[0].newText, '犬');
  const symbol = new types.DocumentSymbol('cat', '', types.SymbolKind.Variable, range, range);
  symbol.children.push(new types.DocumentSymbol('child', '', types.SymbolKind.String, range, range));
  const symbols = normalize('symbols', [symbol], document);
  assert.equal(symbols[0].kind, 13); assert.equal(symbols[0].children[0].kind, 15);
  const signature = new types.SignatureInformation('f(猫,🙂)'); signature.parameters = [new types.ParameterInformation([2, 3]), new types.ParameterInformation([4, 6])];
  const help = new types.SignatureHelp(); help.signatures = [signature]; help.activeParameter = 1;
  assert.equal(normalize('signature', help, document).signatures[0].parameters[1].label[1], 6);
  assert.throws(() => normalize('definition', new types.Location(Uri.parse('https://example.com'), range), document), /local file/);
  assert.throws(() => normalize('formatting', [types.TextEdit.insert(new Position(0, 2), 'x')], document), /splits UTF-16/);
  assert.throws(() => normalize('hover', new types.Hover(Object.assign(new types.MarkdownString('x'), { isTrusted: true })), document), /Trusted/);
});

test('oversized, malformed, unsafe and unsupported results reject without leaking call slots', async () => {
  const { providers, request } = fixture(); let result;
  providers.forOwner('test.extension').registerCompletionItemProvider('*', { provideCompletionItems() { return result; } });
  const params = request(providers.snapshot()[0].id);
  for (result of [Array(301).fill({ label: 'x' }), [{ label: 'x', kind: 25 }], [{ label: 'x', command: { command: 'test' } }], [{ label: 'x', range: { start: { line: 1, character: 2 }, end: { line: 0, character: 0 } } }], Array(300).fill({ label: 'x', detail: 'x'.repeat(4096) })]) {
    await assert.rejects(providers.provide(params)); assert.equal(providers.pendingCount(), 0);
  }
  result = undefined; assert.equal(await providers.provide(params), null);
  result = []; assert.deepEqual(await providers.provide(params), { items: [], isIncomplete: false });
});

test('provider value helpers preserve numeric kinds, snippets, cancellation and constructor semantics', () => {
  assert.equal(types.CompletionItemKind.Function, 2); assert.equal(types.SymbolKind.Function, 11);
  const snippet = new types.SnippetString().appendText('$x}').appendPlaceholder('default').appendTabstop(0);
  assert.equal(snippet.value, '\\$x\\}${1:default}$0');
  const source = new types.CancellationTokenSource(); let count = 0;
  source.token.onCancellationRequested(() => count++); source.cancel(); source.cancel(); source.dispose(true);
  assert.equal(count, 1); assert.equal(source.token.isCancellationRequested, true);
  const location = new types.Location(Uri.file('/test.cpp'), new Position(0, 1)); assert.equal(location.range.isEmpty, true);
});


test('late cancellation listeners run once asynchronously and remain disposable', async () => {
  const parent = new types.CancellationTokenSource(); parent.cancel();
  const source = new types.CancellationTokenSource(parent.token); let observed = 0;
  source.token.onCancellationRequested(() => observed++);
  const discarded = source.token.onCancellationRequested(() => observed += 100); discarded.dispose();
  assert.equal(observed, 0); assert.equal(source.token.isCancellationRequested, true);
  await new Promise(done => setTimeout(done, 5)); assert.equal(observed, 1);
  source.dispose(); parent.dispose();
});


test('signature metadata overload dispatches the original context and validates options', async () => {
  const { providers, request } = fixture(); let observed;
  providers.forOwner('test.extension').registerSignatureHelpProvider('cpp', {
    provideSignatureHelp(document, position, token, context) { observed = context; return new types.SignatureHelp(); },
  }, { triggerCharacters: ['('], retriggerCharacters: [','] });
  const entry = providers.snapshot()[0]; assert.deepEqual(entry.triggers, ['(', ',']);
  await providers.provide(request(entry.id)); assert.deepEqual(observed, { triggerKind: 1, isRetrigger: false });
  assert.throws(() => providers.forOwner('test.extension').registerSignatureHelpProvider('*', { provideSignatureHelp() {} }, { unsupported: true }), /metadata/);
});


test('deprecated completion textEdit retains original replacement range and precedence', () => {
  const { document } = fixture();
  const item = new types.CompletionItem('label'); item.insertText = new types.SnippetString('ignored');
  item.range = new Range(0, 3, 0, 4); item.textEdit = types.TextEdit.replace(new Range(0, 0, 0, 4), 'actual');
  const result = normalize('completion', [item], document).items[0];
  assert.equal(result.insertText, 'actual'); assert.equal(result.insertTextFormat, undefined);
  assert.deepEqual(result.textEdit.range, { start: new Position(0, 0), end: new Position(0, 4) });
});
