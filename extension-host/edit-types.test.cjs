'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { Position, Range, Uri } = require('./api-types.cjs');
const { SnippetString } = require('./snippet-string.cjs');
const { TextEdit, SnippetTextEdit, NotebookEdit, NotebookRange, NotebookCellData, NotebookData, WorkspaceEdit, snapshotWorkspaceEdit, snapshotTextEdit, captureWorkspaceEntries } = require('./edit-values.cjs');
const uri = Uri.parse('file:///tmp/source.cpp'), other = Uri.parse('file:///tmp/other.cpp');
const range = () => new Range(0, 1, 0, 3);

test('TextEdit retains constructor values; setters validate only truthy values and JSON contains EOL', () => {
  const selected = range(), edit = new TextEdit(selected, null);
  assert.equal(edit.range, selected); assert.equal(edit.newText, '');
  edit.newText = '猫🙂\r\n'; assert.equal(edit.newText, '猫🙂\r\n');
  assert.throws(() => { edit.range = {}; }, /Illegal argument: range/);
  assert.throws(() => { edit.newText = 1; }, /Illegal argument: newText/);
  assert.throws(() => { edit.newEol = 'LF'; }, /Illegal argument: newEol/);
  edit.newText = 0; assert.equal(edit.newText, '');
  edit.range = null; assert.equal(edit.range, null);
  const unchecked = new TextEdit('unchecked', 1); assert.equal(unchecked.range, 'unchecked');
  const eol = TextEdit.setEndOfLine(2);
  assert.equal(eol.newEol, 2); assert(eol.range.isEmpty);
  assert.deepEqual(eol.toJSON(), { range: eol.range, newText: '', newEol: 2 });
});

test('TextEdit utility methods and pinned internal structural helper behavior', () => {
  const selected = range(); assert.equal(TextEdit.replace(selected, 'x').range, selected);
  assert.equal(TextEdit.delete(selected).newText, ''); assert(TextEdit.insert(new Position(0, 2), 'a').range.isEmpty);
  assert.equal(TextEdit.isTextEdit(new TextEdit(selected, 'a')), true);
  // Source checks Range.isRange(thing), not thing.range. Keep the actual quirk.
  assert.equal(TextEdit.isTextEdit({ range: selected, newText: 'a' }), false);
  assert.equal(TextEdit.isTextEdit({ start: selected.start, end: selected.end, newText: 'a' }), true);
});

test('SnippetTextEdit references remain mutable and structural detection matches pinned types', () => {
  const selected = range(), snippet = new SnippetString('${1:猫}');
  const edit = SnippetTextEdit.replace(selected, snippet); edit.keepWhitespace = true;
  assert.equal(edit.range, selected); assert.equal(edit.snippet, snippet);
  assert.equal(SnippetTextEdit.isSnippetTextEdit({ range: selected, snippet }), true);
  assert(SnippetTextEdit.insert(new Position(0, 2), snippet).range.isEmpty);
  snippet.appendText('!'); assert.equal(edit.snippet.value, '${1:猫}!');
});

test('NotebookRange sorts endpoints, preserves no-op identity and rejects negative values', () => {
  const cells = new NotebookRange(5, 2); assert.equal(cells.start, 2); assert.equal(cells.end, 5);
  assert.equal(cells.with({}), cells); assert.equal(cells.with({ start: 2 }), cells);
  assert.equal(cells.with({ end: 1 }).start, 1);
  assert.throws(() => new NotebookRange(-1, 0), /start must be positive/);
  assert.equal(NotebookRange.isNotebookRange({ start: 0, end: 1 }), true);
});

test('NotebookEdit all constructors/statics retain arrays and metadata identities', () => {
  const cells = [new NotebookCellData(2, '猫', 'cpp')], span = new NotebookRange(1, 3), metadata = { nested: { value: 1 } };
  const edit = NotebookEdit.replaceCells(span, cells);
  assert.equal(edit.range, span); assert.equal(edit.newCells, cells);
  assert(NotebookEdit.insertCells(2, cells).range.isEmpty);
  assert.deepEqual(NotebookEdit.deleteCells(span).newCells, []);
  assert.equal(NotebookEdit.updateCellMetadata(2, metadata).newCellMetadata, metadata);
  assert.equal(NotebookEdit.updateNotebookMetadata(metadata).newNotebookMetadata, metadata);
  assert.equal(new NotebookData(cells).cells, cells);
  assert.throws(() => new NotebookCellData(2, null, 'cpp'), /value/);
  assert.equal(NotebookEdit.isNotebookCellEdit({ range: span, newCells: cells }), false);
});

