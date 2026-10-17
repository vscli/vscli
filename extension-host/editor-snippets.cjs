/*---------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. Source-guided overload/option normalization:
 * https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts#L468-L496
 * VSCLI adaptations: private native proofs, bounded DTOs and exact receipts.
 *--------------------------------------------------------------------------------------------*/
'use strict';
const {Position, Range} = require('./api-types.cjs');
const coordinates = require('./coordinates.cjs');
const MAX_RANGES = 4096, MAX_TEMPLATE = 64 * 1024, MAX_WIRE = 1024 * 1024;

function location(value, arrayEntry = false) {
  if (value instanceof Range) return coordinates.range(value, 'Snippet');
  // Upstream array entries use Position.from for each non-Range value, which
  // also accepts a position-like entry. The single-value overload is branded.
  if (value instanceof Position || arrayEntry) {
    const point = coordinates.position(value, 'Snippet');
    return {start: point, end: {...point}};
  }
  throw new TypeError('Snippet location must be a Position, Range or Selection');
}

function rangesFor(where, state) {
  let values;
  if (!where) values = state.overlay?.values || state.actual;
  else if (Array.isArray(where)) {
    const length = where.length;
    if (!Number.isInteger(length) || length < 0 || length > MAX_RANGES) throw new RangeError('Snippet range count exceeds 4096');
    if (length === 0) values = state.overlay?.values || state.actual;
    else {
      const result = [];
      for (let index = 0; index < length; index++) result.push(location(where[index], true));
      return result;
    }
  } else return [location(where)];
  // Snapshot the private current cohort before evaluating snippet/options getters.
  // Never read editor.selections or any public document metadata for authority.
  if (!Array.isArray(values)) throw new Error('Native editor selections are not available');
  const length = values.length;
  if (!Number.isInteger(length) || length < 1 || length > MAX_RANGES) throw new RangeError('Snippet range count exceeds 4096');
  const result = [];
  for (let index = 0; index < length; index++) result.push(coordinates.range(values[index], 'Snippet'));
  return result;
}

function createEditorSnippets({enabled = false, captureEditor, call}) {
  async function insertSnippet(owner, document, base, snippet, where, options) {
    if (!enabled) throw new Error('Native editor snippets are not implemented by this host');
    const captured = captureEditor(owner, document, base);
    const ranges = rangesFor(where, captured.state);
    const template = snippet?.value;
    if (typeof template !== 'string') throw new TypeError('Snippet value must be a string');
    if (template.length > MAX_TEMPLATE || Buffer.byteLength(template) > MAX_TEMPLATE) throw new RangeError('Snippet template exceeds 64 KiB');
    let undoStopBefore = true, undoStopAfter = true, keepWhitespace = false;
    if (options !== undefined) {
      if (!options || (typeof options !== 'object' && typeof options !== 'function')) throw new TypeError('Snippet options must be an object');
      // Each user property is evaluated exactly once; do not mutate supplied options.
      undoStopBefore = Boolean(options.undoStopBefore);
      undoStopAfter = Boolean(options.undoStopAfter);
      keepWhitespace = Boolean(options.keepWhitespace);
    }
    const args = {document: captured.source.document, version: captured.source.version,
      editor: captured.state.nativeId, snippet: template, ranges,
      undoStopBefore, undoStopAfter, keepWhitespace};
    // All values are captured primitives. Range count/template caps bound this
    // escaped allocation before checking the exact outgoing UTF-8 byte count.
    const serialized = JSON.stringify(args);
    if (serialized.length > MAX_WIRE || Buffer.byteLength(serialized) > MAX_WIRE) throw new RangeError('Snippet request exceeds 1 MiB');
    captured.recheck();
    const result = await call(owner, 'nativeEditorInsertSnippet', args);
    if (!result || typeof result !== 'object' || Array.isArray(result)
        || Object.keys(result).length !== 3
        || !Object.hasOwn(result, 'document') || !Object.hasOwn(result, 'editor') || !Object.hasOwn(result, 'applied')
        || result.document !== args.document || result.editor !== args.editor || typeof result.applied !== 'boolean') {
      throw new Error('Invalid native snippet acknowledgement');
    }
    captured.accept(result);
    return result.applied;
  }
  return {insertSnippet};
}

module.exports = {createEditorSnippets};
