'use strict';
const { Uri, TextDocument, Range, Selection } = require('./api-types.cjs');
const coordinates = require('./coordinates.cjs');
const {normalizeOptions}=require('./editor-presentation.cjs');
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
    const lifecycle = ['nativeDocumentSave', 'nativeWorkspaceApplyEdit', 'nativeFindFiles', 'nativeVirtualDocumentOpen', 'nativeVirtualDocumentChange', 'nativeUntitledDocumentOpen', 'nativeDocumentLanguage', 'nativeEditorSelections', 'nativeDocumentShow', 'nativeEditorOptions', 'nativeEditorReveal', 'nativeEditorDecorations', 'nativeEditorInsertSnippet', 'nativeTabsClose'].includes(method) ? { retained: false, onTerminal: release, onRequest: controls?.onRequest } : undefined;
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
    async function showTextDocument(input, columnOrOptions = undefined, preserveFocus = undefined) {
      const guard = model.captureCaller(owner), originalSource = input instanceof TextDocument ? currentSource(input) : undefined;
      let viewColumn = -1, preview, selection;
      if (typeof columnOrOptions === 'number') {
        viewColumn = columnOrOptions;
      } else if (columnOrOptions !== undefined) {
        if (columnOrOptions === null || typeof columnOrOptions !== 'object' || Array.isArray(columnOrOptions)) throw new TypeError('showTextDocument requires supported options');
        const rawColumn = columnOrOptions.viewColumn, rawPreserve = columnOrOptions.preserveFocus;
        preview = columnOrOptions.preview; selection = columnOrOptions.selection;
        viewColumn = rawColumn ?? -1; preserveFocus = rawPreserve;
      }
      if (!Number.isInteger(viewColumn) || ![-2,-1,1,2,3,4].includes(viewColumn)) throw new Error('showTextDocument viewColumn supports Active, Beside, or native groups 1..4');
      if (preserveFocus !== undefined && typeof preserveFocus !== 'boolean') throw new TypeError('showTextDocument preserveFocus must be a boolean');
      if (preview !== undefined && preview !== false) throw new Error('Preview editor tabs are not implemented');
      if (selection !== undefined && !(selection instanceof Range)) throw new TypeError('showTextDocument selection must be a Range');
      const range = selection === undefined ? null : coordinates.range(selection,'Selection');
      guard(); if (originalSource) recheckSource(originalSource);
      const document = input instanceof TextDocument ? input : await openTextDocument(input);
      guard();
      const source = currentSource(document);
      const result = await call(owner,'nativeDocumentShow',{document:source.document,version:source.version,selection:range,viewColumn,preserveFocus:preserveFocus ?? false});
      guard();
      const current = currentSource(document);
      if (result?.document !== source.document || current.uri !== source.uri) throw new Error('Native editor mirror was not available before show acknowledgement');
      const shown = model.editor(source.document,result.editor), state = model.editorSnapshot(shown);
      if (!shown || state?.document !== document || !model.editorCurrent(shown)) throw new Error('Native editor mirror was not available before show acknowledgement');
      guard();
      return editor(shown);
    }
    return { openTextDocument, showTextDocument };
  }
  function currentSource(document) {
    const source = model.snapshotFor(document);
    if (!source || source.isClosed || model.snapshot(source.document)?.model !== document) throw new Error('Document is no longer open');
    return source;
  }
  function recheckSource(source) {
    const current = currentSource(source.model);
    if (current.document !== source.document || current.version !== source.version || current.uri !== source.uri || current.languageEpoch !== source.languageEpoch)
      throw new Error('Native editor source changed during argument capture');
  }
  async function setTextDocumentLanguage(owner, document, language) {
    const guard = model.captureCaller(owner), source = currentSource(document);
    if (!options.documentLanguage) throw new Error('Native document language changes are not implemented by this host');
    if (typeof language !== 'string' || !language || language.length > 128 || Buffer.byteLength(language) > 128 || language.includes('\0')) throw new TypeError('Document language must be a nonempty string of at most 128 bytes');
    guard(); recheckSource(source);
    const result = await call(owner, 'nativeDocumentLanguage', { document: source.document, version: source.version, language });
    guard();
    const current = currentSource(document);
    if (result?.document !== source.document || current.uri !== source.uri || current.languageId !== language) throw new Error('Native document language mirror was not published before acknowledgement');
    return document;
  }
  function setEditorSelections(owner, document, input, single, base) {
    const guard = model.captureCaller(owner), source = currentSource(document), editorState = model.editorSnapshot(base);
    if (!options.editorSelections) throw new Error('Native editor selection changes are not implemented by this host');
    if (editorState?.document !== document || !model.editorCurrent(base)) throw new Error('Text editor is no longer available');
    let values;
    if (single) {
      if (!(input instanceof Selection)) throw new TypeError('selection must be a Selection');
      values = [input];
    } else {
      if (!Array.isArray(input)) throw new TypeError('selections must be an array of Selection values');
      const length = input.length;
      if (!Number.isInteger(length) || length < 0 || length > 10000) throw new RangeError('Editor selection count exceeds 10000');
      values = [];
      for (let i = 0; i < length; i++) {
        const value = input[i];
        if (!(value instanceof Selection)) throw new TypeError('selections must contain only Selection values');
        values.push(value);
      }
      if (!values.length) values.push(new Selection(0,0,0,0));
    }
    const selections = [];
    for (const value of values) {
      const anchor = value.anchor, active = value.active;
      selections.push({ anchor: coordinates.position(anchor, 'Selection'), active: coordinates.position(active, 'Selection') });
    }
    guard(); recheckSource(source);
    if (editorState?.document !== document || !model.editorCurrent(base)) throw new Error('Text editor is no longer available');
    // Presentation only. Native source/selection authority remains the captured
    // primitive DTO and actual mirrored receipt, never the assigned objects.
    const rollback = model.stageSelections(owner, base, values);
    call(owner, 'nativeEditorSelections', { document: source.document, version: source.version, selections,
      ...(editorState.nativeId === undefined ? {} : {editor:editorState.nativeId}) }).then(result => {
      guard();
      const current = currentSource(document);
      if (result?.document !== source.document || current.uri !== source.uri || !model.editorCurrent(base) || (editorState.nativeId !== undefined && result.editor !== editorState.nativeId)) throw new Error('Native editor selection mirror was not published before acknowledgement');
    }).catch(error => {
      rollback();
      try { model.selectionFailure(owner, error); } catch { /* Reporting cannot produce an unhandled rejection. */ }
    });
  }
  function editorServiceSource(owner,document,base,capability) {
    const guard=model.captureCaller(owner),source=currentSource(document),state=model.editorSnapshot(base);
    if(!options[capability])throw new Error('Native editor presentation service is not implemented by this host');
    if(state?.document!==document||state.nativeId===undefined||!model.editorCurrent(base))throw new Error('Text editor is no longer available');
    const recheck=()=>{guard();recheckSource(source);if(!model.editorCurrent(base))throw new Error('Text editor is no longer available');};
    const accept=result=>{
      guard();const current=currentSource(document);
      if(result?.document!==source.document||result.editor!==state.nativeId||current.uri!==source.uri||!model.editorCurrent(base))throw new Error('Native editor presentation mirror was not published before acknowledgement');
    };
    return {source,state,recheck,accept};
  }
  function setEditorOptions(owner,document,input,field,base) {
    const captured=editorServiceSource(owner,document,base,'editorOptions');
    const normalized=normalizeOptions(input,model.editorOptions(base),field);
    captured.recheck();
    if(!Object.keys(normalized.update).length)return;
    const rollback=model.stageOptions(owner,base,normalized.optimistic);
    call(owner,'nativeEditorOptions',{document:captured.source.document,version:captured.source.version,editor:captured.state.nativeId,options:normalized.update})
      .then(captured.accept).catch(error=>{rollback();try{model.presentationFailure(owner,error);}catch{/* No unhandled error from reporting. */}});
  }
  function revealEditorRange(owner,document,range,revealType,base) {
    const captured=editorServiceSource(owner,document,base,'editorReveal');
    if(!(range instanceof Range))throw new TypeError('revealRange requires a Range');
    const bounds=coordinates.range(range,'Reveal range'),kind=revealType||0;
    if(!Number.isInteger(kind)||kind<0||kind>3)throw new TypeError('Invalid TextEditorRevealType');
    captured.recheck();
    call(owner,'nativeEditorReveal',{document:captured.source.document,version:captured.source.version,editor:captured.state.nativeId,range:bounds,revealType:kind})
      .then(captured.accept).catch(error=>{try{model.presentationFailure(owner,error);}catch{/* No unhandled error from reporting. */}});
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
  return { editorServiceSource, forOwner, saveDocument, setTextDocumentLanguage, setEditorSelections, setEditorOptions, revealEditorRange, call, execute(owner, id, args) {
    if (!NATIVE_COMMANDS.includes(id)) return Promise.reject(new Error(`Native command delegation is not implemented: ${id}`));
    if (args.length) return Promise.reject(new Error(`Native command arguments are not implemented: ${id}`));
    return call(owner, 'nativeCommand', { id });
  } };
}
module.exports = { createDocumentServices, NATIVE_COMMANDS };
