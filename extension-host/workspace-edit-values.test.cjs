'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { Uri, Position, Range } = require('./api-types.cjs');
const { WorkspaceEdit, TextEdit, SnippetTextEdit, NotebookEdit, NotebookRange, NotebookCellData, captureWorkspaceEntries } = require('./edit-values.cjs');
const { SnippetString } = require('./snippet-string.cjs');
const { snapshotWorkspaceEdit, toPublicWorkspaceEdit } = require('./workspace-edit-values.cjs');
const uri = Uri.parse('file:///tmp/original.cpp'), other = Uri.parse('file:///tmp/other.cpp');
const span = () => new Range(1, 2, 3, 4);
const internalRange = { startLineNumber: 2, startColumn: 3, endLineNumber: 4, endColumn: 5 };
const wire = value => JSON.parse(JSON.stringify(value));

test('versioned conversion suppresses creates across the whole original edit and stamps notebook rows', () => {
  const changes = new WorkspaceEdit(), calls = [];
  changes.replace(uri, span(), 'before create');changes.createFile(uri);
  changes.set(other, [SnippetTextEdit.replace(span(), new SnippetString('snippet'))]);
  changes.set(other, [NotebookEdit.updateNotebookMetadata({flag:true})]);
  const savedIsUri = Uri.isUri;
  // Public namespace mutation cannot change the converter's trusted predicate.
  Uri.isUri = () => false;
  let result;
  try { result = snapshotWorkspaceEdit(changes,()=>{}, {
    getTextDocumentVersion:resource=>{calls.push(['text',resource.toString()]);return 7;},
    getNotebookDocumentVersion:resource=>{calls.push(['notebook',resource.toString()]);return 12;},
  }); } finally { Uri.isUri = savedIsUri; }
  assert.equal(result.edits[0].versionId,undefined);assert.equal(result.edits[2].versionId,7);assert.equal(result.edits[3].notebookVersionId,12);
  assert.deepEqual(calls,[['text',other.toString()],['notebook',other.toString()]]);
});

test('version authority is checked immediately and cannot mutate edits or manufacture invalid versions', () => {
  const changes = new WorkspaceEdit();changes.replace(uri,span(),'original');
  assert.throws(()=>snapshotWorkspaceEdit(changes,()=>{}, {getTextDocumentVersion:()=>0}),/Invalid.*version/);
  assert.throws(()=>snapshotWorkspaceEdit(changes,()=>{}, {getTextDocumentVersion:()=>{changes.deleteFile(other);return 7;}}),/changed during projection/);
});

test('unknown brands project to the source empty DTO and still check original caller', () => {
  for (const value of [undefined, null, 1, {}, { entries() { throw new Error('must not be called'); } }]) {
    let checked = 0;
    assert.deepEqual(snapshotWorkspaceEdit(value, () => checked++), { edits: [] });
    assert.equal(checked, 2);
  }
  assert.throws(() => snapshotWorkspaceEdit({}, () => { throw new Error('retired'); }), /retired/);
});

test('from preserves ordered full text/snippet/EOL/file annotations in source internal DTO', () => {
  const changes = new WorkspaceEdit(), metadata = { label: 'Rename 猫', needsConfirmation: true, description: 'detail' };
  changes.replace(uri, span(), '猫😀', metadata);
  const snippet = SnippetTextEdit.replace(span(), new SnippetString('${1:name}')); snippet.keepWhitespace = true;
  changes.set(other, [[snippet, metadata], TextEdit.setEndOfLine(2)]);
  changes.renameFile(uri, other, { overwrite: true, ignoreIfExists: false }, metadata);
  assert.deepEqual(wire(snapshotWorkspaceEdit(changes)), { edits: [
    { resource: uri.toString(), textEdit: { text: '猫😀', range: internalRange }, metadata },
    { resource: other.toString(), textEdit: { range: internalRange, text: '${1:name}', insertAsSnippet: true, keepWhitespace: true }, metadata },
    { resource: other.toString(), textEdit: { text: '', eol: 1, range: { startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 1 } } },
    { oldResource: uri.toString(), newResource: other.toString(), options: { overwrite: true, ignoreIfExists: false }, metadata },
  ] });
});

test('from creates/deletes retain complete file options and exact view slice contents', () => {
  const changes = new WorkspaceEdit(), storage = new Uint8Array([1, 2, 3, 4]);
  changes.createFile(uri, { overwrite: true, contents: storage.subarray(1, 3), custom: { reason: 'create' } }, { label: 'Create' });
  changes.deleteFile(other, { recursive: true, ignoreIfNotExists: true });
  const result = wire(snapshotWorkspaceEdit(changes));
  assert.deepEqual(result.edits[0], { newResource: uri.toString(), options: {
    overwrite: true, custom: { reason: 'create' }, contents: { type: 'base64', value: 'AgM=' },
  }, metadata: { label: 'Create' } });
  assert.deepEqual(result.edits[1], { oldResource: other.toString(), options: { recursive: true, ignoreIfNotExists: true } });
  storage[1] = 255;
  assert.equal(result.edits[0].options.contents.value, 'AgM=');
  const file = { _itemId: 'original-transfer' };
  const transfer = new WorkspaceEdit(); transfer.createFile(other, { contents: file });
  assert.deepEqual(wire(snapshotWorkspaceEdit(transfer)).edits[0].options.contents, { type: 'dataTransferItem', id: 'original-transfer' });
});

