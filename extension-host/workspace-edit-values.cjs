'use strict';
// Source-guided from Microsoft VS Code 1.141.0, immutable commit
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c:
// extHostTypeConverters.ts WorkspaceEdit/TextEdit/Range/NotebookCellData,
// extHostTypes/workspaceEdit.ts and notebookCommon.ts CellEditType.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT license reproduced in edit-types.cjs.
// Native adaptations: detached bounded DTOs, trusted URI formatting, and an
// original caller/generation guard around every extension-owned getter.
const { Uri, Range } = require('./api-types.cjs');
const { SnippetString } = require('./snippet-string.cjs');
const { WorkspaceEdit, TextEdit, SnippetTextEdit, captureWorkspaceEntries } = require('./edit-values.cjs');
const { uriSnapshot } = require('./language-selector.cjs');
const parseUri = Uri.parse.bind(Uri);
const MAX_ENTRIES = 4096, MAX_RESOURCES = 128, MAX_RAW = 1024 * 1024, MAX_WIRE = 2 * MAX_RAW;

function snapshotWorkspaceEdit(value, check = () => {}) {
  check();
  let captured;
  try { captured = captureWorkspaceEntries(value); }
  catch (error) {
    // Pinned WorkspaceEdit.from accepts any unknown brand as an empty DTO.
    if (error instanceof TypeError && error.message === 'WorkspaceEdit receiver is invalid') {
      check(); return { edits: [] };
    }
    throw error;
  }
  const generation = captured.generation;
  function current() {
    check();
    if (captured.current() !== generation) throw new Error('Workspace edit changed during projection');
  }
  function read(object, key) { const result = object?.[key]; current(); return result; }
  let raw = 0, payload = 0, nodes = 0;
  const resources = new Set(), ancestors = new WeakSet();
  function string(value, isRaw = false) {
    if (typeof value !== 'string') throw new TypeError('Workspace edit text requires a string');
    if (value.length > MAX_WIRE) throw new RangeError('Workspace edit JSON exceeds 2 MiB');
    if (isRaw) {
      if (value.length > MAX_RAW - raw) throw new RangeError('Workspace edit raw payload exceeds 1 MiB');
      raw += Buffer.byteLength(value);
      if (raw > MAX_RAW) throw new RangeError('Workspace edit raw payload exceeds 1 MiB');
    }
    payload += Buffer.byteLength(JSON.stringify(value));
    if (payload > MAX_WIRE) throw new RangeError('Workspace edit JSON exceeds 2 MiB');
    return value;
  }
  // Follow JSON object/array conversion semantics while preventing unbounded
  // traversal or retained provider references. No extension toJSON survives.
  function json(value, key = '', depth = 0, converted = false) {
    if (++nodes > 32768 || depth > 64) throw new RangeError('Workspace edit metadata exceeds traversal bounds');
    if (value && typeof value === 'object' && !converted) {
      const toJSON = read(value, 'toJSON');
      if (typeof toJSON === 'function') {
        const replacement = Reflect.apply(toJSON, value, [key]); current();
        return json(replacement, key, depth, true);
      }
    }
    if (value === null) return null;
    switch (typeof value) {
      case 'undefined': case 'function': case 'symbol': return undefined;
      case 'string': return string(value);
      case 'boolean': return value;
      case 'number': return Number.isFinite(value) ? value : null;
      case 'bigint': throw new TypeError('Workspace edit metadata cannot serialize BigInt');
      case 'object': break;
      default: throw new TypeError('Invalid workspace edit metadata');
    }
    if (ancestors.has(value)) throw new TypeError('Workspace edit metadata is cyclic');
    ancestors.add(value);
    let result;
    if (Array.isArray(value)) {
      const count = read(value, 'length');
      if (!Number.isSafeInteger(count) || count < 0 || count > MAX_ENTRIES) throw new RangeError('Workspace edit array exceeds 4096 entries');
      result = [];
      for (let index = 0; index < count; index++) result.push(json(read(value, index), String(index), depth + 1) ?? null);
      if (read(value, 'length') !== count) throw new Error('Workspace edit array changed during projection');
    } else {
      const keys = Object.keys(value); current();
      if (keys.length > MAX_ENTRIES) throw new RangeError('Workspace edit object exceeds 4096 fields');
      result = {};
      for (const name of keys) {
        string(name);
        const field = json(read(value, name), name, depth + 1);
        if (field !== undefined) Object.defineProperty(result, name, { value: field, writable: true, enumerable: true, configurable: true });
      }
    }
    ancestors.delete(value);
    return result;
  }
  function uri(value) {
    if (value === undefined || value === null) return value;
    // Capture public components once; caches and overrides cannot change keys.
    const components = {};
    for (const key of ['scheme', 'authority', 'path', 'query', 'fragment']) components[key] = read(value, key);
    const result = uriSnapshot(components); current();
    resources.add(result);
    if (resources.size > MAX_RESOURCES) throw new RangeError('Workspace edit resources exceed 128');
    return result;
  }
  function range(value) {
    if (!value) return undefined; // Pinned Range.from falsy handling.
    const start = read(value, 'start'), end = read(value, 'end');
    const result = {
      startLineNumber: read(start, 'line') + 1, startColumn: read(start, 'character') + 1,
      endLineNumber: read(end, 'line') + 1, endColumn: read(end, 'character') + 1,
    };
    // Query conversion follows public constructor coordinates. Native editing
    // separately requires finite integers and validates document UTF-16 bounds.
    for (const key of Object.keys(result)) if (typeof result[key] !== 'number') throw new TypeError('Invalid workspace edit range coordinate');
    return result;
  }
  function bytes(view) {
    if (!ArrayBuffer.isView(view)) throw new TypeError('Workspace edit contents require an ArrayBuffer view');
    // Use intrinsic accessors, never extension overrides of byteLength/buffer.
    const typed = Object.getPrototypeOf(Uint8Array.prototype);
    const proto = view instanceof DataView ? DataView.prototype : typed;
    const length = Reflect.apply(Object.getOwnPropertyDescriptor(proto, 'byteLength').get, view, []);
    const offset = Reflect.apply(Object.getOwnPropertyDescriptor(proto, 'byteOffset').get, view, []);
    const buffer = Reflect.apply(Object.getOwnPropertyDescriptor(proto, 'buffer').get, view, []);
    raw += length;
    if (raw > MAX_RAW) throw new RangeError('Workspace edit raw payload exceeds 1 MiB');
    const result = Buffer.from(new Uint8Array(buffer, offset, length)); current(); return result;
  }
  function array(value) {
    if (!Array.isArray(value)) throw new TypeError('Workspace notebook edits require an array');
    const length = read(value, 'length');
    if (!Number.isSafeInteger(length) || length < 0 || length > MAX_ENTRIES) throw new RangeError('Workspace notebook edits exceed 4096 entries');
    const copy = [];
    for (let index = 0; index < length; index++) copy.push(read(value, index));
    if (read(value, 'length') !== length) throw new Error('Workspace notebook array changed during projection');
    return copy;
  }
  function cell(data) {
    const kind = read(data, 'kind'), language = read(data, 'languageId'), mime = read(data, 'mime'),
      source = string(read(data, 'value'), true), metadata = json(read(data, 'metadata'), 'metadata'),
      summary = read(data, 'executionSummary') ?? {}, outputs = read(data, 'outputs');
    const timing = read(summary, 'timing');
    return {
      cellKind: kind === 1 ? 1 : 2, language: json(language, 'language'), mime: json(mime, 'mime'), source, metadata,
      internalMetadata: {
        lastRunSuccess: json(read(summary, 'success'), 'lastRunSuccess'),
        runStartTime: json(read(timing, 'startTime'), 'runStartTime'),
        runEndTime: json(read(timing, 'endTime'), 'runEndTime'),
        executionOrder: json(read(summary, 'executionOrder'), 'executionOrder'),
      },
      outputs: outputs ? array(outputs).map(output => ({
        outputId: json(read(output, 'id'), 'outputId'),
        items: array(read(output, 'items')).map(item => {
          const mime = json(read(item, 'mime'), 'mime'), buffer = bytes(read(item, 'data'));
          // JSON-safe representation of the source VSBuffer's own fields in
          // this Node transport (VSBuffer.wrap uses a native Buffer backing).
          return { mime, valueBytes: { buffer: buffer.toJSON(), byteLength: buffer.byteLength } };
        }),
        metadata: json(read(output, 'metadata'), 'metadata'),
      })) : [],
    };
  }
  if (captured.entries.length > MAX_ENTRIES) throw new RangeError('Workspace edit count exceeds 4096');
  const result = { edits: [] };
  for (const entry of captured.entries) {
    current();
    const metadata = json(entry.metadata, 'metadata');
    if (entry.type === 'file') {
      const oldResource = uri(entry.from), newResource = uri(entry.to), options = entry.options;
      const contents = read(options, 'contents');
      let convertedContents;
      if (contents) {
        convertedContents = ArrayBuffer.isView(contents)
          ? { type: 'base64', value: bytes(contents).toString('base64') }
          : { type: 'dataTransferItem', id: json(read(contents, '_itemId'), 'id') };
      }
      const projectedOptions = {};
      if (options != null) {
        const keys = Object.keys(Object(options)); current();
        if (keys.length > MAX_ENTRIES) throw new RangeError('Workspace file options exceed 4096 fields');
        for (const key of keys) if (key !== 'contents') {
          string(key);
          const value = json(read(options, key), key);
          if (value !== undefined) Object.defineProperty(projectedOptions, key, { value, enumerable: true, configurable: true, writable: true });
        }
      }
      projectedOptions.contents = convertedContents;
      result.edits.push({ oldResource, newResource, options: projectedOptions, metadata });
    } else if (entry.type === 'text') {
      const resource = uri(entry.uri), text = string(read(entry.edit, 'newText'), true), eol = read(entry.edit, 'newEol');
      result.edits.push({ resource, textEdit: {
        text, eol: eol && (eol === 1 ? 0 : eol === 2 ? 1 : undefined), range: range(read(entry.edit, 'range')),
      }, versionId: undefined, metadata });
    } else if (entry.type === 'snippet') {
      result.edits.push({ resource: uri(entry.uri), textEdit: {
        range: range(entry.range), text: string(read(entry.edit, 'value'), true),
        insertAsSnippet: true, keepWhitespace: json(entry.keepWhitespace, 'keepWhitespace'),
      }, versionId: undefined, metadata });
    } else if (entry.type === 'cellMetadata' || entry.type === 'notebookMetadata') {
      const cellEdit = { editType: entry.type === 'cellMetadata' ? 3 : 5, metadata: json(entry.value, 'metadata') };
      if (entry.type === 'cellMetadata') cellEdit.index = json(entry.index, 'index');
      result.edits.push({ metadata, resource: uri(entry.uri), cellEdit, notebookVersionId: undefined });
    } else if (entry.type === 'cells') {
      result.edits.push({ metadata, resource: uri(entry.uri), notebookVersionId: undefined, cellEdit: {
        editType: 1, index: json(entry.index, 'index'), count: json(entry.count, 'count'), cells: array(entry.cells).map(cell),
      } });
    } else throw new TypeError('Unknown private workspace edit entry');
  }
  current();
  const serialized = JSON.stringify(result);
  if (serialized.length > MAX_WIRE || Buffer.byteLength(serialized) > MAX_WIRE) throw new RangeError('Workspace edit JSON exceeds 2 MiB');
  current();
  return result;
}

