'use strict';
const { Uri, TextDocument, Range } = require('./api-types.cjs');
const coordinates = require('./coordinates.cjs');
const path = require('node:path');
const uriFrom = Uri.from, uriFormat = Uri.prototype.toString, uriParse = Uri.parse;
const uriFilePath = Object.getOwnPropertyDescriptor(Uri.prototype, 'fsPath').get;
const NATIVE_COMMANDS = Object.freeze(['undo', 'redo', 'editor.action.selectAll', 'cursorLeft', 'cursorRight', 'cursorUp', 'cursorDown', 'cursorHome', 'cursorEnd', 'cursorTop', 'cursorBottom']);
function createDocumentServices(request, options, model) {
  let pending = 0, queuedBytes = 0;
  async function call(owner, method, args, controls) {
    if (!owner) throw new Error('Native document services require an extension owner');
    if (pending >= 8) throw new Error('Native document service queue limit reached');
    const bytes = Buffer.byteLength(JSON.stringify(args));
    if (bytes > 2 * 1024 * 1024 || bytes > 8 * 1024 * 1024 - queuedBytes) throw new Error('Native document service payload limit reached');
    pending++; queuedBytes += bytes;
    let released = false;
    const release = () => { if (!released) { released = true; pending--; queuedBytes -= bytes; } };
    const lifecycle = ['nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeUntitledDocumentOpen'].includes(method) ? { retained: false, onTerminal: release, onRequest: controls?.onRequest } : undefined;
    try { return await request(method, { session: options.session, owner, generation: model.generation(), args }, lifecycle); }
    finally { if (!lifecycle?.retained) release(); }
  }
  function forOwner(owner, editor) {
    // Stable extHost.api.impl.ts openTextDocument overloads, with the native
    // UTF-8 encoding boundary and runtime-private acknowledgement authority.
    async function openTextDocument(input, openOptions) {
      const guard = model.captureCaller(owner);
      const prior = new Map();
      for (const snapshot of model.snapshots()) {
        if (prior.size >= 128) throw new Error('Document mirror inventory limit reached');
        prior.set(snapshot.document, snapshot);
      }
      guard();
      let uri, filename, content = '', language = null;
      const optionsInput = openOptions ?? input;
      let scheme;
      if (input && typeof input === 'object') scheme = input.scheme;
      if (typeof input === 'string') {
        if (!input || input.includes('\0') || input.length > 16 * 1024 || Buffer.byteLength(input) > 16 * 1024) throw new Error('Document path exceeds 16 KiB');
        filename = path.resolve(model.root() || '.', input);
      } else if (typeof scheme === 'string') {
        // Capture geometry fields once and format a trusted URI; an overridden
        // toString/fsPath or a stateful getter cannot retarget the native DTO.
        const authority = input.authority, uriPath = input.path, query = input.query, fragment = input.fragment;
        if (![scheme, authority, uriPath, query, fragment].every(value => typeof value === 'string' && value.length <= 4096 && !value.includes('\0'))) throw new TypeError('Invalid document URI');
        uri = Reflect.apply(uriFrom, Uri, [{ scheme, authority, path: uriPath, query, fragment }, true]);
        const serialized = Reflect.apply(uriFormat, uri, []);
        if (Buffer.byteLength(serialized) > 4096) throw new Error('Document URI exceeds 4096 bytes');
        if (scheme === 'file') filename = Reflect.apply(uriFilePath, uri, []);
      } else {
        if (!options.untitledDocuments) throw new Error('Native untitled document options are not implemented by this native host');
        if (optionsInput !== undefined && optionsInput !== null && (typeof optionsInput !== 'object' || Array.isArray(optionsInput))) throw new TypeError('Invalid openTextDocument options');
        const rawContent = optionsInput?.content, rawLanguage = optionsInput?.language;
        if (rawContent !== undefined && typeof rawContent !== 'string') throw new TypeError('Untitled content must be a string');
        if (rawLanguage !== undefined && typeof rawLanguage !== 'string') throw new TypeError('Untitled language must be a string');
        content = rawContent ?? '';
        language = rawLanguage || null;
        if (content.length > 1024 * 1024 || Buffer.byteLength(content) > 1024 * 1024) throw new Error('Untitled content exceeds 1 MiB');
        if (rawLanguage !== undefined && (rawLanguage.length > 128 || Buffer.byteLength(rawLanguage) > 128)) throw new Error('Untitled language exceeds 128 bytes');
      }
      // The second options argument wins just as in the upstream overload.
      // Only a known UTF-8 decoder is currently backed by the native service.
      const encoding = optionsInput?.encoding;
      if (encoding !== undefined && encoding !== 'utf8') throw new Error('openTextDocument encoding is not implemented: only utf8 is supported');
      guard();
      if (uri && uri.scheme !== 'file' && uri.scheme !== 'untitled') {
        if (options.virtualDocuments && model.openVirtual) {
          const document = await model.openVirtual(owner, uri);
          guard();
          return document;
        }
        throw new Error('Document URI requires a registered virtual content provider');
      }
      const untitled = !filename;
      if (untitled && !options.untitledDocuments) throw new Error('Native untitled documents are not implemented by this native host');
      const resource = uri && Reflect.apply(uriFormat, uri, []);
      if (untitled && resource) {
        const cached = [...prior.values()].find(snapshot => snapshot.uri === resource);
        if (cached) {
          guard();
          const current = model.snapshot(cached.document);
          if (!current || current.model !== cached.model || current.uri !== resource || model.document(cached.document) !== cached.model) throw new Error('Cached native document mirror was retired');
          return cached.model;
        }
      }
      if (filename && Buffer.byteLength(filename) > 16 * 1024) throw new Error('Document path exceeds 16 KiB');
      const result = await call(owner, untitled ? 'nativeUntitledDocumentOpen' : 'nativeDocumentOpen', untitled
        ? { uri: resource || null, content, language } : { path: filename });
      guard();
      const id = result?.document;
      if (!Number.isSafeInteger(id) || id <= 0) throw new Error('Invalid native document open acknowledgement');
      const snapshot = model.snapshot(id), document = model.document(id), previous = prior.get(id);
      if (!snapshot || snapshot.model !== document || !document || !Number.isSafeInteger(snapshot.version) || snapshot.version <= 0 ||
          (previous && (previous.model !== document || previous.uri !== snapshot.uri))) throw new Error('Native document mirror was not available before open acknowledgement');
      if (untitled && (Reflect.apply(uriParse, Uri, [snapshot.uri]).scheme !== 'untitled' || snapshot.isReadonly ||
          (resource ? snapshot.uri !== resource : !!previous))) throw new Error('Native untitled document mirror did not match open acknowledgement');
      // didOpen handlers may legitimately edit before the reply. Require the
      // fresh initial state only when that mirror is still at version one.
      if (untitled && !previous && snapshot.version === 1 && (snapshot.isDirty !== true || snapshot.text !== content.replace(/\r\n|\r|\n/g, snapshot.eol === 2 ? '\r\n' : '\n') ||
          (language !== null && snapshot.languageId !== language))) throw new Error('Native untitled content was not published before open acknowledgement');
      guard();
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
    const guard = model.captureCaller(owner);
    const source = model.snapshotFor(document);
    guard();
    if (!source || source.isClosed || model.snapshot(source.document)?.model !== document) throw new Error('Document is no longer open');
    const id = source.document, version = source.version, saved = source.savedGeneration;
    if (!Number.isSafeInteger(id) || id <= 0 || !Number.isSafeInteger(version) || version <= 0 ||
        !Number.isSafeInteger(saved) || saved < 0) throw new Error('Invalid native document save identity');
    const result = await call(owner, 'nativeDocumentSave', { document: id, version });
    guard();
    if (typeof result !== 'boolean') throw new Error('Invalid native document save acknowledgement');
    const published = model.snapshot(id);
    if (result) {
      const sameModel = published?.model === document && published.uri === source.uri;
      // An authorized untitled Save As closes the old public identity and opens
      // a file identity for the same native document. Only actual publication
      // of the captured save permits that replacement; an arbitrary reopen or
      // public metadata mutation cannot manufacture a successful receipt.
      const savedAs = published && model.snapshotFor(document)?.isClosed === true &&
        Reflect.apply(uriParse, Uri, [source.uri]).scheme === 'untitled' &&
        Reflect.apply(uriParse, Uri, [published.uri]).scheme === 'file';
      if (!published || published.isClosed || model.document(id) !== published.model ||
          !Number.isSafeInteger(published.savedGeneration) || published.savedGeneration <= saved ||
          (!sameModel && !savedAs)) throw new Error('Native saved document mirror was not published before save acknowledgement');
    }
    guard();
    return result;
  }
  return { forOwner, saveDocument, call, execute(owner, id, args) {
    if (!NATIVE_COMMANDS.includes(id)) return Promise.reject(new Error(`Native command delegation is not implemented: ${id}`));
    if (args.length) return Promise.reject(new Error(`Native command arguments are not implemented: ${id}`));
    return call(owner, 'nativeCommand', { id });
  } };
}
module.exports = { createDocumentServices, NATIVE_COMMANDS };
