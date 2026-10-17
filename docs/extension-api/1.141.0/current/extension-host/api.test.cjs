'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const initial = () => ({ generation: 1, documents: [{ id: 1, uri: 'untitled:test', text: 'b\na\n', version: 1, languageId: 'plaintext', isDirty: true }], active: 1, selections: [{ anchor: { line: 0, character: 0 }, active: { line: 2, character: 0 } }] });

test('public symbol constructors expose hierarchy, both standard location overloads and geometry validation', () => {
  const api = createApi(() => {}, () => {}).api;
  const range = new api.Range(0,0,0,4), selection = new api.Range(0,1,0,3), uri = api.Uri.parse('untitled:test');
  const parent = new api.DocumentSymbol('猫 class','detail',api.SymbolKind.Class,range,selection);
  const child = new api.DocumentSymbol('🙂 method','',api.SymbolKind.Method,selection,selection);
  parent.children.push(child); api.DocumentSymbol.validate(parent);
  assert.equal(parent.children[0],child); assert.equal(parent.range,range); assert.equal(parent.selectionRange,selection);
  assert.equal(api.SymbolKind.Class,4); assert.equal(api.SymbolKind.Function,11); assert.equal(api.SymbolTag.Deprecated,1);
  const location = new api.Location(uri,selection);
  const flat = new api.SymbolInformation('flat',api.SymbolKind.Function,'container',location);
  const old = new api.SymbolInformation('flat',api.SymbolKind.Function,selection,uri,'container');
  assert.equal(flat.location,location); assert.equal(old.location.uri,uri); assert.equal(old.location.range,selection);
  assert.equal(flat.containerName,'container'); assert.equal(old.containerName,'container');
  assert.throws(()=>new api.DocumentSymbol('bad','',api.SymbolKind.Class,selection,range),/contained/);
  assert.throws(()=>new api.DocumentSymbol('','',api.SymbolKind.Class,range,selection),/nonempty/);
  assert.throws(()=>new api.SymbolInformation('',api.SymbolKind.Class,'container',location),/nonempty/);
  parent.children.push(parent); assert.throws(()=>api.DocumentSymbol.validate(parent),/depth/);
});

test('coalesced edit and undo notifies a fresh version even with unchanged bytes', () => {
  const runtime = createApi(() => {}, () => {});
  runtime.sync(initial());
  const document = runtime.api.workspace.textDocuments[0], events = [];
  runtime.api.workspace.onDidChangeTextDocument(event => events.push(event));
  const state = initial(); state.generation = 2; state.documents[0].version = 2;
  runtime.sync(state);
  assert.equal(events.length, 1);
  assert.equal(events[0].document, document);
  assert.equal(document.version, 2);
  assert.deepEqual(events[0].contentChanges.map(change => [change.rangeOffset,change.rangeLength,change.text]), [[0,4,'b\na\n']]);
  state.generation = 3; state.documents[0].savedGeneration = 1;
  state.selections = [{anchor:{line:1,character:0},active:{line:1,character:0}}];
  runtime.sync(state);
  assert.equal(events.length, 1, 'Save and cursor updates without a text version remain quiet');
});

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

test('extension facades share documents, route commands and retain edit owners', async () => {
  const calls = [], registries = [];
  const runtime = createApi(async (method, params) => { calls.push({ method, params }); return { applied: false }; }, (method, params) => {
    if (method === 'commands') registries.push(params);
  }, { session: 17, reservedCommands: ['type'] });
  runtime.sync(initial());
  const a = runtime.forExtension('test.a'), b = runtime.forExtension('test.b');
  assert.equal(a.workspace.textDocuments[0], b.workspace.textDocuments[0]);
  assert.equal(a.window.activeTextEditor.document, b.window.activeTextEditor.document);
  a.commands.registerCommand('a.run', () => a.window.activeTextEditor.edit(edit => edit.insert(new a.Position(0, 0), 'A')));
  b.commands.registerCommand('b.run', () => b.commands.executeCommand('a.run'));
  assert.equal(await b.commands.executeCommand('b.run'), false);
  assert.equal(calls[0].params.owner, 'test.a');
  assert.equal(calls[0].params.session, 17);
  assert.throws(() => b.commands.registerCommand('a.run', () => {}), /registered by test.a/);
  assert.throws(() => b.commands.registerCommand('type', () => {}), /reserved/);
  let observed;
  b.workspace.onDidChangeTextDocument(event => { observed = [event.document === a.workspace.textDocuments[0], a.window.activeTextEditor.document.version]; });
  runtime.sync({ ...initial(), generation: 2, documents: [{ ...initial().documents[0], version: 2, text: 'shared' }] });
  assert.deepEqual(observed, [true, 2]);
  runtime.disposeOwner('test.a');
  assert.deepEqual(await b.commands.getCommands(), [...require('./document-services.cjs').NATIVE_COMMANDS, 'b.run']);
  assert.deepEqual(registries.at(-1), { session: 17, commands: [{ id: 'b.run', owner: 'test.b' }] });
});

