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
const { createDocumentServices, NATIVE_COMMANDS } = require('./document-services.cjs');
const { createMementos } = require('./memento.cjs');
const filesystem = require('./filesystem.cjs');
const { createWorkspaceScope } = require('./workspace-scope.cjs');
const { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument } = types;

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
  const documents = new Map(), editors = new Map(), commands = new Map();
  const changed = new EventEmitter(), opened = new EventEmitter(), closed = new EventEmitter();
  const activeChanged = new EventEmitter(), saved = new EventEmitter();
  const configuration = createConfiguration();
  let active, workspaceRoot, activation, generation = -1;
  const facades = new Map(), owned = new Map();
  const callers = createCallerContext(owner => { assertActiveOwner(owner); sessionOptions.assertOwner?.(owner); });
  const mementos = createMementos(request, sessionOptions.session, sessionOptions.extensionState);
  const documentServices = createDocumentServices(request, sessionOptions, {
    generation: () => generation, root: () => workspaceRoot,
    document: id => documents.get(id), editor: id => active?.document._snapshot.id === id ? active : undefined,
  });
  const reserved = new Set(sessionOptions.reservedCommands || []);
  let registrationCount = 0, commandCalls = 0;
  const promptBudget = { pending: 0, bytes: 0 };
  const fileSystem = filesystem.createFileSystem({ assertOwner, track, bindProvider: callers.bindProvider, invokeProviderResult: callers.invokeProviderResult, bindProviderResult: callers.bindProviderResult });
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
  function request(method, params) {
    try { assertOwner(params.owner); return sendRequest(method, params); }
    catch (error) { return Promise.reject(error); }
  }
  const providers = createProviders({ session: sessionOptions.session, notify, track, bindProvider: callers.bindProvider, bindProviderResult: callers.bindProviderResult, invokeProviderResult: callers.invokeProviderResult, document: id => documents.get(id), diagnostics: (document,range) => diagnostics.currentDiagnostics(document,range) });
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
  function sync(state) {
    // A newer notification can arrive before an edit promise callback runs.
    if (state.generation <= generation) return;
    generation = state.generation;
    const changes = [];
    const present = new Set(state.documents.map(snapshot => snapshot.id));
    for (const snapshot of state.documents) {
      let document = documents.get(snapshot.id);
      const previous = document?.getText();
      const previousVersion = document?.version;
      const previousSaved = document?._snapshot.savedGeneration || 0;
      const previousEnd = typeof snapshot.text === 'string' && document?.positionAt(previous.length);
      if (!document) {
        document = new TextDocument(snapshot);
        documents.set(snapshot.id, document);
        changes.push(() => opened.fire(document));
      } else {
        document._update(snapshot);
        if (snapshot.version !== previousVersion && typeof snapshot.text === 'string') {
          changes.push(() => changed.fire({ document, contentChanges: [{
            range: new Range(new Position(0, 0), previousEnd),
            rangeOffset: 0, rangeLength: previous.length, text: document.getText(),
          }] }));
        }
        if (Number.isSafeInteger(snapshot.savedGeneration) && snapshot.savedGeneration > previousSaved) changes.push(() => saved.fire(document));
      }
      if (!editors.has(snapshot.id)) editors.set(snapshot.id, editorFor(snapshot.id, document));
    }
    for (const [id, document] of documents) {
      if (!present.has(id)) {
        document._snapshot = { ...document._snapshot, isClosed: true };
        documents.delete(id); editors.delete(id);
        changes.push(() => closed.fire(document));
      }
    }
    const previousActive = active;
    active = editors.get(state.active);
    if (active) active._selections = state.selections.map(s => new Selection(s.anchor, s.active));
    providers.documentChanged();
    diagnostics.documentChanged();
    // All mirrors must reflect the new state before any extension callback runs.
    for (const fire of changes) fire();
    if (active !== previousActive) activeChanged.fire(active);
  }
  function editorFor(id, document, owner = '') {
    return supported('TextEditor', {
      document, _selections: [new Selection(0, 0, 0, 0)],
      get selection() { return (owner ? (editors.get(id)?._selections || this._selections) : this._selections)[0]; },
      get selections() { return (owner ? (editors.get(id)?._selections || this._selections) : this._selections).slice(); },
      edit(callback, options) {
        try { assertOwner(owner); } catch (error) { return Promise.reject(error); }
        if (options && (options.undoStopBefore === false || options.undoStopAfter === false)) {
          return Promise.reject(new Error('VSCLI does not yet support grouped extension undo stops'));
        }
        const version = document.version;
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
  }
  const api = supported('vscode', {
    ...types, ...providerTypes, ...diagnosticTypes, ...surfaces.types,
    FileSystemError: filesystem.FileSystemError, FileType: filesystem.FileType,
    FilePermission: filesystem.FilePermission, FileChangeType: filesystem.FileChangeType,
    version: '1.95.0',
    ViewColumn: Object.freeze({ Active: -1, Beside: -2, One: 1, Two: 2, Three: 3, Four: 4, Five: 5, Six: 6, Seven: 7, Eight: 8, Nine: 9 }),
    ExtensionMode: Object.freeze({ Production: 1, Development: 2, Test: 3 }),
    window: supported('window', {
      ...createPrompts(request, sessionOptions.session, '', promptBudget),
      ...surfaces.forExtension(''),
      get activeTextEditor() { return active; },
      get visibleTextEditors() { return active ? [active] : []; },
      onDidChangeActiveTextEditor: activeChanged.event,
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
    api, sync, providerSnapshot: providers.snapshot, provideLanguage: params => coreOperation(providers.provide, params), resolveLanguageCompletion: params => coreOperation(providers.resolveCompletion, params), resolveLanguageCodeAction: params => coreOperation(providers.resolveAction, params), cancelLanguageProvider: providers.cancel, updateConfiguration: configuration.update, commandSnapshot, assertOwner,
    setActivation(value) { activation = value; },
    // Private runtime hooks, intentionally absent from the vscode facade.
    runWithOwner: callers.run, currentCallerOwner: callers.currentOwner,
    contextForExtension(owner) { return { ...mementos.forOwner(owner), extension: activation?.extension(owner) }; },
    mergeExtensionState: mementos.merge,
    configureSurfaces: surfaces.configure, treeChildren: params => coreOperation(surfaces.treeChildren, params),
    surfaceAction: surfaces.action, treeEvent: surfaces.treeEvent,
    disposeOwner(owner) {
      return callers.unowned(() => {
        callers.retire(owner);
        if (sessionOptions.languageProviders) providers.disposeOwner(owner);
        diagnostics.disposeOwner(owner);
        for (const disposable of [...(owned.get(owner) || [])]) disposable.dispose();
        owned.delete(owner); facades.delete(owner);
      });
    },
    forExtension(owner) {
      if (facades.has(owner)) return facades.get(owner);
      const scopedEditors = new WeakMap();
      function scopedEditor(base) {
        if (!base) return undefined;
        if (!scopedEditors.has(base.document)) scopedEditors.set(base.document, editorFor(base.document._snapshot.id, base.document, owner));
        return scopedEditors.get(base.document);
      }
      const services = documentServices.forOwner(owner, scopedEditor);
      // The native document objects are shared; only request-producing editor handles are scoped.
      const facade = supported('vscode', {
        ...api,
        languages: supported('languages', { ...diagnostics.forOwner(owner), ...(sessionOptions.languageProviders ? providers.forOwner(owner) : {}) }),
        window: supported('window', {
          ...createPrompts(request, sessionOptions.session, owner, promptBudget),
          showTextDocument: services.showTextDocument,
          ...surfaces.forExtension(owner),
          get activeTextEditor() { return scopedEditor(active); },
          get visibleTextEditors() { return active ? [scopedEditor(active)] : []; },
          onDidChangeActiveTextEditor: event(owner, activeChanged.event, scopedEditor),
          showInformationMessage: (text, ...items) => messageFor(owner, text, ...items),
          showWarningMessage: (text, ...items) => messageFor(owner, text, ...items),
          showErrorMessage: (text, ...items) => messageFor(owner, text, ...items),
        }),
        workspace: workspace({
          ...fileSystem.forOwner(owner),
          openTextDocument: services.openTextDocument,
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
    },
  };
}
module.exports = { createApi, supported };