test('WorkspaceEdit set appends; empty set retains entries and null/undefined remove resource edits', () => {
  const changes = new WorkspaceEdit(), first = TextEdit.replace(range(), 'first'), second = TextEdit.replace(range(), 'second');
  changes.set(uri, [first]); changes.set(uri, [second]); changes.set(uri, []);
  assert.deepEqual(changes.get(uri), [first, second]); assert.equal(changes.size, 1);
  changes.set(other, [first]); changes.set(uri, null);
  assert.equal(changes.has(uri), false); assert.equal(changes.has(other), true);
  changes.set(other, undefined); assert.equal(changes.size, 0);
});

test('WorkspaceEdit metadata tuples retain references but get/entries/toJSON expose text edits only', () => {
  const changes = new WorkspaceEdit(), edit = TextEdit.replace(range(), 'a'), metadata = { needsConfirmation: true, label: 'Change' };
  changes.set(uri, [[edit, metadata]]); changes.insert(other, new Position(0, 0), 'b');
  assert.equal(changes.get(uri)[0], edit);
  const entries = changes.entries(); assert.equal(entries[0][0], uri); assert.equal(entries[0][1][0], edit);
  entries[0][1].length = 0; changes.get(uri).length = 0;
  assert.equal(changes.get(uri).length, 1); assert.deepEqual(changes.toJSON(), changes.entries());
  metadata.label = 'Updated'; edit.newText = 'updated';
  const snapshot = snapshotWorkspaceEdit(changes);
  assert.equal(snapshot.resources[0].values[0][1], metadata);
  assert.equal(snapshot.resources[0].values[0][0].newText, 'updated');
});

test('ordered file/snippet/notebook operations retain options but text map stays empty', () => {
  const changes = new WorkspaceEdit(), options = { overwrite: true, contents: new Uint8Array([1, 2]) }, metadata = { label: 'file', needsConfirmation: false };
  changes.createFile(uri, options, metadata); changes.renameFile(uri, other, { ignoreIfExists: true }); changes.deleteFile(other, { recursive: true });
  const snippet = SnippetTextEdit.replace(range(), new SnippetString('$1'));
  changes.set(uri, [snippet, NotebookEdit.insertCells(0, [new NotebookCellData(2, 'x', 'cpp')]), NotebookEdit.updateCellMetadata(0, {}), NotebookEdit.updateNotebookMetadata({})]);
  assert.equal(changes.size, 0); assert.equal(changes.has(uri), false); assert.deepEqual(changes.entries(), []);
  const snapshot = captureWorkspaceEntries(changes);
  assert.deepEqual(snapshot.entries.map(entry => entry.type), ['file', 'file', 'file', 'snippet', 'cells', 'cellMetadata', 'notebookMetadata']);
  assert.equal(snapshot.entries[0].options, options); assert.equal(snapshot.entries[0].metadata, metadata);
  snapshot.entries.length = 0; assert.equal(captureWorkspaceEntries(changes).entries.length, 7);
  assert.deepEqual(snapshotWorkspaceEdit(changes).unsupported, ['file creation', 'file rename', 'file deletion', 'snippet edit', 'notebook edit', 'notebook edit', 'notebook edit']);
});

test('resource null removal retains file operations, empty notebook replace does not create entry', () => {
  const changes = new WorkspaceEdit(); changes.createFile(uri); changes.insert(uri, new Position(0, 0), 'x');
  changes.set(uri, [SnippetTextEdit.insert(new Position(0, 0), new SnippetString('x')), NotebookEdit.updateNotebookMetadata({})]);
  changes.set(other, [NotebookEdit.insertCells(0, [])]);
  assert.equal(captureWorkspaceEntries(changes).entries.length, 4);
  changes.set(uri, null); assert.equal(captureWorkspaceEntries(changes).entries.length, 1);
  assert.deepEqual(snapshotWorkspaceEdit(changes).unsupported, ['file creation']);
});

