'use strict';
const types = require('./api-types.cjs');
const { createCallerContext } = require('./caller-context.cjs');
const coordinates = require('./coordinates.cjs');
const diagnosticTypes = require('./diagnostic-types.cjs');
const { createDiagnostics } = require('./diagnostics.cjs');
const providerTypes = require('./provider-types.cjs');
const { createProviders } = require('./providers.cjs');
const { createPrompts } = require('./prompts.cjs');
const { createSurfaces } = require('./surfaces.cjs');
const { createConfiguration } = require('./configuration.cjs');
const { createWorkspaceEdits } = require('./workspace-edits.cjs');
const { RelativePattern, createFileSearch } = require('./workspace-files.cjs');
const { createFileWatchers } = require('./file-watchers.cjs');
const { createTextContentProviders } = require('./text-content-providers.cjs');
const { createDocumentServices, NATIVE_COMMANDS } = require('./document-services.cjs');
const { createMementos } = require('./memento.cjs');
const filesystem = require('./filesystem.cjs');
const { createWorkspaceScope } = require('./workspace-scope.cjs');
const { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument } = types;
const updateDocument = TextDocument.prototype._update, getDocumentText = TextDocument.prototype.getText, parseDocumentUri = Uri.parse;

function supported(name, values) {
  return new Proxy(values, { get(target, key) {
    if (typeof key === 'symbol' || key === 'then' || key in target) return target[key];
    // This facade is a CommonJS export, not a transpiled ES module. Bundlers
    // probe the absent marker to preserve module.exports as their default.
    if (name === 'vscode' && key === '__esModule') return undefined;
    throw new Error(`VSCLI extension API is not implemented: ${name}.${key}`);
  } });
}

