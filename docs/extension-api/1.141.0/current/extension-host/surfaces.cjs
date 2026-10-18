'use strict';
const { Disposable, EventEmitter, Uri } = require('./api-types.cjs');
const limits = Object.freeze({ channels: 32, status: 64, views: 8, children: 256, handles: 1024, updateBytes: 65536 });
function text(value, max, name, empty = true) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > max || (!empty && !value)) throw new TypeError(`Invalid ${name}`);
  return value;
}
class ThemeColor { constructor(id) { this.id = text(id, 256, 'theme color', false); } }
class TreeItem {
  constructor(label, collapsibleState = 0) {
    if (label instanceof Uri) this.resourceUri = label;
    else this.label = label;
    this.collapsibleState = collapsibleState;
  }
}
const enums = Object.freeze({ StatusBarAlignment: Object.freeze({ Left: 1, Right: 2 }),
  TreeItemCollapsibleState: Object.freeze({ None: 0, Collapsed: 1, Expanded: 2 }), ThemeColor, TreeItem });
function createSurfaces(notify, session, track, execute, assertOwner = () => {}) {
  const channels = new Map(), statuses = new Map(), views = new Map(), declared = new Map();
  let sequence = 0, treeCallbackPending = false;
  function send(owner, id, op, data = {}) {
    if (!op.endsWith('Dispose')) assertOwner(owner);
    notify('nativeSurface', { session, owner, id, op, ...data });
  }
  function own(owner, close) { return track(owner, new Disposable(close)); }
  function command(value) {
    if (value === undefined) return undefined;
    if (typeof value === 'string') return { command: text(value, 1024, 'command', false), arguments: [] };
    if (!value || typeof value !== 'object') throw new TypeError('Invalid surface command');
    text(value.command, 1024, 'command', false);
    if (value.arguments !== undefined && (!Array.isArray(value.arguments) || value.arguments.length > 32)) throw new TypeError('Invalid command arguments');
    // Keep argument object identity, cycles and extension classes inside the host.
    return { command: value.command, arguments: value.arguments || [] };
  }
  function createOutputChannel(owner, name, language) {
    assertOwner(owner);
    text(name, 256, 'output name', false);
    if (language !== undefined) text(language, 128, 'output language');
    if (channels.size >= limits.channels) throw new Error('Output channel limit reached');
    const id = `output-${++sequence}`, state = { owner, id, name, disposed: false };
    channels.set(id, state);
    send(owner, id, 'outputCreate', { name });
    const disposable = own(owner, () => {
      if (state.disposed) return;
      state.disposed = true; channels.delete(id); send(owner, id, 'outputDispose');
    });
    function update(op, value) {
      if (state.disposed) return;
      text(value, limits.updateBytes, 'output update');
      send(owner, id, op, { text: value });
    }
    return { name,
      append(value) { if (value) update('outputAppend', value); },
      appendLine(value) { text(value, limits.updateBytes - 1, 'output line'); update('outputAppend', `${value}\n`); },
      replace(value) { update('outputReplace', value); },
      clear() { if (!state.disposed) send(owner, id, 'outputClear'); },
      show(column, preserveFocus) {
        if (typeof column !== 'number') preserveFocus = column;
        if (preserveFocus !== undefined && typeof preserveFocus !== 'boolean') throw new TypeError('Invalid preserveFocus');
        if (!state.disposed) send(owner, id, 'outputShow', { preserveFocus: preserveFocus === true });
      },
      hide() { if (!state.disposed) send(owner, id, 'outputHide'); },
      dispose() { disposable.dispose(); },
    };
  }
  function createStatusBarItem(owner, id, alignment, priority) {
    assertOwner(owner);
    if (typeof id !== 'string') { priority = alignment; alignment = id; id = owner; }
    text(id, 256, 'status id', false);
    alignment ??= 1;
    if (![1, 2].includes(alignment) || (priority !== undefined && !Number.isFinite(priority))) throw new TypeError('Invalid status position');
    if (statuses.size >= limits.status) throw new Error('Status item limit reached');
    const key = `status-${++sequence}`;
    const state = { owner, key, id, alignment, priority, name: undefined, text: '', tooltip: undefined,
      color: undefined, backgroundColor: undefined, command: undefined, visible: false, disposed: false, generation: 0 };
    statuses.set(key, state);
    function publish() {
      if (!state.disposed) send(owner, key, 'status', { generation: ++state.generation, name: state.name || id, text: state.text, tooltip: state.tooltip || '',
        alignment, priority: priority || 0, visible: state.visible, hasCommand: !!state.command,
        color: state.color instanceof ThemeColor ? state.color.id : state.color || '', background: state.backgroundColor?.id || '' });
    }
    const disposable = own(owner, () => { state.disposed = true; statuses.delete(key); send(owner, key, 'statusDispose'); });
    const item = { id, alignment, priority,
      show() { state.visible = true; publish(); }, hide() { state.visible = false; publish(); }, dispose() { disposable.dispose(); } };
    const validators = {
      name: value => value === undefined ? value : text(value, 256, 'status name'),
      text: value => text(value, 1024, 'status text'),
      tooltip: value => value === undefined ? value : text(value, 4096, 'status tooltip (plain text only)'),
      command,
      color: value => {
        if (value === undefined || value instanceof ThemeColor) return value;
        if (typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value)) return value;
        throw new TypeError('Status color must be #RRGGBB or ThemeColor');
      },
      backgroundColor: value => {
        if (value === undefined || value instanceof ThemeColor && ['statusBarItem.errorBackground', 'statusBarItem.warningBackground'].includes(value.id)) return value;
        throw new TypeError('Unsupported status background color');
      },
    };
    for (const [property, validate] of Object.entries(validators)) Object.defineProperty(item, property, {
      enumerable: true, get: () => state[property], set(value) { const checked = validate(value); if (!state.disposed) { state[property] = checked; publish(); } },
    });
    publish();
    return item;
  }
  function createTreeView(owner, id, options) {
    assertOwner(owner);
    text(id, 256, 'tree view id', false);
    if (!declared.get(owner)?.has(id)) throw new Error(`Tree view is not contributed by ${owner}: ${id}`);
    if (views.has(id)) throw new Error(`Tree view is already registered: ${id}`);
    if (views.size >= limits.views) throw new Error('Tree view limit reached');
    if (!options || !options.treeDataProvider || typeof options.treeDataProvider.getChildren !== 'function' || typeof options.treeDataProvider.getTreeItem !== 'function') throw new TypeError('Invalid tree provider');
    if (options.canSelectMany || options.dragAndDropController || options.manageCheckboxStateManually) throw new Error('Native trees currently support single selection without drag/drop/checkboxes');
    const changed = new EventEmitter(), expanded = new EventEmitter(), collapsed = new EventEmitter(), selected = new EventEmitter();
    const state = { owner, id, provider: options.treeDataProvider, generation: ++sequence, handles: new Map(), elements: new Map(), identifiers: new Map(),
      next: 0, title: declared.get(owner).get(id), description: '', message: '', visible: false, selection: [], disposed: false };
    views.set(id, state);
    function publish() { send(owner, id, 'tree', { title: state.title, description: state.description, message: state.message, generation: state.generation }); }
    function invalidate() {
      if (state.disposed) return;
      state.generation = ++sequence; state.handles.clear(); state.elements.clear(); state.identifiers.clear(); state.selection = []; publish();
    }
    let listener;
    try {
      listener = state.provider.onDidChangeTreeData?.(() => invalidate());
    } catch (error) {
      views.delete(id); state.disposed = true;
      throw error;
    }
    const disposable = own(owner, () => {
      state.disposed = true; listener?.dispose(); views.delete(id); state.handles.clear(); state.elements.clear();
      for (const event of [changed, expanded, collapsed, selected]) event.dispose();
      send(owner, id, 'treeDispose');
    });
    const view = { dispose: () => disposable.dispose(), get visible() { return state.visible; }, get selection() { return state.selection.slice(); },
      onDidChangeVisibility: changed.event, onDidExpandElement: expanded.event, onDidCollapseElement: collapsed.event, onDidChangeSelection: selected.event,
      reveal() { return Promise.reject(new Error('Native tree reveal/getParent navigation is not implemented')); } };
    state.events = { changed, expanded, collapsed, selected };
    for (const key of ['title', 'description', 'message']) Object.defineProperty(view, key, {
      enumerable: true, get: () => state[key], set(value) { const checked = text(value, key === 'message' ? 4096 : 256, `tree ${key}`); if (!state.disposed) { state[key] = checked; publish(); } },
    });
    publish();
    return view;
  }
  function getView(params) {
    assertOwner(params.owner);
    const view = views.get(params.id);
    if (params.session !== session || !view || view.owner !== params.owner || view.disposed || params.generation !== view.generation) throw new Error('Stale native tree request');
    return view;
  }
  async function treeChildren(params) {
    const view = getView(params);
    if (treeCallbackPending) throw new Error('A tree provider callback is still running; wait for it to settle or restart the extension host');
    // A caller timeout, refresh or disposal cannot cancel extension promises.
    // Retain one shared slot until the actual callback chain has settled.
    treeCallbackPending = true;
    try { return await loadTreeChildren(params, view); }
    finally { treeCallbackPending = false; }
  }
  async function loadTreeChildren(params, view) {
    const generation = view.generation;
    function current() {
      assertOwner(view.owner);
      if (views.get(view.id) !== view || view.disposed || view.generation !== generation) throw new Error('Tree changed while children were loading');
    }
    const parent = params.node === undefined ? undefined : view.handles.get(params.node);
    if (params.node !== undefined && !parent) throw new Error('Unknown native tree node');
    current();
    const children = await view.provider.getChildren(parent?.element) ?? [];
    current();
    if (!Array.isArray(children) || children.length > limits.children) throw new Error('Native tree children exceed 256 items');
    const prepared = [], seen = new Set();
    let bytes = 0;
    for (const element of children) {
      current();
      const item = await view.provider.getTreeItem(element);
      current();
      if (!item || ![undefined, 0, 1, 2].includes(item.collapsibleState) || item.checkboxState !== undefined) throw new Error('Unsupported native tree item');
      let label = typeof item.label === 'object' ? item.label?.label : item.label;
      if (!label && item.resourceUri instanceof Uri) label = require('node:path').basename(item.resourceUri.fsPath);
      text(label, 1024, 'tree label', false);
      const description = item.description === true ? item.resourceUri?.fsPath || '' : item.description || '';
      text(description, 2048, 'tree description');
      const tooltip = item.tooltip === undefined ? '' : text(item.tooltip, 4096, 'tree tooltip (plain text only)');
      const explicit = item.id === undefined ? undefined : text(item.id, 256, 'tree item id', false);
      const action = command(item.command);
      bytes += Buffer.byteLength(label + description + tooltip);
      if (bytes > 128 * 1024) throw new Error('Native tree reply exceeds 128 KiB');
      if (seen.has(element) || explicit && prepared.some(p => p.explicit === explicit)) throw new Error('Duplicate native tree item');
      seen.add(element);
      prepared.push({ element, explicit, action, label, description, tooltip, collapsible: item.collapsibleState || 0 });
    }
    current();
    // Never overwrite an old handle: native validation can reject a new batch,
    // in which case the previously rendered actions must retain their meaning.
    if (view.handles.size + prepared.length > limits.handles) throw new Error('Native tree handle budget exceeded');
    for (const item of prepared) if (item.explicit && view.identifiers.has(item.explicit) && view.identifiers.get(item.explicit) !== item.element) throw new Error('Duplicate tree item identifier');
    for (const item of prepared) {
      const previous = view.handles.get(view.elements.get(item.element));
      if (previous && previous.parent !== params.node) throw new Error('Tree element appears under multiple parents');
      let ancestor = parent;
      while (ancestor) {
        if (ancestor.element === item.element) throw new Error('Cyclic tree element');
        ancestor = view.handles.get(ancestor.parent);
      }
    }
    const items = prepared.map(item => {
      const key = `node-${++view.next}`;
      item.parent = params.node;
      view.elements.set(item.element, key); view.handles.set(key, item);
      if (item.explicit) view.identifiers.set(item.explicit, item.element);
      return { node: key, label: item.label, description: item.description, tooltip: item.tooltip, collapsible: item.collapsible, hasCommand: !!item.action };
    });
    return { session, owner: view.owner, id: view.id, generation, items };
  }
  async function action(params) {
    assertOwner(params.owner);
    if (params.session !== session) throw new Error('Stale surface action session');
    let entry;
    if (params.kind === 'status') {
      const item = statuses.get(params.id);
      if (!item || item.owner !== params.owner || item.disposed || !item.visible || params.generation !== item.generation) throw new Error('Stale status action');
      entry = item.command;
    } else {
      const view = getView(params), item = view.handles.get(params.node);
      if (!item) throw new Error('Unknown tree action node');
      view.selection = [item.element]; view.events.selected.fire({ selection: view.selection.slice() });
      entry = item.action;
    }
    if (!entry) throw new Error('Surface has no command');
    await execute(params.owner, entry.command, entry.arguments);
    return null;
  }
  function treeEvent(params) {
    const view = getView(params);
    if (params.event === 'visibility') { view.visible = params.visible === true; view.events.changed.fire({ visible: view.visible }); return; }
    const item = view.handles.get(params.node);
    if (!item) throw new Error('Unknown tree event node');
    if (params.event === 'expand') view.events.expanded.fire({ element: item.element });
    else if (params.event === 'collapse') view.events.collapsed.fire({ element: item.element });
    else throw new Error('Unknown native tree event');
  }
  return {
    types: enums, treeChildren, action, treeEvent,
    configure(packages) {
      const staged = new Map();
      for (const package_ of packages) {
        const entries = new Map();
        for (const group of Object.values(package_.manifest.contributes?.views || {})) {
          if (!Array.isArray(group)) throw new Error('Invalid contributed views');
          for (const view of group) {
            if (typeof view?.type === 'string' && view.type !== 'tree') continue;
            if (entries.size >= limits.views) throw new Error('Contributed tree view limit reached');
            text(view.id, 256, 'contributed view id', false); text(view.name, 256, 'contributed view name', false);
            if (entries.has(view.id)) throw new Error('Duplicate contributed view ID');
            entries.set(view.id, view.name);
          }
        }
        const previous = declared.get(package_.id);
        if (previous && (previous.size !== entries.size || [...previous].some(([id, name]) => entries.get(id) !== name))) throw new Error('Existing surface declaration changed during admission');
        staged.set(package_.id, entries);
      }
      for (const [owner, entries] of staged) declared.set(owner, entries);
    },
    forExtension(owner) { return {
      createOutputChannel: (name, language) => createOutputChannel(owner, name, language),
      createStatusBarItem: (...args) => createStatusBarItem(owner, ...args),
      createTreeView: (id, options) => createTreeView(owner, id, options),
      registerTreeDataProvider: (id, provider) => createTreeView(owner, id, { treeDataProvider: provider }),
    }; },
  };
}
module.exports = { createSurfaces, ...enums, limits };