test('shared registration limits include listeners and disposal releases the budget', () => {
  const runtime = createApi(() => {}, () => {});
  const api = runtime.forExtension('test.bounded');
  for (let i = 0; i < 4096; i++) api.workspace.onDidChangeTextDocument(() => {});
  assert.throws(() => api.workspace.onDidCloseTextDocument(() => {}), /registration limit/);
  runtime.disposeOwner('test.bounded');
  assert.doesNotThrow(() => runtime.forExtension('test.bounded').workspace.onDidCloseTextDocument(() => {}));
});

test('recursive extension command calls are bounded and release their execution slots', async () => {
  const runtime = createApi(() => {}, () => {});
  const api = runtime.forExtension('test.recursive');
  api.commands.registerCommand('loop', () => api.commands.executeCommand('loop'));
  await assert.rejects(api.commands.executeCommand('loop'), /execution limit/);
  api.commands.registerCommand('after', () => 'usable');
  assert.equal(await api.commands.executeCommand('after'), 'usable');
});

test('oversized edit builders reject without submitting a partial transaction', async () => {
  let requests = 0;
  const runtime = createApi(() => { requests++; }, () => {});
  runtime.sync(initial());
  await assert.rejects(runtime.api.window.activeTextEditor.edit(builder => {
    for (let i = 0; i < 4097; i++) builder.insert(new runtime.api.Position(0, 0), 'x');
  }), /edit count limit/);
  assert.equal(requests, 0);
  assert.equal(runtime.api.window.activeTextEditor.document.getText(), 'b\na\n');
});

// Empty selections are commonly deleted before an insertion; they change no text.
test('empty-range deletes do not introduce overlapping insertion edits', async () => {
  let submitted;
  const runtime = createApi(async (method, params) => { submitted = params.edits; return { applied: true }; }, () => {});
  runtime.sync(initial());
  const editor = runtime.api.window.activeTextEditor;
  await editor.edit(builder => {
    builder.delete(new runtime.api.Range(0, 0, 0, 0));
    builder.insert(new runtime.api.Position(0, 0), 'prefix');
  });
  assert.equal(submitted.length, 1);
  assert.equal(submitted[0].newText, 'prefix');
});

test('surface facade forwards the lifecycle owner guard without blocking disposal cleanup', () => {
  let active = true;
  const messages=[];
  const runtime=createApi(async()=>null,(method,params)=>messages.push({method,params}),{
    session:7,assertOwner(owner){assert.equal(owner,'fixture.owner');if(!active)throw new Error('Owner retired');},
  });
  const facade=runtime.forExtension('fixture.owner');
  const output=facade.window.createOutputChannel('native');output.append('retained');
  active=false;
  assert.throws(()=>output.append('late'),/Owner retired/);
  assert.throws(()=>facade.window.createStatusBarItem('late'),/Owner retired/);
  assert.doesNotThrow(()=>runtime.disposeOwner('fixture.owner'));
  assert.equal(messages.at(-1).params.op,'outputDispose');
});

test('surface facade uses live activation ownership and clears failed owner only', () => {
  const active = new Set(['old.owner', 'new.owner']), messages = [];
  const runtime = createApi(async () => ({}), (method, params) => messages.push({ method, params }), { session: 7 });
  runtime.setActivation({ accepts: owner => active.has(owner), facade: () => ({}) });
  const old = runtime.forExtension('old.owner'), fresh = runtime.forExtension('new.owner');
  const retained = old.window.createOutputChannel('Retained');
  const failed = fresh.window.createOutputChannel('Failed');
  const status = fresh.window.createStatusBarItem('Failed'); status.show();
  active.delete('new.owner');
  assert.throws(() => failed.append('late'), /owner is not active/);
  assert.throws(() => status.show(), /owner is not active/);
  assert.throws(() => fresh.window.createOutputChannel('new late'), /owner is not active/);
  runtime.disposeOwner('new.owner');
  retained.append('still active');
  assert.equal(messages.at(-1).params.owner, 'old.owner');
  assert.equal(messages.at(-1).params.text, 'still active');
  assert.equal(messages.filter(item => item.params.op === 'outputDispose').length, 1);
  assert.equal(messages.filter(item => item.params.op === 'statusDispose').length, 1);
});
