'use strict';
const { es5ClassCompat } = require('./es5-class-compat.cjs');
// Derived from Microsoft VS Code 1.141.0 commit
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c, extHostTypes/{textEdit,
// snippetTextEdit,workspaceEdit,notebooks}.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
/*
MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
// Value construction does not authorize native edits or filesystem changes.
function createEditTypes({ Position, Range, Uri, SnippetString }) {
  class TextEdit {
    static isTextEdit(value) {
      if (value instanceof TextEdit) return true;
      return !!value && typeof value === 'object' && Range.isRange(value) && typeof value.newText === 'string';
    }
    static replace(range, text) { return new TextEdit(range, text); }
    static insert(position, text) { return TextEdit.replace(new Range(position, position), text); }
    static delete(range) { return TextEdit.replace(range, ''); }
    static setEndOfLine(eol) {
      const edit = new TextEdit(new Range(new Position(0, 0), new Position(0, 0)), '');
      edit.newEol = eol; return edit;
    }
    constructor(range, text) { this._range = range; this._newText = text; }
    get range() { return this._range; }
    set range(value) { if (value && !Range.isRange(value)) throw new Error('Illegal argument: range'); this._range = value; }
    get newText() { return this._newText || ''; }
    set newText(value) { if (value && typeof value !== 'string') throw new Error('Illegal argument: newText'); this._newText = value; }
    get newEol() { return this._newEol; }
    set newEol(value) { if (value && typeof value !== 'number') throw new Error('Illegal argument: newEol'); this._newEol = value; }
    toJSON() { return { range: this.range, newText: this.newText, newEol: this._newEol }; }
  }
  class SnippetTextEdit {
    static isSnippetTextEdit(value) {
      return value instanceof SnippetTextEdit || !!value && Range.isRange(value.range) && SnippetString.isSnippetString(value.snippet);
    }
    static replace(range, snippet) { return new SnippetTextEdit(range, snippet); }
    static insert(position, snippet) { return SnippetTextEdit.replace(new Range(position, position), snippet); }
    constructor(range, snippet) { this.range = range; this.snippet = snippet; }
  }
  class NotebookRange {
    static isNotebookRange(value) { return value instanceof NotebookRange || !!value && typeof value.start === 'number' && typeof value.end === 'number'; }
    constructor(start, end) {
      if (start < 0) throw new Error('Illegal argument: start must be positive');
      if (end < 0) throw new Error('Illegal argument: end must be positive');
      if (start <= end) { this._start = start; this._end = end; }
      else { this._start = end; this._end = start; }
    }
    get start() { return this._start; }
    get end() { return this._end; }
    get isEmpty() { return this._start === this._end; }
    with(change) {
      const start = change.start !== undefined ? change.start : this._start;
      const end = change.end !== undefined ? change.end : this._end;
      return start === this._start && end === this._end ? this : new NotebookRange(start, end);
    }
  }
  const NotebookCellKind = Object.freeze({ Markup: 1, Code: 2, 1: 'Markup', 2: 'Code' });
  class NotebookCellData {
    static validate(data) {
      for (const [key, type] of [['kind', 'number'], ['value', 'string'], ['languageId', 'string']]) {
        if (typeof data[key] !== type) throw new Error(`NotebookCellData MUST have '${key}' property`);
      }
    }
    static isNotebookCellDataArray(value) { return Array.isArray(value) && value.every(item => NotebookCellData.isNotebookCellData(item)); }
    static isNotebookCellData() { return true; } // Exact upstream helper, not backend validation.
    constructor(kind, value, languageId, mime, outputs, metadata, executionSummary) {
      Object.assign(this, { kind, value, languageId, mime, outputs: outputs ?? [], metadata, executionSummary });
      NotebookCellData.validate(this);
    }
  }
  class NotebookData { constructor(cells) { this.cells = cells; } }
  class NotebookEdit {
    static isNotebookCellEdit(value) {
      return value instanceof NotebookEdit || !!value && NotebookRange.isNotebookRange(value) && Array.isArray(value.newCells);
    }
    static replaceCells(range, cells) { return new NotebookEdit(range, cells); }
    static insertCells(index, cells) { return new NotebookEdit(new NotebookRange(index, index), cells); }
    static deleteCells(range) { return new NotebookEdit(range, []); }
    static updateCellMetadata(index, metadata) {
      const edit = new NotebookEdit(new NotebookRange(index, index), []); edit.newCellMetadata = metadata; return edit;
    }
    static updateNotebookMetadata(metadata) {
      const edit = new NotebookEdit(new NotebookRange(0, 0), []); edit.newNotebookMetadata = metadata; return edit;
    }
    constructor(range, cells) { this.range = range; this.newCells = cells; }
  }

  // Bounded native adaptation: ordered entries (not grouped copies) retain
  // upstream append/reference semantics; staged commits prevent partial changes
  // on getter reentrancy, limit failures and exhausted generation counters.
  const states = new WeakMap(), MAX_ENTRIES = 4096, MAX_RESOURCES = 128;
  function state(value) { const found = states.get(value); if (!found) throw new TypeError('WorkspaceEdit receiver is invalid'); return found; }
  function key(uri) {
    if (!(uri instanceof Uri)) throw new TypeError('WorkspaceEdit requires URI');
    const result = uri.toString();
    if (typeof result !== 'string' || Buffer.byteLength(result) > 4096) throw new Error('Workspace edit URI exceeds budget');
    return result;
  }
  function commit(value, expected, entries) {
    if (entries.length > MAX_ENTRIES) throw new Error('Workspace edit count exceeds 4096');
    const resources = new Set();
    for (const entry of entries) {
      if (entry.uri !== undefined) resources.add(key(entry.uri));
      if (entry.from !== undefined) resources.add(key(entry.from));
      if (entry.to !== undefined) resources.add(key(entry.to));
      if (resources.size > MAX_RESOURCES) throw new Error('Workspace edit resources exceed 128');
    }
    const current = state(value);
    if (current.generation !== expected) throw new Error('Workspace edit changed during capture');
    if (current.generation >= Number.MAX_SAFE_INTEGER) throw new Error('Workspace edit generation exhausted');
    current.entries = entries; current.generation++;
  }
  function append(value, entries, expected) {
    const current = state(value);
    if (entries.length) commit(value, expected ?? current.generation, [...current.entries, ...entries]);
    else if (expected !== undefined && current.generation !== expected) throw new Error('Workspace edit changed during capture');
  }
  function textEntries(value) { return state(value).entries.filter(entry => entry.type === 'text'); }
  class WorkspaceEdit {
    constructor() { states.set(this, { entries: [], generation: 0 }); }
    renameFile(from, to, options, metadata) { append(this, [{ type: 'file', from, to, options, metadata }]); }
    createFile(uri, options, metadata) { append(this, [{ type: 'file', from: undefined, to: uri, options, metadata }]); }
    deleteFile(uri, options, metadata) { append(this, [{ type: 'file', from: uri, to: undefined, options, metadata }]); }
    replace(uri, range, text, metadata) { append(this, [{ type: 'text', uri, edit: new TextEdit(range, text), metadata }]); }
    insert(uri, position, text, metadata) { this.replace(uri, new Range(position, position), text, metadata); }
    delete(uri, range, metadata) { this.replace(uri, range, '', metadata); }
    set(uri, edits) {
      const current = state(this), expected = current.generation;
      if (!edits) {
        const target = key(uri);
        const retained = current.entries.filter(entry => entry.type === 'file' || key(entry.uri) !== target);
        if (retained.length !== current.entries.length) commit(this, expected, retained);
        else if (state(this).generation !== expected) throw new Error('Workspace edit changed during capture');
        return;
      }
      if (!Array.isArray(edits)) throw new Error('Workspace edits require an array of at most 4096 entries');
      const length = edits.length, captured = [];
      if (!Number.isSafeInteger(length) || length < 0 || length > MAX_ENTRIES) throw new Error('Workspace edits require an array of at most 4096 entries');
      for (let index = 0; index < length; index++) {
        const item = edits[index]; if (!item) continue;
        const edit = Array.isArray(item) ? item[0] : item, metadata = Array.isArray(item) ? item[1] : undefined;
        if (NotebookEdit.isNotebookCellEdit(edit)) {
          const cellMetadata = edit.newCellMetadata;
          if (cellMetadata) captured.push({ type: 'cellMetadata', uri, index: edit.range.start, value: cellMetadata, metadata });
          else {
            const documentMetadata = edit.newNotebookMetadata;
            if (documentMetadata) captured.push({ type: 'notebookMetadata', uri, value: documentMetadata, metadata });
            else {
              const range = edit.range, cells = edit.newCells;
              if (!Array.isArray(cells)) throw new Error('Notebook cells exceed 4096');
              const length = cells.length;
              if (!Number.isSafeInteger(length) || length < 0 || length > MAX_ENTRIES) throw new Error('Notebook cells exceed 4096');
              // Pinned replaceNotebookCells captures each endpoint once.
              const start = range.start, end = range.end;
              if (start !== end || length) captured.push({ type: 'cells', uri, index: start, count: end - start, cells, metadata });
            }
          }
        } else if (SnippetTextEdit.isSnippetTextEdit(edit)) {
          captured.push({ type: 'snippet', uri, range: edit.range, edit: edit.snippet, metadata, keepWhitespace: edit.keepWhitespace });
        } else captured.push({ type: 'text', uri, edit, metadata });
      }
      if (edits.length !== length) throw new Error('Workspace edit array changed during capture');
      append(this, captured, expected);
    }
    has(uri) { const target = key(uri); return textEntries(this).some(entry => key(entry.uri) === target); }
    get(uri) { const target = key(uri); return textEntries(this).filter(entry => key(entry.uri) === target).map(entry => entry.edit); }
    entries() {
      const resources = new Map();
      for (const entry of textEntries(this)) {
        const target = key(entry.uri); let result = resources.get(target);
        if (!result) { result = [entry.uri, []]; resources.set(target, result); }
        result[1].push(entry.edit);
      }
      return [...resources.values()];
    }
    get size() { return this.entries().length; }
    toJSON() { return this.entries(); }
  }
  function captureWorkspaceEntries(value) {
    const current = state(value), generation = current.generation;
    return { generation, entries: current.entries.map(entry => ({ ...entry })), current: () => state(value).generation };
  }
  function captureTextEdit(edit) {
    if (!edit || typeof edit !== 'object') throw new Error('Unsupported workspace text edit');
    const allowed = edit instanceof TextEdit ? ['_range', '_newText', '_newEol'] : ['range', 'newText', 'newEol'];
    if (Object.keys(edit).some(name => !allowed.includes(name))) throw new Error('Unsupported workspace text edit property');
    return { range: edit.range, newText: edit.newText, eol: edit.newEol };
  }
  function snapshotTextEdit(edit) {
    const { range, newText, eol } = captureTextEdit(edit);
    if (eol !== undefined) throw new Error('Text end-of-line changes are unsupported by the native edit backend');
    return { range, newText };
  }
  function snapshotWorkspaceEdit(value) {
    const captured = captureWorkspaceEntries(value), resources = new Map(), unsupported = [];
    for (const entry of captured.entries) {
      if (entry.type !== 'text') {
        unsupported.push(entry.type === 'file' ? entry.from === undefined ? 'file creation' : entry.to === undefined ? 'file deletion' : 'file rename' : entry.type === 'snippet' ? 'snippet edit' : 'notebook edit');
        continue;
      }
      const target = key(entry.uri), edit = entry.edit;
      const { range, newText, eol } = captureTextEdit(edit);
      if (eol !== undefined) unsupported.push('text end-of-line change');
      let resource = resources.get(target);
      if (!resource) { resource = { uri: entry.uri, key: target, values: [] }; resources.set(target, resource); }
      const plain = { range, newText };
      resource.values.push(entry.metadata === undefined ? plain : [plain, entry.metadata]);
    }
    if (captured.current() !== captured.generation) throw new Error('Workspace edit changed during normalization');
    return { generation: captured.generation, resources: [...resources.values()], unsupported, current: captured.current };
  }
  for (const target of [TextEdit, WorkspaceEdit, NotebookEdit]) es5ClassCompat(target);
  const EndOfLine = Object.freeze({ LF: 1, CRLF: 2, 1: 'LF', 2: 'CRLF' });
  return { TextEdit, SnippetTextEdit, NotebookEdit, NotebookRange, NotebookCellData, NotebookData, NotebookCellKind, EndOfLine, WorkspaceEdit, snapshotWorkspaceEdit, snapshotTextEdit, captureWorkspaceEntries };
}
module.exports = { createEditTypes };
