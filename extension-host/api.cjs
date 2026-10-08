'use strict';
const types = require('./api-types.cjs');
const { createConfiguration } = require('./configuration.cjs');
const { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument } = types;

function supported(name, values) {
  return new Proxy(values, { get(target, key) {
    if (typeof key === 'symbol' || key === 'then' || key in target) return target[key];
    throw new Error(`VSCLI extension API is not implemented: ${name}.${key}`);
  } });
}

function createApi(request, notify) {
  const documents = new Map(), editors = new Map(), commands = new Map();
  const changed = new EventEmitter(), opened = new EventEmitter(), closed = new EventEmitter();
  const activeChanged = new EventEmitter();
  const configuration = createConfiguration();
  let active, workspaceFolder, generation = -1;
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
      const previousEnd = typeof snapshot.text === 'string' && document?.positionAt(previous.length);
      if (!document) {
        document = new TextDocument(snapshot);
        documents.set(snapshot.id, document);
        changes.push(() => opened.fire(document));
      } else {
        document._update(snapshot);
        if (snapshot.version !== previousVersion && previous !== document.getText()) {
          changes.push(() => changed.fire({ document, contentChanges: [{
            range: new Range(new Position(0, 0), previousEnd),
            rangeOffset: 0, rangeLength: previous.length, text: document.getText(),
          }] }));
        }
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
    // All mirrors must reflect the new state before any extension callback runs.
    for (const fire of changes) fire();
    if (active !== previousActive) activeChanged.fire(active);
  }
  function editorFor(id, document) {
    return supported('TextEditor', {
      document, _selections: [new Selection(0, 0, 0, 0)],
      get selection() { return this._selections[0]; },
      get selections() { return this._selections.slice(); },
      edit(callback, options) {
        if (options && (options.undoStopBefore === false || options.undoStopAfter === false)) {
          return Promise.reject(new Error('VSCLI does not yet support grouped extension undo stops'));
        }
        const version = document.version;
        const edits = [];
        let valid = true;
        const add = (range, newText) => {
          if (!valid) throw new Error('TextEditorEdit is only valid during its callback');
          if (typeof newText !== 'string') throw new TypeError('Edit text must be a string');
          edits.push({ range, newText });
        };
        const builder = supported('TextEditorEdit', {
          replace: (range, text) => add(document.validateRange(range), text),
          insert: (position, text) => add(new Range(document.validatePosition(position), document.validatePosition(position)), text),
          delete: range => add(document.validateRange(range), ''),
        });
        try { callback(builder); } catch (error) { return Promise.reject(error); }
        finally { valid = false; }
        // State notifications are processed before the native acknowledgement.
        return request('edit', { document: id, version, edits }).then(result => result.applied);
      },
    });
  }
  const api = supported('vscode', {
    ...types,
    version: '1.95.0',
    EndOfLine: Object.freeze({ LF: 1, CRLF: 2 }),
    ExtensionMode: Object.freeze({ Production: 1, Development: 2, Test: 3 }),
    window: supported('window', {
      get activeTextEditor() { return active; },
      get visibleTextEditors() { return active ? [active] : []; },
      onDidChangeActiveTextEditor: activeChanged.event,
      showInformationMessage: message,
      showWarningMessage: message,
      showErrorMessage: message,
    }),
    workspace: supported('workspace', {
      get textDocuments() { return [...documents.values()]; },
      get workspaceFolders() { return workspaceFolder ? [workspaceFolder] : undefined; },
      get rootPath() { return workspaceFolder?.uri.fsPath; },
      onDidChangeTextDocument: changed.event,
      onDidOpenTextDocument: opened.event,
      onDidCloseTextDocument: closed.event,
      getConfiguration: configuration.get,
      onDidChangeConfiguration: configuration.onDidChange,
    }),
    commands: supported('commands', {
      registerCommand(id, callback, thisArg) {
        if (typeof id !== 'string' || !id || commands.has(id)) throw new Error(`Invalid or duplicate command: ${id}`);
        if (commands.size >= 1024) throw new Error('Extension command limit reached');
        commands.set(id, (...args) => callback.apply(thisArg, args));
        notify('commands', [...commands.keys()]);
        return new Disposable(() => { commands.delete(id); notify('commands', [...commands.keys()]); });
      },
      async executeCommand(id, ...args) {
        const command = commands.get(id);
        if (!command) throw new Error(`VSCLI cannot execute unregistered extension command: ${id}`);
        return command(...args);
      },
      async getCommands() { return [...commands.keys()]; },
    }),
  });
  function message(text, ...items) {
    if (items.length) return Promise.reject(new Error('VSCLI extension message choices are not implemented'));
    notify('message', String(text));
    return Promise.resolve(undefined);
  }
  return {
    api, sync, updateConfiguration: configuration.update,
    configure(root, schema, layers) {
      workspaceFolder = { uri: Uri.file(root), name: require('node:path').basename(root), index: 0 };
      configuration.initialize(schema, layers);
    },
  };
}
module.exports = { createApi, supported };