test('all source EOL conversion cases retain their query DTO while to omits EOL', () => {
  for (const [input, expected] of [[1, 0], [2, 1], [0, 0], [3, undefined], [undefined, undefined]]) {
    const changes = new WorkspaceEdit(), edit = TextEdit.replace(span(), 'x'); edit.newEol = input; changes.set(uri, [edit]);
    const dto = snapshotWorkspaceEdit(changes);
    assert.equal(dto.edits[0].textEdit.eol, expected);
    assert.equal(toPublicWorkspaceEdit(dto).get(uri)[0].newEol, undefined);
  }
});

test('from preserves all notebook edit variants, cell fields, execution summary and output bytes', () => {
  const changes = new WorkspaceEdit(), data = new NotebookCellData(1, 'source 猫', 'markdown');
  data.mime = 'text/markdown'; data.metadata = { custom: true };
  data.executionSummary = { success: true, executionOrder: 3, timing: { startTime: 10, endTime: 20 } };
  data.outputs = [{ id: 'output-one', metadata: { render: 'custom' }, items: [{ mime: 'text/plain', data: new Uint8Array([65, 66]) }] }];
  changes.set(uri, [
    [NotebookEdit.updateCellMetadata(2, { flag: 'cell' }), { label: 'cell meta' }],
    NotebookEdit.updateNotebookMetadata({ flag: 'notebook' }),
    NotebookEdit.replaceCells(new NotebookRange(1, 3), [data]),
  ]);
  const dto = wire(snapshotWorkspaceEdit(changes));
  assert.deepEqual(dto.edits[0], { metadata: { label: 'cell meta' }, resource: uri.toString(), cellEdit: { editType: 3, metadata: { flag: 'cell' }, index: 2 } });
  assert.deepEqual(dto.edits[1], { resource: uri.toString(), cellEdit: { editType: 5, metadata: { flag: 'notebook' } } });
  assert.deepEqual(dto.edits[2], { resource: uri.toString(), cellEdit: { editType: 1, index: 1, count: 2, cells: [{
    cellKind: 1, language: 'markdown', mime: 'text/markdown', source: 'source 猫', metadata: { custom: true },
    internalMetadata: { lastRunSuccess: true, runStartTime: 10, runEndTime: 20, executionOrder: 3 },
    outputs: [{ outputId: 'output-one', metadata: { render: 'custom' }, items: [{ mime: 'text/plain', valueBytes: { buffer: { type: 'Buffer', data: [65, 66] }, byteLength: 2 } }] }],
  }] } });
});

test('to projects files first then resource-grouped text/snippets and drops metadata/version/EOL', () => {
  const dto = { edits: [
    { resource: uri.toString(), versionId: 12, metadata: { label: 'drop' }, textEdit: { text: 'first', eol: 1, range: internalRange } },
    { resource: other.toString(), textEdit: { text: '${1:snippet}', insertAsSnippet: true, keepWhitespace: false, range: internalRange } },
    { oldResource: uri.toString(), newResource: other.toString(), metadata: { label: 'drop' }, options: { overwrite: true } },
    { resource: uri.toString(), textEdit: { text: 'third', range: internalRange } },
  ] };
  const projected = toPublicWorkspaceEdit(dto), records = captureWorkspaceEntries(projected).entries;
  assert(projected instanceof WorkspaceEdit);
  assert.deepEqual(records.map(record => record.type), ['file', 'text', 'text', 'snippet']);
  assert.equal(records[0].from.toString(), uri.toString());
  assert.deepEqual(records[0].options, { overwrite: true });
  assert(records.every(record => record.metadata === undefined));
  assert.equal(records[1].edit.newEol, undefined);
  assert.deepEqual(records[1].edit.range.toJSON(), span().toJSON());
  assert.equal(records[2].edit.newText, 'third');
  assert.equal(records[3].keepWhitespace, false);
  assert.equal(records[3].edit.value, '${1:snippet}');
  assert.notEqual(toPublicWorkspaceEdit(dto), projected);
});

test('to source notebook fallthrough creates file records with absent endpoints', () => {
  const projected = toPublicWorkspaceEdit({ edits: [{ resource: uri.toString(), cellEdit: { editType: 5, metadata: { a: 1 } } }] });
  const records = captureWorkspaceEntries(projected).entries;
  assert.equal(records.length, 1); assert.equal(records[0].type, 'file');
  assert.equal(records[0].from, undefined); assert.equal(records[0].to, undefined);
  assert.deepEqual(projected.entries(), []);
});