function toPublicWorkspaceEdit(dto) {
  const result = new WorkspaceEdit(), grouped = new Map();
  const revive = value => value == null ? value : typeof value === 'string' ? parseUri(value, true) : Uri.revive(value);
  for (const edit of dto.edits) {
    if (edit.textEdit) {
      const item = edit.textEdit, uri = revive(edit.resource), span = item.range;
      const range = span ? new Range(span.startLineNumber - 1, span.startColumn - 1, span.endLineNumber - 1, span.endColumn - 1) : undefined;
      const value = item.insertAsSnippet
        ? SnippetTextEdit.replace(range, new SnippetString(item.text)) : TextEdit.replace(range, item.text);
      if (item.insertAsSnippet) value.keepWhitespace = item.keepWhitespace;
      const key = Reflect.apply(Uri.prototype.toString, uri, []);
      let group = grouped.get(key);
      if (!group) { group = { uri, edits: [] }; grouped.set(key, group); }
      group.edits.push(value);
    } else {
      // Source intentionally has no notebook arm: all non-text DTO rows pass
      // through renameFile, including undefined endpoints. Preserve that quirk.
      result.renameFile(revive(edit.oldResource), revive(edit.newResource), edit.options);
    }
  }
  for (const group of grouped.values()) result.set(group.uri, group.edits);
  return result;
}
module.exports = { snapshotWorkspaceEdit, toPublicWorkspaceEdit };
