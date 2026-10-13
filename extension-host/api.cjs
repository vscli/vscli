'use strict';
const types = require('./api-types.cjs');
const { createCallerContext } = require('./caller-context.cjs');
const coordinates = require('./coordinates.cjs');
const diagnosticTypes = require('./diagnostic-types.cjs');
const { createDiagnostics } = require('./diagnostics.cjs');
const providerTypes = require('./provider-types.cjs');
const { createProviders } = require('./providers.cjs');
const {createNavigationProviders}=require('./navigation-providers.cjs');
const {createRenameProviders}=require('./rename-providers.cjs');
const { createPrompts } = require('./prompts.cjs');
const { createSurfaces } = require('./surfaces.cjs');
const { createConfiguration } = require('./configuration.cjs');
const { createWorkspaceEdits } = require('./workspace-edits.cjs');
const { RelativePattern, createFileSearch } = require('./workspace-files.cjs');
const { createFileWatchers } = require('./file-watchers.cjs');
const { createFileOperations } = require('./file-operations.cjs');
const { createTextContentProviders } = require('./text-content-providers.cjs');
const editorPresentation = require('./editor-presentation.cjs');
const {createEditorDecorations} = require('./editor-decorations.cjs');
const {createEditorSnippets} = require('./editor-snippets.cjs');
const {createEditorTabs} = require('./editor-tabs.cjs');
const tabInputs = require('./tab-inputs.cjs');
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
  const documents = new Map(), editors = new Map(), explicitEditors = new Map(), commands = new Map(), editorStates = new WeakMap();
  let visibleEditors = [], explicitInventory = false;
  const workspaceEditMirrors = new Map(), documentMirrorRecords = new WeakMap();
  let workspaceEditConfigurationIdentity = {};
  const changed = new EventEmitter(), opened = new EventEmitter(), closed = new EventEmitter();
  const activeChanged = new EventEmitter(), visibleChanged = new EventEmitter(), columnChanged = new EventEmitter(), saved = new EventEmitter(), selectionChanged = new EventEmitter(), optionsChanged = new EventEmitter(), rangesChanged = new EventEmitter();
  const configuration = createConfiguration();
  let active, workspaceRoot, activation, generation = -1;
  const facades = new Map(), owned = new Map();
  const callers = createCallerContext(owner => { assertActiveOwner(owner); sessionOptions.assertOwner?.(owner); });
  const mementos = createMementos(request, sessionOptions.session, sessionOptions.extensionState);
  const documentServices = createDocumentServices(request, sessionOptions, {
    generation: () => generation, root: () => workspaceRoot, callerOwner: callers.currentOwner, unowned: callers.unowned,
    customerRequest: (method, params, lifecycle) => callers.unowned(() => sendRequest(method, params, lifecycle)),
    captureCaller: owner => callers.bind(owner, () => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }),
    snapshot: id => workspaceEditMirrors.get(id), snapshots: () => workspaceEditMirrors.values(),
    snapshotFor: document => documentMirrorRecords.get(document),
    document: id => documents.get(id),
    editor: (id, nativeId) => nativeId === undefined
      ? (!explicitInventory && editorStates.get(active)?.id === id ? active : undefined)
      : explicitEditors.get(nativeId),
    openVirtual: (owner,uri) => textContentProviders.open(owner,uri),
    editorSnapshot: base => editorStates.get(base),
    editorCurrent: base => {
      const state = editorStates.get(base);
      return !!state?.alive && (state.nativeId === undefined ? !explicitInventory && editors.get(state.id) === base : explicitEditors.get(state.nativeId) === base)
        && workspaceEditMirrors.get(state.id)?.model === state.document;
    },
    stageSelections: (owner, base, values) => {
      const state = editorStates.get(base), overlay = { owner, values }; state.overlay = overlay;
      return () => { if (state.overlay === overlay) state.overlay = undefined; };
    },
    editorOptions: base => editorStates.get(base)?.presentation.current(),
    stageOptions: (owner,base,values) => editorStates.get(base).presentation.stage(owner,values),
    presentationFailure: (owner,error) => notify('message',{session:sessionOptions.session,owner,text:`Editor presentation update failed: ${String(error.message || error).slice(0,1024)}`}),
    selectionFailure: (owner,error) => notify('message',{session:sessionOptions.session,owner,
      text:`Editor selection update failed: ${String(error.message || error).slice(0,1024)}`}),
  });
  // Runtime-private mirror authority is captured before user value getters.
  const workspaceEdits = createWorkspaceEdits(documentServices.call, {
    notice: (owner, text) => messageFor(owner, text),
  }, {
    snapshots: () => workspaceEditMirrors.values(), snapshot: id => workspaceEditMirrors.get(id),
    unopenedEdits:sessionOptions.workspaceUnopenedEdits===true,
    refactoringSaves:sessionOptions.workspaceRefactoringSaves===true&&sessionOptions.workspaceUnopenedEdits===true,
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
    session:sessionOptions.session,generatedLoads:sessionOptions.sharedGeneratedDocumentLoads===true,notify,call:documentServices.call,track,unowned:callers.unowned,
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    bindProvider:callers.bindProvider,bindProviderResult:callers.bindProviderResult,
    invokeProviderResult:callers.invokeProviderResult,bindProviderCleanup:callers.bindProviderCleanup,
    mirrors:()=>workspaceEditMirrors.values(),mirror:id=>workspaceEditMirrors.get(id),
    diagnostic:(owner,error)=>notify('message',{session:sessionOptions.session,owner,text:`Virtual document refresh failed: ${error.message}`}),
  });
  const workspaceScope = createWorkspaceScope({ Uri, EventEmitter, pathCaseSensitive: fileSystem.pathCaseSensitive });
  const fileOperations = createFileOperations(sessionOptions.session);
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
    } catch (error) {
      if(operation===providers.provide||operation===providers.resolveCompletion||operation===providers.resolveAction||operation===providers.provideWorkspaceSymbols||operation===providers.resolveWorkspaceSymbol||operation===providers.prepareRename||operation===providers.provideRenameEdits){
        try{providers.releaseRefusal(params);}catch{/* Preserve the original pre-entry error. */}
      }
      return Promise.reject(error);
    }
  }
  function request(method, params, lifecycle) {
    try { assertOwner(params.owner); return sendRequest(method, params, lifecycle); }
    catch (error) { return Promise.reject(error); }
  }
  const providers = createProviders({ session: sessionOptions.session, notify, track, bindProvider: callers.bindProvider, bindProviderResult: callers.bindProviderResult, invokeProviderResult: callers.invokeProviderResult, document: id => documents.get(id), documentSource: document => documentMirrorRecords.get(document), captureCaller: owner => callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}), workspaceIdentity:()=>workspaceEditConfigurationIdentity,unowned:callers.unowned, languageEpoch: document => documentMirrorRecords.get(document)?.languageEpoch, diagnostics: (document,range) => diagnostics.currentDiagnostics(document,range) });
  const navigationProviders=createNavigationProviders({
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    workspaceIdentity:()=>workspaceEditConfigurationIdentity,
    mirrors:()=>workspaceEditMirrors.values(),snapshot:id=>workspaceEditMirrors.get(id),sourceFor:document=>documentMirrorRecords.get(document),
    open:(owner,uri)=>documentServices.forOwner(owner).openTextDocument(uri),
    query:providers.query,
  });
  const renameProviders=createRenameProviders({
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    workspaceIdentity:()=>workspaceEditConfigurationIdentity,mirrors:()=>workspaceEditMirrors.values(),snapshot:id=>workspaceEditMirrors.get(id),sourceFor:document=>documentMirrorRecords.get(document),
    open:(owner,uri)=>documentServices.forOwner(owner).openTextDocument(uri),query:providers.queryRename,
  });
  const diagnostics = createDiagnostics({ session: sessionOptions.session, notify, track, assertOwner, document: key => {
    for (const document of documents.values()) if (document._snapshot.uri === key) return document;
  } });
  const surfaces = createSurfaces(notify, sessionOptions.session, track,
    (owner, id, args) => commandsFor(owner).executeCommand(id, ...args), owner => { assertOwner(owner); sessionOptions.assertOwner?.(owner); }, callers);
  const decorations = createEditorDecorations({
    enabled:sessionOptions.editorDecorations===true,
    session:sessionOptions.session,notify,call:documentServices.call,track,
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    captureEditor:(owner,document,base)=>documentServices.editorServiceSource(owner,document,base,'editorDecorations'),
    diagnostic:(owner,error)=>notify('message',{session:sessionOptions.session,owner,text:`Editor decorations update failed: ${String(error.message || error).slice(0,1024)}`}),
  });
  const editorSnippets = createEditorSnippets({
    enabled:sessionOptions.editorSnippets===true,call:documentServices.call,
    captureEditor:(owner,document,base)=>documentServices.editorServiceSource(owner,document,base,'editorSnippets'),
  });
  const editorTabs = createEditorTabs({
    enabled:sessionOptions.editorTabs===true,call:documentServices.call,
    captureCaller:owner=>callers.bind(owner,()=>{assertOwner(owner);sessionOptions.assertOwner?.(owner);}),
    bindEvent:(owner,source)=>event(owner,source),
  });
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
        if(id==='vscode.executeWorkspaceSymbolProvider'){if(!sessionOptions.languageProviders)throw new Error('Native language providers are unavailable');return providers.queryWorkspaceSymbols(owner,args[0]);}
        if(renameProviders.commands.includes(id)){if(!sessionOptions.languageProviders)throw new Error('Native language providers are unavailable');return renameProviders.execute(owner,id,args);}
        if(navigationProviders.commands.includes(id)){if(!sessionOptions.languageProviders)throw new Error('Native language providers are unavailable');return navigationProviders.execute(owner,id,args);}
        const command = commands.get(id);
        if (!command && NATIVE_COMMANDS.includes(id)) return documentServices.execute(owner, id, args);
        if (!command) throw new Error(`VSCLI cannot execute unregistered extension command: ${id}`);
        if (commandCalls >= 64) throw new Error('Extension command execution limit reached');
        commandCalls++;
        try { return await command.callback(...args); } finally { commandCalls--; }
      },
      async getCommands() { return [...NATIVE_COMMANDS, ...(sessionOptions.languageProviders?[...navigationProviders.commands,...renameProviders.commands,'vscode.executeWorkspaceSymbolProvider']:[]), ...commands.keys()]; },
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
  function mirrorSelections(base, nativeSelections, kind, changes) {
    const state = editorStates.get(base);
    const wire = nativeSelections.map(value => ({anchor:coordinates.position(value.anchor,'Mirror selection'),active:coordinates.position(value.active,'Mirror selection')}));
    if (!wire.length) wire.push({anchor:{line:0,character:0},active:{line:0,character:0}});
    const changed = state.initialized && (wire.length !== state.wire.length || wire.some((value,index) => {
      const old = state.wire[index];
      return value.anchor.line !== old.anchor.line || value.anchor.character !== old.anchor.character || value.active.line !== old.active.line || value.active.character !== old.active.character;
    }));
    state.overlay = undefined; state.initialized = true; state.wire = wire;
    state.actual = wire.map(value => new Selection(value.anchor,value.active));
    base._selections = state.actual;
    if (changed) {
      const eventValue = {textEditor:base,selections:state.actual.slice(),kind:[1,2,3].includes(kind)?kind:undefined};
      changes.push(() => selectionChanged.fire(eventValue));
    }
  }
  function retireEditor(base) {
    const state = editorStates.get(base);
    if (state) { state.alive = false; state.overlay = undefined; state.presentation.retire(); }
  }
  function validateEditorInventory(state) {
    if (!Object.hasOwn(state,'editors')) return undefined;
    if (!Array.isArray(state.editors) || state.editors.length > 4) throw new Error('Invalid native editor inventory');
    const ids = new Set(), documents = new Set(state.documents.map(value=>value.id));
    for (const entry of state.editors) {
      if (!entry || typeof entry.id !== 'string' || !entry.id || entry.id.includes('\0') || entry.id.length > 128 || Buffer.byteLength(entry.id) > 128 || ids.has(entry.id) ||
          !Number.isSafeInteger(entry.document) || entry.document <= 0 || !documents.has(entry.document) || !Number.isInteger(entry.viewColumn) || entry.viewColumn < 1 || entry.viewColumn > 4 ||
          !Array.isArray(entry.selections) || entry.selections.length > 10000) throw new Error('Invalid native editor inventory');
      ids.add(entry.id);
      editorPresentation.decodeOptions(entry.options); editorPresentation.decodeVisibleRanges(entry.visibleRanges);
      for (const value of entry.selections) { coordinates.position(value.anchor,'Mirror selection'); coordinates.position(value.active,'Mirror selection'); }
    }
    if (state.activeEditor !== null && (typeof state.activeEditor !== 'string' || !ids.has(state.activeEditor))) throw new Error('Invalid native active editor');
    return state.editors;
  }
  function sync(state) {
    // A newer notification can arrive before an edit promise callback runs.
    if (state.generation <= generation) return;
    const inventory = validateEditorInventory(state);
    const tabPlan = sessionOptions.editorTabs===true ? editorTabs.prepare(state.tabs) : undefined;
    generation = state.generation;
    const changes = [];
    const previousVisible = visibleEditors, previousActive = active;
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
        documents.delete(snapshot.id); retireEditor(editors.get(snapshot.id)); editors.delete(snapshot.id);
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
      if (!inventory) {
        if (!editors.has(snapshot.id)) editors.set(snapshot.id, editorFor(snapshot.id, document));
        const base = editors.get(snapshot.id), live = editorStates.get(base);
        if (previous && (previous.model !== document || previous.version !== record.version || previous.languageEpoch !== record.languageEpoch)) live.overlay = undefined;
        const selections = snapshot.selections ?? (snapshot.id === state.active ? state.selections : undefined);
        if (selections !== undefined) {
          if (!Array.isArray(selections) || selections.length > 10000) throw new Error('Invalid native editor selection snapshot');
          mirrorSelections(base,selections,snapshot.selectionKind ?? state.selectionKind,changes);
        }
      }

    }
    for (const [id, document] of documents) {
      if (!present.has(id)) {
        retireDocument(document, workspaceEditMirrors.get(id));
        documents.delete(id); retireEditor(editors.get(id)); editors.delete(id); workspaceEditMirrors.delete(id);
      }
    }
    explicitInventory = inventory !== undefined;
    if (explicitInventory) {
      for (const base of editors.values()) retireEditor(base);
      editors.clear();
      const presentEditors = new Set();
      visibleEditors = inventory.map(entry => {
        presentEditors.add(entry.id);
        const document = documents.get(entry.document);
        let base = explicitEditors.get(entry.id), privateState = editorStates.get(base);
        if (!base || privateState.document !== document) {
          retireEditor(base);
          base = editorFor(entry.document,document,'',undefined,entry.id);
          explicitEditors.set(entry.id,base); privateState = editorStates.get(base);
        }
        if (privateState.viewColumn !== undefined && privateState.viewColumn !== entry.viewColumn) {
          const eventValue = {textEditor:base,viewColumn:entry.viewColumn};
          changes.push(()=>columnChanged.fire(eventValue));
        }
        privateState.viewColumn = entry.viewColumn;
        const presentation = privateState.presentation.accept(editorPresentation.decodeOptions(entry.options),editorPresentation.decodeVisibleRanges(entry.visibleRanges));
        if (presentation.optionsChanged) {
          const eventValue = {textEditor:base,options:{...presentation.options}};
          changes.push(()=>optionsChanged.fire(eventValue));
        }
        mirrorSelections(base,entry.selections,entry.selectionKind,changes);
        if (presentation.rangesChanged) {
          const eventValue = {textEditor:base,visibleRanges:presentation.visibleRanges};
          changes.push(()=>rangesChanged.fire(eventValue));
        }
        return base;
      });
      for (const [id,base] of explicitEditors) if (!presentEditors.has(id)) { retireEditor(base); explicitEditors.delete(id); }
      active = explicitEditors.get(state.activeEditor);
    } else {
      for (const base of explicitEditors.values()) retireEditor(base);
      explicitEditors.clear();
      active = editors.get(state.active);
      visibleEditors = active ? [active] : [];
    }
    const changedVisible = previousVisible.length !== visibleEditors.length || previousVisible.some(value=>!visibleEditors.includes(value));
    const fireTabs = tabPlan ? editorTabs.publish(tabPlan) : undefined;
    providers.documentChanged();
    diagnostics.documentChanged();
    textContentProviders.documentChanged();
    // All mirrors must reflect the new state before any extension callback runs.
    for (const fire of changes) fire();
    if (changedVisible) visibleChanged.fire(visibleEditors.slice());
    if (active !== previousActive) activeChanged.fire(active);
    fireTabs?.();
  }
  function editorFor(id, document, owner = '', base = undefined, nativeId = undefined) {
    const privateState = base ? editorStates.get(base) : {document,id,nativeId,alive:true,actual:[new Selection(0,0,0,0)],wire:[],initialized:false,overlay:undefined,viewColumn:undefined,presentation:editorPresentation.createEditorPresentation()};
    const ownerGuard = owner ? callers.bind(owner,()=>assertOwner(owner)) : ()=>assertOwner(owner);
    let result;
    const selected = () => privateState.overlay?.values || privateState.actual;
    const original = () => base || result;
    const current = () => privateState.alive && (privateState.nativeId === undefined ? !explicitInventory && editors.get(id) === original() : explicitEditors.get(privateState.nativeId) === original());
    const optionValues = privateState.presentation.optionsValue((value,field)=>{
      ownerGuard(); documentServices.setEditorOptions(owner || callers.currentOwner(),document,value,field,original());
    });
    result = supported('TextEditor', {
      insertSnippet(snippet,where,options) {
        try {ownerGuard();return editorSnippets.insertSnippet(owner || callers.currentOwner(),document,original(),snippet,where,options);}
        catch(error) {return Promise.reject(error);}
      },
      document, get viewColumn() { return privateState.viewColumn; }, _selections: [new Selection(0, 0, 0, 0)],
      get options() { return optionValues; },
      set options(value) { ownerGuard(); documentServices.setEditorOptions(owner || callers.currentOwner(),document,value,undefined,original()); },
      get visibleRanges() { return privateState.presentation.visibleRanges(); },
      revealRange(range,revealType) { ownerGuard(); documentServices.revealEditorRange(owner || callers.currentOwner(),document,range,revealType,original()); },
      setDecorations(type,ranges) { ownerGuard(); decorations.setDecorations(owner || callers.currentOwner(),document,original(),type,ranges); },
      get selection() { return selected()[0]; },
      set selection(value) { ownerGuard(); documentServices.setEditorSelections(owner || callers.currentOwner(),document,value,true,original()); },
      get selections() { return selected().slice(); },
      set selections(value) { ownerGuard(); documentServices.setEditorSelections(owner || callers.currentOwner(),document,value,false,original()); },
      edit(callback, options) {
        try {
          ownerGuard();
          const record = documentMirrorRecords.get(document);
          if (!record || record.isClosed || workspaceEditMirrors.get(id)?.model !== document) throw new Error('Document is no longer open');
          if (!current()) throw new Error('Text editor is no longer available');
        } catch (error) { return Promise.reject(error); }
        if (options && (options.undoStopBefore === false || options.undoStopAfter === false)) {
          return Promise.reject(new Error('VSCLI does not yet support grouped extension undo stops'));
        }
        const capturedSource = documentMirrorRecords.get(document), version = capturedSource.version;
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
        const add = (range, newText, forceMoveMarkers = false) => {
          if (!valid) throw new Error('TextEditorEdit is only valid during its callback');
          if (typeof newText !== 'string') throw new TypeError('Edit text must be a string');
          if (edits.length >= 4096) throw new Error('Extension edit count limit exceeded');
          edits.push({ range: coordinates.range(range, 'Edit'), newText, ...(forceMoveMarkers ? {forceMoveMarkers:true} : {}) });
        };
        const builder = supported('TextEditorEdit', {
          replace: (range, text) => add(validateRange(range), text),
          insert: (position, text) => {
            const point = validatePosition(position);
            add(new Range(point, point), text, true);
          },
          delete: range => {
            const validRange = validateRange(range);
            if (!validRange.isEmpty) add(validRange, '', true);
          },
        });
        try { callback(builder); } catch (error) { return Promise.reject(error); }
        finally { valid = false; }
        // Callback/getter reentrancy cannot borrow a replacement pane.
        try {
          ownerGuard();
          if (!current() || workspaceEditMirrors.get(id)?.model !== document || documentMirrorRecords.get(document)?.version !== version || documentMirrorRecords.get(document)?.languageEpoch !== capturedSource.languageEpoch) throw new Error('Text editor source changed during edit capture');
        } catch (error) { return Promise.reject(error); }
        return request('edit', { session: sessionOptions.session, owner, document: id, version, edits,
          ...(privateState.nativeId === undefined ? {} : {editor:privateState.nativeId}) }).then(result => { ownerGuard(); return result.applied; });
      },
    });
    editorStates.set(result,privateState);
    return result;
  }
  const api = supported('vscode', {
    ...types, ...providerTypes, ...diagnosticTypes, ...surfaces.types, ...tabInputs, RelativePattern,
    ...decorations.types,
    FileSystemError: filesystem.FileSystemError, FileType: filesystem.FileType,
    FilePermission: filesystem.FilePermission, FileChangeType: filesystem.FileChangeType,
    version: '1.95.0',
    TextEditorCursorStyle:editorPresentation.TextEditorCursorStyle, TextEditorLineNumbersStyle:editorPresentation.TextEditorLineNumbersStyle, TextEditorRevealType:editorPresentation.TextEditorRevealType,
    TextEditorSelectionChangeKind: Object.freeze({Keyboard:1,Mouse:2,Command:3}),
    ViewColumn: Object.freeze({ Active: -1, Beside: -2, One: 1, Two: 2, Three: 3, Four: 4, Five: 5, Six: 6, Seven: 7, Eight: 8, Nine: 9 }),
    ExtensionMode: Object.freeze({ Production: 1, Development: 2, Test: 3 }),
    window: supported('window', {
      tabGroups:editorTabs.forOwner(''),
      ...createPrompts(request, sessionOptions.session, '', promptBudget),
      ...surfaces.forExtension(''),
      ...decorations.forOwner(''),
      get activeTextEditor() { return active; },
      get visibleTextEditors() { return visibleEditors.slice(); },
      onDidChangeActiveTextEditor: activeChanged.event,
      onDidChangeTextEditorSelection: selectionChanged.event,
      onDidChangeTextEditorOptions: optionsChanged.event,
      onDidChangeTextEditorVisibleRanges: rangesChanged.event,
      onDidChangeVisibleTextEditors: visibleChanged.event,
      onDidChangeTextEditorViewColumn: columnChanged.event,
      showInformationMessage: message,
      showWarningMessage: message,
      showErrorMessage: message,
    }),
    workspace: workspace({
      ...fileOperations.api,
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
    fileOperation: fileOperations.publish,
    api, sync, textContentProviderSnapshot: textContentProviders.snapshot, providerSnapshot: providers.snapshot, rankLanguageProviders: providers.rank, prepareLanguageRename:params=>coreOperation(providers.prepareRename,params), provideLanguageRename:params=>coreOperation(providers.provideRenameEdits,params), provideWorkspaceSymbols:params=>coreOperation(providers.provideWorkspaceSymbols,params), resolveLanguageWorkspaceSymbol:params=>coreOperation(providers.resolveWorkspaceSymbol,params), retireWorkspaceSymbols:providers.retireWorkspaceSymbols, provideLanguage: params => coreOperation(providers.provide, params), resolveLanguageCompletion: params => coreOperation(providers.resolveCompletion, params), resolveLanguageCodeAction: params => coreOperation(providers.resolveAction, params), cancelLanguageProvider: providers.cancel, releaseLanguageProviderRefusal: providers.releaseRefusal, languageProviderPending: providers.pending, updateConfiguration: next => { configuration.update(next); workspaceEditConfigurationIdentity = {}; providers.documentChanged(); fileWatchers.refresh(); }, commandSnapshot, assertOwner,
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
        for (const base of [...editors.values(),...explicitEditors.values()]) {
          const state = editorStates.get(base);
          if (state?.overlay?.owner === owner) state.overlay = undefined;
          state?.presentation.retire(owner);
        }
        callers.retire(owner);
        editorTabs.disposeOwner(owner);
        decorations.disposeOwner(owner);
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
        if (!scopedEditors.has(base)) scopedEditors.set(base, editorFor(source.id, source.document, owner, base));
        return scopedEditors.get(base);
      }
      const services = documentServices.forOwner(owner, scopedEditor);
      // The native document objects are shared; only request-producing editor handles are scoped.
      const facade = supported('vscode', {
        ...api,
        languages: supported('languages', { ...diagnostics.forOwner(owner), ...(sessionOptions.languageProviders ? providers.forOwner(owner) : {}),
          ...(sessionOptions.documentLanguage ? {setTextDocumentLanguage:(document,language)=>documentServices.setTextDocumentLanguage(owner,document,language)} : {}) }),
        window: supported('window', {
          tabGroups:editorTabs.forOwner(owner),
          ...createPrompts(request, sessionOptions.session, owner, promptBudget),
          showTextDocument: services.showTextDocument,
          ...surfaces.forExtension(owner),
          ...decorations.forOwner(owner),
          get activeTextEditor() { return scopedEditor(active); },
          get visibleTextEditors() { return visibleEditors.map(scopedEditor); },
          onDidChangeActiveTextEditor: event(owner, activeChanged.event, scopedEditor),
          onDidChangeTextEditorSelection: event(owner,selectionChanged.event,value=>({...value,textEditor:scopedEditor(value.textEditor)})),
          onDidChangeTextEditorOptions: event(owner,optionsChanged.event,value=>({...value,textEditor:scopedEditor(value.textEditor)})),
          onDidChangeTextEditorVisibleRanges: event(owner,rangesChanged.event,value=>({...value,textEditor:scopedEditor(value.textEditor)})),
          onDidChangeVisibleTextEditors: event(owner,visibleChanged.event,value=>value.map(scopedEditor)),
          onDidChangeTextEditorViewColumn: event(owner,columnChanged.event,value=>({...value,textEditor:scopedEditor(value.textEditor)})),
          showInformationMessage: (text, ...items) => messageFor(owner, text, ...items),
          showWarningMessage: (text, ...items) => messageFor(owner, text, ...items),
          showErrorMessage: (text, ...items) => messageFor(owner, text, ...items),
        }),
        workspace: workspace({
          onDidCreateFiles:event(owner,fileOperations.api.onDidCreateFiles),
          onDidRenameFiles:event(owner,fileOperations.api.onDidRenameFiles),
          onDidDeleteFiles:event(owner,fileOperations.api.onDidDeleteFiles),
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
      providers.documentChanged();
      fileWatchers.refresh();
    },
  };
}
module.exports = { createApi, supported };