test('workspace primitive snapshot is detached from later edit fields and generation tracks collection mutation', () => {
  const changes = new WorkspaceEdit(), edit = TextEdit.replace(range(), 'before'); changes.set(uri, [edit]);
  const captured = snapshotWorkspaceEdit(changes); edit.newText = 'after';
  assert.equal(captured.resources[0].values[0].newText, 'before');
  assert.equal(captured.current(), captured.generation);
  changes.delete(other, range()); assert.notEqual(captured.current(), captured.generation);
});

test('EOL and annotated edits survive value model but remain unsupported at strict serialization boundary', () => {
  const changes = new WorkspaceEdit(); changes.set(uri, [TextEdit.setEndOfLine(2)]);
  assert.deepEqual(snapshotWorkspaceEdit(changes).unsupported, ['text end-of-line change']);
  assert.throws(() => snapshotTextEdit(TextEdit.setEndOfLine(1)), /unsupported by the native edit backend/);
  const ordinary = TextEdit.replace(range(), 'a'); assert.deepEqual(snapshotTextEdit(ordinary), { range: ordinary.range, newText: 'a' });
  ordinary.annotationId = 'a'; assert.throws(() => snapshotTextEdit(ordinary), /Unsupported workspace text edit property/);
});

test('batch array getter failure is atomic and reentrant capture rejects stale outer publication', () => {
  const changes = new WorkspaceEdit(), retained = TextEdit.replace(range(), 'safe'); changes.set(uri, [retained]);
  const hostile = [retained, retained]; Object.defineProperty(hostile, 1, { get() { throw new Error('getter'); } });
  assert.throws(() => changes.set(other, hostile), /getter/); assert.equal(changes.has(other), false); assert.equal(changes.get(uri)[0], retained);
  const reentrant = [retained]; Object.defineProperty(reentrant, 0, { get() { changes.insert(uri, new Position(0, 0), 'inner'); return retained; } });
  assert.throws(() => changes.set(other, reentrant), /changed during capture/);
  assert.equal(changes.has(other), false); assert.equal(changes.get(uri).length, 2);
});

test('snapshot getter reentrancy rejects generation change rather than serializing torn workspace', () => {
  const changes = new WorkspaceEdit();
  const edit = { range: range(), get newText() { changes.insert(other, new Position(0, 0), 'inner'); return 'outer'; } };
  changes.set(uri, [edit]); assert.throws(() => snapshotWorkspaceEdit(changes), /changed during normalization/);
  assert.equal(changes.has(other), true);
});

test('full batch resource/count budgets reject before publishing any prefix', () => {
  const changes = new WorkspaceEdit(), edit = TextEdit.replace(range(), 'a');
  changes.set(uri, Array(4096).fill(edit)); const original = captureWorkspaceEntries(changes);
  assert.throws(() => changes.insert(other, new Position(0, 0), 'lost'), /count exceeds 4096/);
  assert.equal(captureWorkspaceEntries(changes).generation, original.generation); assert.equal(changes.has(other), false);
  assert.throws(() => changes.set(uri, Array(4097).fill(edit)), /at most 4096/);
  changes.set(uri, null);
  for (let index = 0; index < 128; index++) changes.insert(Uri.parse(`file:///tmp/${index}`), new Position(0, 0), 'a');
  const before = captureWorkspaceEntries(changes).generation;
  assert.throws(() => changes.insert(Uri.parse('file:///tmp/extra'), new Position(0, 0), 'lost'), /resources exceed 128/);
  assert.equal(changes.size, 128); assert.equal(captureWorkspaceEntries(changes).generation, before);
});

test('proxy array length is admitted once before traversal and checked again after capture', () => {
  const changes = new WorkspaceEdit(), edit = TextEdit.replace(range(), 'a'); let reads = 0;
  const values = new Proxy([edit], { get(target, name) { if (name === 'length') return ++reads === 1 ? 1 : 4097; return Reflect.get(target, name); } });
  assert.throws(() => changes.set(uri, values), /array changed during capture/); assert.equal(changes.size, 0); assert.equal(reads, 2);
});


