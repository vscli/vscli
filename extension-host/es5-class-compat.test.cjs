'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { es5ClassCompat } = require('./es5-class-compat.cjs');
const types = require('./api-types.cjs');
const providers = require('./provider-types.cjs');
const diagnostics = require('./diagnostic-types.cjs');
const edits = require('./edit-values.cjs');
const { SnippetString } = require('./snippet-string.cjs');
const { CancellationTokenSource } = require('./cancellation.cjs');
const { FileSystemError } = require('./filesystem.cjs');
const { ThemeColor, TreeItem } = require('./surfaces.cjs');
const { Position, Range, Selection, Disposable, EventEmitter, Uri } = types;
function subclass(Parent, viaApply = false) {
  function Legacy(...args) { return viaApply ? Parent.apply(this, args) : Parent.call(this, ...args); }
  Legacy.prototype = Object.create(Parent.prototype); Legacy.prototype.constructor = Legacy;
  return Legacy;
}

test('exact helper preserves class identity, property flags, zero-argument call/apply branches', () => {
  class Value { constructor(value) { this.value = value; } }
  const reference = Value;
  assert.equal(es5ClassCompat(Value), reference);
  for (const method of ['call', 'apply']) assert.deepEqual(Object.getOwnPropertyDescriptor(Value, method), {
    value: Value[method], enumerable: true, writable: true, configurable: true,
  });
  assert.equal(Value.call().value, undefined); assert.equal(Value.apply().value, undefined);
  const Legacy = subclass(Value, true); const value = new Legacy('kept');
  assert(value instanceof Legacy); assert(value instanceof Value); assert.equal(value.value, 'kept');
  assert.equal(Value.apply({ constructor: Legacy }).value, undefined);
  assert.throws(() => Value.call(null), TypeError); assert.throws(() => Value.apply(null, []), TypeError);
});

test('legacy Position/Range/Selection subclassing preserves direction, factories and document type identity', () => {
  const OldPosition = subclass(Position), OldRange = subclass(Range, true), OldSelection = subclass(Selection, true);
  const start = new OldPosition(0, 1), end = new OldPosition(0, 4);
  assert(start instanceof OldPosition); assert.equal(Position.of(start), start);
  const span = new OldRange(start, end); assert(span instanceof OldRange); assert.equal(Range.of(span), span);
  const selection = new OldSelection(end, start);
  assert(selection instanceof OldSelection); assert(selection instanceof Selection); assert(selection instanceof Range);
  assert.equal(selection.anchor, end); assert.equal(selection.active, start); assert.equal(selection.isReversed, true);
  assert.equal(selection.start, start); assert.equal(selection.end, end);
  const document = new types.TextDocument({ id: 1, uri: 'untitled:test', version: 1, languageId: 'cpp', isDirty: true, text: 'a🙂猫\r\nb' });
  assert.equal(document.validatePosition(start), start); assert.equal(document.validateRange(span), span);
  assert.equal(document.getText(span), '🙂猫');
});

test('legacy Disposable initializes private brand and retains original successful cleanup identity', () => {
  const OldDisposable = subclass(Disposable, true); let calls = 0;
  const disposable = new OldDisposable(() => { calls++; });
  assert(disposable instanceof OldDisposable); assert(disposable instanceof Disposable);
  disposable.dispose(); disposable.dispose(); assert.equal(calls, 1);
  let attempts = 0;
  const retry = new OldDisposable(() => { if (++attempts === 1) throw new Error('retry'); });
  assert.throws(() => retry.dispose(), /retry/); retry.dispose(); retry.dispose(); assert.equal(attempts, 2);
});

test('all source-decorated existing values own compat call/apply while preserving construction and instanceof', () => {
  const range = new Range(0, 0, 0, 1), uri = Uri.parse('file:///tmp/a'), location = new providers.Location(uri, range);
  const rows = [
    [providers.MarkdownString, ['text']], [SnippetString, ['${1:猫}']],
    [providers.CompletionItem, ['text', 0]], [providers.CompletionList, [[], false]],
    [providers.Hover, [['text'], range]], [providers.Location, [uri, range]],
    [providers.DocumentSymbol, ['x', '', 11, range, range]],
    [providers.SymbolInformation, ['x', 11, '', location]],
    [providers.ParameterInformation, ['x']], [providers.SignatureInformation, ['f(x)']],
    [providers.SignatureHelp, []], [diagnostics.Diagnostic, [range, 'message', 0]],
    [diagnostics.DiagnosticRelatedInformation, [location, 'message']],
    [providers.CodeActionKind, ['quickfix']], [providers.CodeAction, ['Fix']],
    [ThemeColor, ['editor.foreground']], [TreeItem, ['item', 0]],
    [edits.TextEdit, [range, 'text']], [edits.WorkspaceEdit, []],
    [edits.NotebookEdit, [new edits.NotebookRange(0, 1), []]],
  ];
  for (const [Parent, args] of rows) {
    assert(Object.hasOwn(Parent, 'call'), Parent.name); assert(Object.hasOwn(Parent, 'apply'), Parent.name);
    const Legacy = subclass(Parent, true), value = new Legacy(...args);
    assert(value instanceof Legacy, Parent.name); assert(value instanceof Parent, Parent.name);
  }
});

test('legacy workspace/snippet edits retain shared classes through existing bounded snapshot adapter', () => {
  const OldWorkspace = subclass(edits.WorkspaceEdit), OldEdit = subclass(edits.TextEdit, true), OldSnippet = subclass(SnippetString);
  const uri = Uri.parse('untitled:legacy'), range = new Range(0, 0, 0, 0), edit = new OldEdit(range, '猫🙂\r\n');
  const workspace = new OldWorkspace(); workspace.set(uri, [edit]);
  assert.equal(workspace.get(uri)[0], edit);
  const snapshot = edits.snapshotWorkspaceEdit(workspace);
  assert.equal(snapshot.resources[0].values[0].range, range);
  assert.equal(snapshot.resources[0].values[0].newText, '猫🙂\r\n');
  const snippet = new OldSnippet('x'); assert(SnippetString.isSnippetString(snippet));
  snippet.appendPlaceholder('cat'); assert.equal(snippet.value, 'x${1:cat}');
});

test('FileSystemError retains pinned explicit prototype reset instead of inventing subclass identity', () => {
  const Legacy = subclass(FileSystemError, true), error = new Legacy('failed');
  assert(error instanceof FileSystemError); assert(error instanceof Error);
  assert.equal(error.message, 'failed'); assert.equal(error instanceof Legacy, false);
  assert(Object.hasOwn(FileSystemError, 'call')); assert(Object.hasOwn(FileSystemError, 'apply'));
  assert.equal(FileSystemError.FileNotFound('missing').name, 'EntryNotFound (FileSystemError)');
});

test('upstream undecorated classes are not made ES5-callable by shared integration', () => {
  for (const Parent of [CancellationTokenSource, EventEmitter, Uri, edits.SnippetTextEdit, edits.NotebookRange, edits.NotebookCellData, edits.NotebookData]) {
    assert.equal(Object.hasOwn(Parent, 'call'), false, Parent.name);
    assert.equal(Object.hasOwn(Parent, 'apply'), false, Parent.name);
    assert.throws(() => Parent.call({ constructor: Parent }), TypeError);
  }
});