function createApi(sendRequest, notify, sessionOptions = {}) {
  const documents = new Map(), editors = new Map(), commands = new Map(), editorStates = new WeakMap();
  const workspaceEditMirrors = new Map(), documentMirrorRecords = new WeakMap();
  let workspaceEditConfigurationIdentity = {};
  const changed = new EventEmitter(), opened = new EventEmitter(), closed = new EventEmitter();
  const activeChanged = new EventEmitter(), saved = new EventEmitter(), selectionChanged = new EventEmitter();
  const configuration = createConfiguration();
  let active, workspaceRoot, activation, generation = -1;
  const facades = new Map(), owned = new Map();
  const callers = createCallerContext(owner => { assertActiveOwner(owner); sessionOptions.assertOwner?.(owner); });
  const mementos = createMementos(request, sessionOptions.session, sessionOptions.extensionState);
  const documentServices = createDocumentServices(request, sessionOptions, {
    generation: () => generation, root: () => workspaceRoot, callerOwner: callers.currentOwner,
    captureCaller: owner => callers.bind(owner, () => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }),
    snapshot: id => workspaceEditMirrors.get(id), snapshots: () => workspaceEditMirrors.values(),
    snapshotFor: document => documentMirrorRecords.get(document),
    document: id => documents.get(id), editor: id => active?.document._snapshot.id === id ? active : undefined,
    openVirtual: (owner,uri) => textContentProviders.open(owner,uri),
    editorCurrent: document => {
      const source = documentMirrorRecords.get(document);
      return !!source && editorStates.get(editors.get(source.document))?.document === document;
    },
    stageSelections: (owner, document, values) => {
      const source = documentMirrorRecords.get(document), state = editorStates.get(editors.get(source.document));
      const overlay = { owner, values }; state.overlay = overlay;
      return () => { if (state.overlay === overlay) state.overlay = undefined; };
    },
    selectionFailure: (owner,error) => notify('message',{session:sessionOptions.session,owner,
      text:`Editor selection update failed: ${String(error.message || error).slice(0,1024)}`}),
  });
  // Runtime-private mirror authority is captured before user value getters.
  const workspaceEdits = createWorkspaceEdits(documentServices.call, {
    notice: (owner, text) => messageFor(owner, text),
  }, {
    snapshots: () => workspaceEditMirrors.values(), snapshot: id => workspaceEditMirrors.get(id),
    captureCaller: owner => callers.bind(owner, () => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }),
    refactoringPolicy: () => ({ identity: workspaceEditConfigurationIdentity,
      autoSave: configuration.get('files.refactoring').get('autoSave', true) }),
  });
  const fileSearch = createFileSearch(documentServices.call, {
    unowned: callers.unowned,
    cancel: (owner, request) => notify('cancelNativeFindFiles', { session: sessionOptions.session, owner, request }),
  }, {
    captureCaller: owner => callers.bind(owner, () => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }),
    workspace: () => ({ root: workspaceRoot, identity: workspaceEditConfigurationIdentity }),
  });
  const reserved = new Set(sessionOptions.reservedCommands || []);
  let registrationCount = 0, commandCalls = 0;
  const promptBudget = { pending: 0, bytes: 0 };
  const fileSystem = filesystem.createFileSystem({ assertOwner, track, bindProvider: callers.bindProvider, invokeProviderResult: callers.invokeProviderResult, bindProviderResult: callers.bindProviderResult, bindProviderCleanup: callers.bindProviderCleanup });
  const fileWatchers = createFileWatchers({
    captureCaller: owner => callers.bind(owner, () => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }),
    track,event,unowned:callers.unowned,fileSystem,workspaceRoot:()=>workspaceRoot,
    configuration:()=>configuration.get('files'),
    diagnostic:(owner,error)=>notify('message',{session:sessionOptions.session,owner,
      text:`Filesystem watcher failed: ${String(error.message || error).slice(0,1024)}`}),
  });
  const textContentProviders = createTextContentProviders({
    session:sessionOptions.session,notify,call:documentServices.call,track,unowned:callers.unowned,
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    bindProvider:callers.bindProvider,bindProviderResult:callers.bindProviderResult,
    invokeProviderResult:callers.invokeProviderResult,bindProviderCleanup:callers.bindProviderCleanup,
    mirrors:()=>workspaceEditMirrors.values(),mirror:id=>workspaceEditMirrors.get(id),
    diagnostic:(owner,error)=>notify('message',{session:sessionOptions.session,owner,text:`Virtual document refresh failed: ${error.message}`}),
  });
  const workspaceScope = createWorkspaceScope({ Uri, EventEmitter, pathCaseSensitive: fileSystem.pathCaseSensitive });
  function workspace(values) {
    // Preserve getters so callers receive the latest native folder metadata.
    for (const key of ['workspaceFolders', 'rootPath', 'name', 'workspaceFile', 'getWorkspaceFolder', 'asRelativePath']) {
      Object.defineProperty(values, key, Object.getOwnPropertyDescriptor(workspaceScope.api, key));
    }
    return supported('workspace', values);
  }
  function assertActiveOwner(owner) {
    if (owner && activation && !activation.accepts(owner)) throw new Error(`Extension owner is not active: ${owner}`);
  }
  function assertOwner(owner) {
    callers.assertInherited();
    assertActiveOwner(owner);
  }
  function coreOperation(operation, params) {
    try {
      callers.assertInherited();
      // Internal cancellation/release listeners cannot acquire a user lease.
      // Keep the host's separate command-origin ALS unchanged.
      return callers.unowned(() => operation(params));
    } catch (error) { return Promise.reject(error); }
  }
  function request(method, params, lifecycle) {
    try { assertOwner(params.owner); return sendRequest(method, params, lifecycle); }
    catch (error) { return Promise.reject(error); }
  }
  const providers = createProviders({ session: sessionOptions.session, notify, track, bindProvider: callers.bindProvider, bindProviderResult: callers.bindProviderResult, invokeProviderResult: callers.invokeProviderResult, document: id => documents.get(id), languageEpoch: document => documentMirrorRecords.get(document)?.languageEpoch, diagnostics: (document,range) => diagnostics.currentDiagnostics(document,range) });
  const diagnostics = createDiagnostics({ session: sessionOptions.session, notify, track, assertOwner, document: key => {
    for (const document of documents.values()) if (document._snapshot.uri === key) return document;
  } });
  const surfaces = createSurfaces(notify, sessionOptions.session, track,
    (owner, id, args) => commandsFor(owner).executeCommand(id, ...args), owner => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }, callers);
  function track(owner, disposable) {
    try { assertOwner(owner); } catch (error) { disposable.dispose(); throw error; }
    if (registrationCount >= 4096) { disposable.dispose(); throw new Error('Extension registration limit reached'); }
    if (!owned.has(owner)) owned.set(owner, new Set());
    const entries = owned.get(owner);
    registrationCount++;
    const wrapped = new Disposable(() => { disposable.dispose(); entries.delete(wrapped); registrationCount--; });
    entries.add(wrapped);
    return wrapped;
  }
  function event(owner, source, transform = value => value) {
    return (listener, thisArg, disposables) => {
      const result = track(owner, callers.run(owner, () => source(callers.bind(owner, value => listener.call(thisArg, transform(value))))));
      if (disposables) disposables.push(result);
      return result;
    };
  }
  function commandSnapshot() { return [...commands].map(([id, entry]) => ({ id, owner: entry.owner })); }
  function publishCommands() { notify('commands', { session: sessionOptions.session, commands: commandSnapshot() }); }
  function commandsFor(owner) {
    return supported('commands', {
      registerCommand(id, callback, thisArg) {
        assertOwner(owner);
        if (typeof id !== 'string' || !id || id.length > 1024 || typeof callback !== 'function') throw new Error(`Invalid command: ${id}`);
        if (reserved.has(id) || id.startsWith('cursor')) throw new Error(`Native command is reserved: ${id}`);
        if (commands.has(id)) throw new Error(`Duplicate command: ${id}; registered by ${commands.get(id).owner}`);
        if (commands.size >= 1024) throw new Error('Extension command limit reached');
        commands.set(id, { owner, callback: callers.bindAsync(owner, (...args) => callback.apply(thisArg, args)) });
        const disposable = track(owner, new Disposable(() => { commands.delete(id); publishCommands(); }));
        publishCommands();
        return disposable;
      },
      async executeCommand(id, ...args) {
        assertOwner(owner);
        const command = commands.get(id);
        if (!command && NATIVE_COMMANDS.includes(id)) return documentServices.execute(owner, id, args);
        if (!command) throw new Error(`VSCLI cannot execute unregistered extension command: ${id}`);
        if (commandCalls >= 64) throw new Error('Extension command execution limit reached');
        commandCalls++;
        try { return await command.callback(...args); } finally { commandCalls--; }
      },
      async getCommands() { return [...NATIVE_COMMANDS, ...commands.keys()]; },
    });
  }
  function sourceEnd(text) {
    // One bounded pass over a newly transmitted snapshot; retain only the end
    // coordinate instead of allocating another full-document line array.
    let line = 0, start = 0;
    for (let i = 0; i < text.length; i++) {
      const code = text.charCodeAt(i);
      if (code === 13 || code === 10) {
        if (code === 13 && text.charCodeAt(i + 1) === 10) i++;
        line++; start = i + 1;
      }
    }
    return new Position(line, text.length - start);
  }
  function sync(state) {
    // A newer notification can arrive before an edit promise callback runs.
    if (state.generation <= generation) return;
    generation = state.generation;
    const changes = [];
    const present = new Set(state.documents.map(snapshot => snapshot.id));
    function retireDocument(document, record) {
      // Stable ExtHostDocumentData.dispose retains URI/text while closing the
      // public identity. Never inspect extension-replaced snapshot metadata.
      const metadata = { ...record.metadata, isClosed: true, isDirty: false };
      document._snapshot = metadata;
      Reflect.apply(updateDocument, document, [metadata]);
      document._uri = Reflect.apply(parseDocumentUri, Uri, [record.uri]);
      documentMirrorRecords.set(document, Object.freeze({ ...record, isClosed: true, isDirty: false, metadata: Object.freeze(metadata) }));
      changes.push(() => closed.fire(document));
    }
    for (const snapshot of state.documents) {
      let document = documents.get(snapshot.id);
      const previous = workspaceEditMirrors.get(snapshot.id);
      const transition = !!previous && previous.uri !== snapshot.uri;
      const text = typeof snapshot.text === 'string' ? snapshot.text : previous?.text;
      const metadata = Object.freeze({ ...previous?.metadata, ...snapshot, text, isClosed: false });
      if (transition) {
        retireDocument(document, previous);
        documents.delete(snapshot.id); editors.delete(snapshot.id);
        document = undefined;
      }
      if (!document) {
        document = new TextDocument(metadata, { save: document => {
          // Shared documents acquire the invoking callback/timer owner NOW.
          // The facade which first constructed this mirror is not authority.
          const owner = callers.currentOwner();
          return documentServices.saveDocument(owner, document);
        } });
        documents.set(snapshot.id, document);
        const openedDocument = document;
        changes.push(() => opened.fire(openedDocument));
      } else {
        // Restore only native metadata before merging. Public _snapshot fields
        // are not authority for previous version, saved generation or URI.
        document._snapshot = previous.metadata;
        Reflect.apply(updateDocument, document, [metadata]);
      }
      const eol = metadata.eol === 2 || metadata.eol === '\r\n' ? 2 : metadata.eol === 1 || metadata.eol === '\n' ? 1 : text.includes('\r\n') ? 2 : 1;
      // TextDocument presents uniform EOLs even when the native source snapshot
      // has mixed delimiters. Share its original prepared/cached string; retain
      // that private value so extension getText overrides cannot alter events.
      const canonicalText = previous && text === previous.text && eol === previous.eol
        ? previous.canonicalText : Reflect.apply(getDocumentText, document, []);
      if (previous && !transition && snapshot.version !== previous.version && typeof snapshot.text === 'string') {
        const changedDocument = document;
        changes.push(() => changed.fire({ document: changedDocument, contentChanges: [{
          range: new Range(new Position(0, 0), previous.end),
          rangeOffset: 0, rangeLength: previous.canonicalText.length, text: canonicalText,
        }] }));
      }
      const languageChanged = previous && !transition && previous.languageId !== metadata.languageId;
      if (languageChanged) {
        const languageDocument = document;
        changes.push(() => {
          const current = documentMirrorRecords.get(languageDocument);
          if (!current || current.isClosed) return;
          const live = { ...languageDocument._snapshot };
          languageDocument._snapshot = { ...live, languageId: previous.languageId };
          try { closed.fire(languageDocument); }
          finally { languageDocument._snapshot = { ...live, languageId: metadata.languageId }; }
          opened.fire(languageDocument);
        });
      }
      if (previous && Number.isSafeInteger(snapshot.savedGeneration) && snapshot.savedGeneration > previous.savedGeneration) {
        const savedDocument = document;
        changes.push(() => saved.fire(savedDocument));
      }
      const record = Object.freeze({ document: snapshot.id, version: snapshot.version, uri: snapshot.uri, model: document, text,
        isReadonly: snapshot.isReadonly === true, languageId: metadata.languageId, isDirty: snapshot.isDirty === true,
        languageEpoch: Number.isSafeInteger(snapshot.languageEpoch) && snapshot.languageEpoch >= 0 ? snapshot.languageEpoch
          : (previous?.languageEpoch ?? 0) + (languageChanged ? 1 : 0),
        savedGeneration: Number.isSafeInteger(snapshot.savedGeneration) ? snapshot.savedGeneration : previous?.savedGeneration ?? 0,
        isClosed: false, metadata, canonicalText, eol,
        end: text === previous?.text ? previous.end : sourceEnd(text) });
      workspaceEditMirrors.set(snapshot.id, record); documentMirrorRecords.set(document, record);
      if (!editors.has(snapshot.id)) editors.set(snapshot.id, editorFor(snapshot.id, document));
      const liveEditorState = editorStates.get(editors.get(snapshot.id));
      if (previous && (previous.model !== document || previous.version !== record.version || previous.languageEpoch !== record.languageEpoch)) liveEditorState.overlay = undefined;
      const nativeSelections = snapshot.selections ?? (snapshot.id === state.active ? state.selections : undefined);
      if (nativeSelections !== undefined) {
        const base = editors.get(snapshot.id), editorState = editorStates.get(base);
        if (!Array.isArray(nativeSelections) || nativeSelections.length > 10000) throw new Error('Invalid native editor selection snapshot');
        const wire = nativeSelections.map(value => ({anchor:coordinates.position(value.anchor,'Mirror selection'),active:coordinates.position(value.active,'Mirror selection')}));
        if (!wire.length) wire.push({anchor:{line:0,character:0},active:{line:0,character:0}});
        const changedSelection = editorState.initialized && (wire.length !== editorState.wire.length || wire.some((value,index) => {
          const old = editorState.wire[index];
          return value.anchor.line !== old.anchor.line || value.anchor.character !== old.anchor.character || value.active.line !== old.active.line || value.active.character !== old.active.character;
        }));
        editorState.overlay = undefined; editorState.initialized = true; editorState.wire = wire;
        editorState.actual = wire.map(value => new Selection(value.anchor,value.active));
        base._selections = editorState.actual;
        if (changedSelection) {
          const kind = snapshot.selectionKind ?? state.selectionKind;
          const eventValue = {textEditor:base,selections:editorState.actual.slice(),kind:[1,2,3].includes(kind)?kind:undefined};
          changes.push(() => selectionChanged.fire(eventValue));
        }
      }
    }
    for (const [id, document] of documents) {
      if (!present.has(id)) {
        retireDocument(document, workspaceEditMirrors.get(id));
        documents.delete(id); editors.delete(id); workspaceEditMirrors.delete(id);
      }
    }
    const previousActive = active;
    active = editors.get(state.active);
    providers.documentChanged();
    diagnostics.documentChanged();
    textContentProviders.documentChanged();
    // All mirrors must reflect the new state before any extension callback runs.
    for (const fire of changes) fire();
    if (active !== previousActive) activeChanged.fire(active);
  }
  function editorFor(id, document, owner = '') {
    const initial = editorStates.get(editors.get(id));
    let result;
    const selected = () => {
      const current = editorStates.get(editors.get(id));
      const state = current?.document === document ? current : editorStates.get(result);
      return state.overlay?.values || state.actual;
    };
    result = supported('TextEditor', {
      document, _selections: [new Selection(0, 0, 0, 0)],
      get selection() { return selected()[0]; },
      set selection(value) { documentServices.setEditorSelections(owner || callers.currentOwner(),document,value,true); },
      get selections() { return selected().slice(); },
      set selections(value) { documentServices.setEditorSelections(owner || callers.currentOwner(),document,value,false); },
      edit(callback, options) {
        try {
          assertOwner(owner);
          const record = documentMirrorRecords.get(document);
          if (!record || record.isClosed || workspaceEditMirrors.get(id)?.model !== document) throw new Error('Document is no longer open');
        } catch (error) { return Promise.reject(error); }
        if (options && (options.undoStopBefore === false || options.undoStopAfter === false)) {
          return Promise.reject(new Error('VSCLI does not yet support grouped extension undo stops'));
        }
        const version = documentMirrorRecords.get(document).version;
        const edits = [];
        let valid = true;
        const validatePosition = value => {
          const point = coordinates.position(value, 'Edit');
          return document.validatePosition(new Position(point.line, point.character));
        };
        const validateRange = value => {
          if (!(value instanceof Range)) throw new TypeError('Invalid range');
          const bounds = coordinates.range(value, 'Edit');
          return document.validateRange(new Range(bounds.start, bounds.end));
        };
        const add = (range, newText) => {
          if (!valid) throw new Error('TextEditorEdit is only valid during its callback');
          if (typeof newText !== 'string') throw new TypeError('Edit text must be a string');
          if (edits.length >= 4096) throw new Error('Extension edit count limit exceeded');
          edits.push({ range: coordinates.range(range, 'Edit'), newText });
        };
        const builder = supported('TextEditorEdit', {
          replace: (range, text) => add(validateRange(range), text),
          insert: (position, text) => {
            const point = validatePosition(position);
            add(new Range(point, point), text);
          },
          delete: range => {
            const validRange = validateRange(range);
            if (!validRange.isEmpty) add(validRange, '');
          },
        });
        try { callback(builder); } catch (error) { return Promise.reject(error); }
        finally { valid = false; }
        // State notifications are processed before the native acknowledgement.
        return request('edit', { session: sessionOptions.session, owner, document: id, version, edits }).then(result => result.applied);
      },
    });
    editorStates.set(result,{document,id,actual:initial?.actual || [new Selection(0,0,0,0)],wire:[],initialized:false,overlay:undefined});
    return result;
  }
  const api = supported('vscode', {
    ...types, ...providerTypes, ...diagnosticTypes, ...surfaces.types, RelativePattern,
    FileSystemError: filesystem.FileSystemError, FileType: filesystem.FileType,
    FilePermission: filesystem.FilePermission, FileChangeType: filesystem.FileChangeType,
    version: '1.95.0',
    TextEditorSelectionChangeKind: Object.freeze({Keyboard:1,Mouse:2,Command:3}),
    ViewColumn: Object.freeze({ Active: -1, Beside: -2, One: 1, Two: 2, Three: 3, Four: 4, Five: 5, Six: 6, Seven: 7, Eight: 8, Nine: 9 }),
    ExtensionMode: Object.freeze({ Production: 1, Development: 2, Test: 3 }),
    window: supported('window', {
      ...createPrompts(request, sessionOptions.session, '', promptBudget),
      ...surfaces.forExtension(''),
      get activeTextEditor() { return active; },
      get visibleTextEditors() { return active ? [active] : []; },
      onDidChangeActiveTextEditor: activeChanged.event,
      onDidChangeTextEditorSelection: selectionChanged.event,
      showInformationMessage: message,
      showWarningMessage: message,
      showErrorMessage: message,
    }),
    workspace: workspace({
      ...fileSystem.forOwner(''),
      get textDocuments() { return [...documents.values()]; },
      onDidChangeTextDocument: changed.event,
      onDidOpenTextDocument: opened.event,
      onDidCloseTextDocument: closed.event,
      onDidSaveTextDocument: saved.event,
      getConfiguration: configuration.get,
      onDidChangeConfiguration: configuration.onDidChange,
    }),
    commands: commandsFor(''),
    get extensions() { return activation?.facade(); },
  });
  function message(text, ...items) { return messageFor('', text, ...items); }
  function messageFor(owner, text, ...items) {
    try { assertOwner(owner); } catch (error) { return Promise.reject(error); }
    if (items.length) return Promise.reject(new Error('VSCLI extension message choices are not implemented'));
    notify('message', { session: sessionOptions.session, owner, text: String(text).slice(0, 2048) });
    return Promise.resolve(undefined);
  }
  return {
    api, sync, textContentProviderSnapshot: textContentProviders.snapshot, providerSnapshot: providers.snapshot, provideLanguage: params => coreOperation(providers.provide, params), resolveLanguageCompletion: params => coreOperation(providers.resolveCompletion, params), resolveLanguageCodeAction: params => coreOperation(providers.resolveAction, params), cancelLanguageProvider: providers.cancel, updateConfiguration: next => { configuration.update(next); workspaceEditConfigurationIdentity = {}; fileWatchers.refresh(); }, commandSnapshot, assertOwner,
    setActivation(value) { activation = value; },
    // Private runtime hooks, intentionally absent from the vscode facade.
    runWithOwner: callers.run, currentCallerOwner: callers.currentOwner,
    contextForExtension(owner) { return { ...mementos.forOwner(owner), extension: activation?.extension(owner) }; },
    mergeExtensionState: mementos.merge,
    configureSurfaces: surfaces.configure, treeChildren: params => coreOperation(surfaces.treeChildren, params),
    surfaceAction: surfaces.action, treeEvent: surfaces.treeEvent,
    disposeOwner(owner) {
      return callers.unowned(() => {
        for (const cleanup of [() => fileWatchers.disposeOwner(owner), () => fileSystem.disposeOwnerWatches(owner)]) {
          try { cleanup(); } catch (error) { console.error(error); }
        }
        for (const base of editors.values()) {
          const state = editorStates.get(base);
          if (state?.overlay?.owner === owner) state.overlay = undefined;
        }
        callers.retire(owner);
        if (sessionOptions.languageProviders) providers.disposeOwner(owner);
        diagnostics.disposeOwner(owner);
        for (const disposable of [...(owned.get(owner) || [])]) {
          try { disposable.dispose(); } catch (error) { console.error(error); }
        }
        textContentProviders.disposeOwner(owner);
        owned.delete(owner); facades.delete(owner);
      });
    },
    forExtension(owner) {
      if (facades.has(owner)) return facades.get(owner);
      const scopedEditors = new WeakMap();
      function scopedEditor(base) {
        if (!base) return undefined;
        const source = editorStates.get(base);
        if (!source) return undefined;
        if (!scopedEditors.has(source.document)) scopedEditors.set(source.document, editorFor(source.id, source.document, owner));
        return scopedEditors.get(source.document);
      }
      const services = documentServices.forOwner(owner, scopedEditor);
      // The native document objects are shared; only request-producing editor handles are scoped.
      const facade = supported('vscode', {
        ...api,
        languages: supported('languages', { ...diagnostics.forOwner(owner), ...(sessionOptions.languageProviders ? providers.forOwner(owner) : {}),
          ...(sessionOptions.documentLanguage ? {setTextDocumentLanguage:(document,language)=>documentServices.setTextDocumentLanguage(owner,document,language)} : {}) }),
        window: supported('window', {
          ...createPrompts(request, sessionOptions.session, owner, promptBudget),
          showTextDocument: services.showTextDocument,
          ...surfaces.forExtension(owner),
          get activeTextEditor() { return scopedEditor(active); },
          get visibleTextEditors() { return active ? [scopedEditor(active)] : []; },
          onDidChangeActiveTextEditor: event(owner, activeChanged.event, scopedEditor),
          onDidChangeTextEditorSelection: event(owner,selectionChanged.event,value=>({...value,textEditor:scopedEditor(value.textEditor)})),
          showInformationMessage: (text, ...items) => messageFor(owner, text, ...items),
          showWarningMessage: (text, ...items) => messageFor(owner, text, ...items),
          showErrorMessage: (text, ...items) => messageFor(owner, text, ...items),
        }),
        workspace: workspace({
          ...fileSystem.forOwner(owner),
          ...fileWatchers.forOwner(owner),
          openTextDocument: services.openTextDocument,
          // Root enables this only after the native grouped transaction gateway qualifies.
          ...(sessionOptions.workspaceEdits ? workspaceEdits.forOwner(owner) : {}),
          ...(sessionOptions.findFiles ? fileSearch.forOwner(owner) : {}),
          ...(sessionOptions.virtualDocuments ? textContentProviders.forOwner(owner) : {}),
          get textDocuments() { return [...documents.values()]; },
          onDidChangeTextDocument: event(owner, changed.event),
          onDidOpenTextDocument: event(owner, opened.event),
          onDidCloseTextDocument: event(owner, closed.event),
          onDidSaveTextDocument: event(owner, saved.event),
          getConfiguration: configuration.get,
          onDidChangeConfiguration: event(owner, configuration.onDidChange),
        }),
        commands: commandsFor(owner),
        get extensions() {
          const registry = activation.facade();
          return supported('extensions', {
            getExtension: registry.getExtension,
            get all() { return registry.all; },
            onDidChange: event(owner, registry.onDidChange),
          });
        },
      });
      facades.set(owner, facade);
      return facade;
    },
    configure(root, schema, layers) {
      const uri = Uri.file(root), name = require('node:path').basename(root);
      // Trust/events are deliberately unexposed until a native authority exists;
      // this private false sentinel is not a workspace security policy.
      workspaceScope.publish({ generation: workspaceScope.generation + 1, trusted: false,
        workspace: { name, folders: [{ uri, name }] } }, { emit: false });
      workspaceRoot = root;
      configuration.initialize(schema, layers);
      workspaceEditConfigurationIdentity = {};
      fileWatchers.refresh();
    },
  };
}
module.exports = { createApi, supported };