test('upstream decorated classes preserve ES5 function subclass call/apply and receiver prototype', () => {
  function OldEdit(selected, text) { return TextEdit.call(this, selected, text); }
  OldEdit.prototype = Object.create(TextEdit.prototype); OldEdit.prototype.constructor = OldEdit;
  const selected = range(), edit = new OldEdit(selected, 'legacy');
  assert(edit instanceof OldEdit); assert(edit instanceof TextEdit); assert.equal(edit.range, selected);
  function OldWorkspace() { return WorkspaceEdit.apply(this, []); }
  OldWorkspace.prototype = Object.create(WorkspaceEdit.prototype); OldWorkspace.prototype.constructor = OldWorkspace;
  const changes = new OldWorkspace(); changes.set(uri, [edit]); assert.equal(changes.get(uri)[0], edit);
  function OldNotebook(cells) { return NotebookEdit.apply(this, [new NotebookRange(0, 0), cells]); }
  OldNotebook.prototype = Object.create(NotebookEdit.prototype); OldNotebook.prototype.constructor = OldNotebook;
  const cells = [], notebook = new OldNotebook(cells); assert(notebook instanceof NotebookEdit); assert.equal(notebook.newCells, cells);
});


test('empty and all-falsy set batches avoid URI access like pinned source', () => {
  const changes = new WorkspaceEdit(), resource = Uri.parse('file:///tmp/noop'), before = captureWorkspaceEntries(changes);
  Object.defineProperty(resource, 'toString', { value() { throw new Error('must not capture noop URI'); } });
  changes.set(resource, []); changes.set(resource, [null, false, undefined, 0]);
  assert.equal(captureWorkspaceEntries(changes).generation, before.generation);
  assert.equal(changes.size, 0);
});

test('notebook replace captures source endpoints once even for accessor-backed valid values', () => {
  let starts = 0, ends = 0;
  const span = { get start() { return 2 + starts++; }, get end() { ends++; return 8; } };
  const changes = new WorkspaceEdit(), cells = [new NotebookCellData(2, 'cat', 'cpp')];
  changes.set(uri, [new NotebookEdit(span, cells)]);
  const [entry] = captureWorkspaceEntries(changes).entries;
  assert.equal(starts, 1); assert.equal(ends, 1);
  assert.equal(entry.index, 2); assert.equal(entry.count, 6); assert.equal(entry.cells, cells);
});

test('notebook cells proxy length is admitted once and invalid lengths reject without mutation', () => {
  const changes = new WorkspaceEdit(), before = captureWorkspaceEntries(changes); let reads = 0;
  const cells = new Proxy([], { get(target, name) { if (name === 'length') return ++reads === 1 ? 0 : 4097; return Reflect.get(target, name); } });
  changes.set(uri, [NotebookEdit.insertCells(0, cells)]);
  assert.equal(reads, 1); assert.equal(captureWorkspaceEntries(changes).generation, before.generation);
  for (const length of [4097, -1, NaN, Infinity, 0.5]) {
    const invalid = new Proxy([], { get(target, name) { return name === 'length' ? length : Reflect.get(target, name); } });
    assert.throws(() => changes.set(uri, [NotebookEdit.insertCells(0, invalid)]), /Notebook cells exceed 4096/);
    assert.equal(captureWorkspaceEntries(changes).generation, before.generation);
  }
});

test('later invalid notebook range getter cannot publish the earlier valid edit prefix', () => {
  const changes = new WorkspaceEdit(), retained = TextEdit.replace(range(), 'safe'); changes.set(uri, [retained]);
  const before = captureWorkspaceEntries(changes), invalidRange = { get start() { throw new Error('invalid cell start'); }, end: 1 };
  assert.throws(() => changes.set(other, [TextEdit.replace(range(), 'lost'), new NotebookEdit(invalidRange, [])]), /invalid cell start/);
  assert.equal(changes.has(other), false); assert.deepEqual(changes.get(uri), [retained]);
  assert.equal(captureWorkspaceEntries(changes).generation, before.generation);
});

test('reentrant notebook endpoint capture retains inner mutation and rejects outer batch exactly once', () => {
  const changes = new WorkspaceEdit(); let calls = 0;
  const span = { get start() { calls++; changes.insert(uri, new Position(0, 0), 'inner'); return 0; }, end: 1 };
  assert.throws(() => changes.set(other, [new NotebookEdit(span, [])]), /changed during capture/);
  assert.equal(calls, 1); assert.equal(changes.get(uri).length, 1); assert.equal(changes.has(other), false);
});
