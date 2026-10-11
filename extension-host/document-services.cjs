'use strict';
const { Uri, TextDocument, Range } = require('./api-types.cjs');
const coordinates = require('./coordinates.cjs');
const path = require('node:path');
const NATIVE_COMMANDS = Object.freeze(['undo', 'redo', 'editor.action.selectAll', 'cursorLeft', 'cursorRight', 'cursorUp', 'cursorDown', 'cursorHome', 'cursorEnd', 'cursorTop', 'cursorBottom']);
function createDocumentServices(request, options, model) {
  let pending = 0;
  async function call(owner, method, args) {
    if (!owner) throw new Error('Native document services require an extension owner');
    if (pending >= 8) throw new Error('Native document service queue limit reached');
    pending++;
    let released = false;
    const release = () => { if (!released) { released = true; pending--; } };
    const lifecycle = method === 'nativeDocumentSave' ? { retained: false, onTerminal: release } : undefined;
    try { return await request(method, { session: options.session, owner, generation: model.generation(), args }, lifecycle); }
    finally { if (!lifecycle?.retained) release(); }
  }
  function forOwner(owner, editor) {
    async function openTextDocument(input) {
      let filename;
      if (typeof input === 'string') filename = path.resolve(model.root() || '.', input);
      else if (input instanceof Uri && input.scheme === 'file') filename = input.fsPath;
      else throw new Error('VSCLI openTextDocument supports file paths and file URIs; untitled content and other schemes are not implemented');
      if (!filename || Buffer.byteLength(filename) > 16 * 1024) throw new Error('Document path exceeds 16 KiB');
      const result = await call(owner, 'nativeDocumentOpen', { path: filename });
      const document = model.document(result.document);
      if (!document || document.isClosed) throw new Error('Native document mirror was not available before open acknowledgement');
      return document;
    }
    async function showTextDocument(input, options = undefined, preserveFocus = undefined) {
      if (preserveFocus !== undefined) throw new Error('Positional showTextDocument view-column overload is not implemented');
      if (options !== undefined && (options === null || typeof options !== 'object' || Array.isArray(options))) throw new Error('showTextDocument requires supported options');
      options ||= {};
      for (const key of Object.keys(options)) {
        if (!['selection', 'preview', 'preserveFocus', 'viewColumn'].includes(key)) throw new Error(`showTextDocument option is not implemented: ${key}`);
      }
      if (options.preview !== undefined && options.preview !== false) throw new Error('Preview editor tabs are not implemented');
      if (options.preserveFocus !== undefined && options.preserveFocus !== false) throw new Error('showTextDocument preserveFocus is not implemented');
      if (options.viewColumn !== undefined && options.viewColumn !== -1) throw new Error('showTextDocument supports only the active editor group');
      if (options.selection !== undefined && !(options.selection instanceof Range)) throw new Error('showTextDocument selection must be a Range');
      const document = input instanceof TextDocument ? input : await openTextDocument(input);
      if (document.isClosed || model.document(document._snapshot.id) !== document) throw new Error('Document is no longer open');
      const result = await call(owner, 'nativeDocumentShow', { document: document._snapshot.id, version: document.version,
        selection: options.selection ? coordinates.range(options.selection, 'Selection') : null });
      const shown = model.editor(result.document);
      if (!shown || shown.document !== document) throw new Error('Native editor mirror was not available before show acknowledgement');
      return editor(shown);
    }
    return { openTextDocument, showTextDocument };
  }
  async function saveDocument(owner, document) {
    if (!(document instanceof TextDocument)) throw new Error('Document is no longer open');
    // Capture each admission value once. A getter cannot pass identity for A
    // then replace the DTO identity with B. Recheck the original ALS lease
    // after all metadata access, never reacquire via run(owner).
    const snapshot = document._snapshot;
    const id = snapshot?.id, version = snapshot?.version;
    const saved = snapshot?.savedGeneration ?? 0, closed = snapshot?.isClosed;
    const retained = model.document(id);
    if (model.callerOwner() !== owner) throw new Error('Document save caller ownership changed');
    if (closed || retained !== document) throw new Error('Document is no longer open');
    if (!Number.isSafeInteger(id) || id <= 0 || !Number.isSafeInteger(version) || version <= 0 ||
        !Number.isSafeInteger(saved) || saved < 0) {
      throw new Error('Invalid native document save identity');
    }
    const result = await call(owner, 'nativeDocumentSave', { document: id, version });
    const published = document._snapshot;
    const nowSaved = published?.savedGeneration, nowClosed = published?.isClosed;
    const nowRetained = model.document(id);
    // This async continuation still owns the original private lease. A
    // dispose/reacquire ABA must reject even when disk genuinely committed;
    // actual host/service occupancy has already settled independently.
    if (model.callerOwner() !== owner) throw new Error('Document save caller ownership changed');
    if (typeof result !== 'boolean') throw new Error('Invalid native document save acknowledgement');
    if (result && (nowClosed || nowRetained !== document ||
        !Number.isSafeInteger(nowSaved) || nowSaved <= saved)) {
      throw new Error('Native saved document mirror was not published before save acknowledgement');
    }
    return result;
  }
  return { forOwner, saveDocument, execute(owner, id, args) {
    if (!NATIVE_COMMANDS.includes(id)) return Promise.reject(new Error(`Native command delegation is not implemented: ${id}`));
    if (args.length) return Promise.reject(new Error(`Native command arguments are not implemented: ${id}`));
    return call(owner, 'nativeCommand', { id });
  } };
}
module.exports = { createDocumentServices, NATIVE_COMMANDS };