test('query snapshots detach nested source metadata, options, ranges, snippets and notebook values', () => {
  const changes = new WorkspaceEdit(), metadata = { label: 'before', nested: { items: ['before'] } }, options = { nested: { a: 'before' } };
  changes.replace(uri, span(), 'before', metadata); changes.createFile(other, options);
  const dto = snapshotWorkspaceEdit(changes);
  metadata.label = 'after'; metadata.nested.items[0] = 'after'; options.nested.a = 'after';
  const records = captureWorkspaceEntries(changes).entries; records[0].edit.newText = 'after';
  assert.equal(dto.edits[0].textEdit.text, 'before'); assert.equal(dto.edits[0].metadata.label, 'before');
  assert.deepEqual(dto.edits[0].metadata.nested.items, ['before']); assert.equal(dto.edits[1].options.nested.a, 'before');
});

test('URI snapshots ignore poisoned formatting and cached strings', () => {
  const poisoned = Uri.parse('file:///tmp/source.cpp');
  poisoned.toString = () => other.toString(); poisoned.toJSON = () => ({ scheme: 'file', path: '/tmp/other.cpp' }); poisoned._formatted = other.toString();
  const changes = new WorkspaceEdit(); changes.insert(poisoned, new Position(0, 0), 'x');
  assert.equal(snapshotWorkspaceEdit(changes).edits[0].resource, 'file:///tmp/source.cpp');
});

test('generation mutations from text/metadata/URI getters fail and retain original caller guard', () => {
  for (const kind of ['text', 'metadata', 'uri', 'guard']) {
    const changes = new WorkspaceEdit(), selected = Uri.parse('file:///tmp/source.cpp');
    const edit = TextEdit.replace(span(), 'x');
    changes.set(selected, [edit]);
    let mutated = false;
    const mutate = () => { if (!mutated) { mutated = true; changes.insert(other, new Position(0, 0), 'reentrant'); } };
    if (kind === 'text') Object.defineProperty(edit, 'newText', { get() { mutate(); return 'x'; } });
    if (kind === 'metadata') {
      const metadata = {}; Object.defineProperty(metadata, 'label', { enumerable: true, get() { mutate(); return 'label'; } });
      changes.set(selected, [[TextEdit.replace(span(), 'y'), metadata]]);
    }
    if (kind === 'uri') Object.defineProperty(selected, 'path', { get() { mutate(); return '/tmp/source.cpp'; } });
    let checks = 0;
    assert.throws(() => snapshotWorkspaceEdit(changes, () => { if (kind === 'guard' && ++checks === 2) mutate(); }), /changed during projection/, kind);
  }
  const changes = new WorkspaceEdit(), edit = TextEdit.replace(span(), 'x'); changes.set(uri, [edit]);
  let retired = false; Object.defineProperty(edit, 'newText', { get() { retired = true; return 'x'; } });
  assert.throws(() => snapshotWorkspaceEdit(changes, () => { if (retired) throw new Error('exact caller retired'); }), /exact caller retired/);
});

test('full query DTO budgets reject raw payloads, oversized metadata and cycles without truncation', () => {
  const large = new WorkspaceEdit(); large.replace(uri, span(), 'x'.repeat(1024 * 1024 + 1));
  assert.throws(() => snapshotWorkspaceEdit(large), /1 MiB/);
  const escaped = new WorkspaceEdit(); escaped.replace(uri, span(), '\0'.repeat(400000));
  assert.throws(() => snapshotWorkspaceEdit(escaped), /2 MiB/);
  const binary = new WorkspaceEdit(); binary.createFile(uri, { contents: new Uint8Array(1024 * 1024 + 1) });
  assert.throws(() => snapshotWorkspaceEdit(binary), /1 MiB/);
  const meta = new WorkspaceEdit(); meta.replace(uri, span(), 'x', { label: 'x'.repeat(2 * 1024 * 1024 + 1) });
  assert.throws(() => snapshotWorkspaceEdit(meta), /2 MiB/);
  const cyclic = {}; cyclic.loop = cyclic;
  const cycles = new WorkspaceEdit(); cycles.replace(uri, span(), 'x', cyclic);
  assert.throws(() => snapshotWorkspaceEdit(cycles), /cyclic/);
});

test('metadata JSON projection preserves own prototype keys and source JSON semantics', () => {
  const metadata = JSON.parse('{"__proto__":{"safe":true},"constructor":"value"}');
  metadata.date = new Date('2026-01-01T00:00:00.000Z'); metadata.values = [undefined, NaN, Infinity];
  const changes = new WorkspaceEdit(); changes.replace(uri, span(), 'x', metadata);
  const dto = snapshotWorkspaceEdit(changes);
  assert.equal(Object.getPrototypeOf(dto.edits[0].metadata), Object.prototype);
  assert.deepEqual(dto.edits[0].metadata.__proto__, { safe: true });
  assert.equal(dto.edits[0].metadata.date, '2026-01-01T00:00:00.000Z');
  assert.deepEqual(dto.edits[0].metadata.values, [null, null, null]);
  assert.equal(dto.edits[0].metadata.constructor, 'value');
});
