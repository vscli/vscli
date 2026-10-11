'use strict';
const { AsyncLocalStorage } = require('node:async_hooks');
const { Uri, Disposable } = require('./api-types.cjs');
const { supported } = require('./api.cjs');

// Only selected immutable descriptors enter this registry. Exports stay in this
// process; activation replies must never serialize another package's API object.
function createActivation(items, hooks) {
  const entries = new Map(), lineage = new AsyncLocalStorage(), waiting = new Map();
  const order = [], listeners = new Set();
  let calls = 0;
  function validate(proposed) {
    if (proposed.size > 8) throw new Error('Extension cohort exceeds eight selected packages');
    const done = new Set(), stack = [];
    function visit(id) {
      if (done.has(id)) return;
      if (stack.includes(id)) throw new Error(`Extension dependency cycle: ${[...stack, id].join(' -> ')}`);
      const entry = proposed.get(id);
      if (!entry) throw new Error(`Required extension is not selected: ${id}`);
      stack.push(id);
      for (const dependency of entry.dependencies) visit(dependency);
      stack.pop(); done.add(id);
    }
    for (const id of [...proposed.keys()].sort()) visit(id);
  }
  function add(values) {
    const proposed = new Map(entries);
    for (const item of values) {
      if (proposed.has(item.id)) throw new Error(`Extension snapshot already selected: ${item.id}`);
      const dependencies = item.manifest.extensionDependencies ?? [];
      if (!Array.isArray(dependencies) || dependencies.length > 8 || dependencies.some(id => typeof id !== 'string' || !/^[\w-]{1,100}\.[\w-]{1,100}$/.test(id))) {
        throw new Error(`${item.id}: invalid extension dependencies`);
      }
      proposed.set(item.id, { item, dependencies: [...new Set(dependencies.map(id => id.toLowerCase()))].sort(), state: 'dormant', value: undefined, error: undefined, promise: undefined, facade: undefined });
    }
    validate(proposed);
    for (const [id, entry] of proposed) entries.set(id, entry);
    if (values.length) for (const listener of [...listeners]) {
      try { listener(); } catch (error) { hooks.listenerError?.(error); }
    }
  }
  function hasPath(from, target, seen = new Set()) {
    if (from === target) return true;
    if (seen.has(from)) return false;
    seen.add(from);
    for (const next of waiting.get(from)?.keys() ?? []) if (hasPath(next, target, seen)) return true;
    return false;
  }
  function edge(from, target) {
    if (!from || entries.get(from)?.state !== 'activating') return () => {};
    if (hasPath(target, from)) throw new Error(`Extension activation wait cycle: ${from} -> ${target}`);
    if (!waiting.has(from)) waiting.set(from, new Map());
    const edges = waiting.get(from);
    edges.set(target, (edges.get(target) ?? 0) + 1);
    return () => {
      const count = edges.get(target) - 1;
      if (count) edges.set(target, count); else edges.delete(target);
      if (!edges.size) waiting.delete(from);
    };
  }
  async function activate(id) {
    id = String(id).toLowerCase();
    const entry = entries.get(id);
    if (!entry) throw new Error(`Extension is not selected: ${id}`);
    if (entry.state === 'active') return entry.value;
    if (entry.state === 'failed') throw entry.error;
    if (calls >= 64) throw new Error('Extension activation call limit exceeded');
    const release = edge(lineage.getStore(), id);
    calls++;
    try {
      if (!entry.promise) {
        entry.state = 'activating';
        // Deferring the callback publishes the shared promise before dependency
        // or API re-entry can request this same package again.
        entry.promise = Promise.resolve().then(() => lineage.run(id, async () => {
          try {
            for (const dependency of entry.dependencies) await activate(dependency);
            entry.value = await hooks.activate(entry.item);
            entry.state = 'active'; order.push(id);
            return entry.value;
          } catch (error) {
            entry.state = 'failed';
            entry.error = error instanceof Error ? error : new Error(String(error));
            try { hooks.failed?.(entry.item); } catch (cleanup) { hooks.listenerError?.(cleanup); }
            throw entry.error;
          } finally { hooks.changed?.(); }
        }));
      }
      return await entry.promise;
    } finally { release(); calls--; }
  }
  function extension(id) {
    const entry = entries.get(id.toLowerCase());
    if (!entry) return undefined;
    if (!entry.facade) entry.facade = supported('Extension', {
      id: entry.item.id,
      extensionUri: Uri.file(entry.item.folder), extensionPath: entry.item.folder,
      extensionKind: 1,
      packageJSON: JSON.parse(JSON.stringify(entry.item.manifest)),
      get isActive() { return entry.state === 'active'; },
      get exports() { return entry.value; },
      activate: () => activate(entry.item.id),
    });
    return entry.facade;
  }
  function facade() {
    return supported('extensions', {
      getExtension(id, includeFromDifferentExtensionHosts) {
        if (typeof id !== 'string' || id.length > 201) throw new TypeError('Extension ID must be a bounded string');
        if (includeFromDifferentExtensionHosts) throw new Error('Cross-host proposed extension lookup is unsupported');
        return extension(id);
      },
      get all() { return [...entries.keys()].sort().map(extension); },
      onDidChange(listener, thisArg, disposables) {
        if (typeof listener !== 'function' || listeners.size >= 4096) throw new Error('Invalid extension registry listener or limit exceeded');
        const call = () => listener.call(thisArg);
        listeners.add(call);
        const disposable = new Disposable(() => listeners.delete(call));
        disposables?.push(disposable);
        return disposable;
      },
    });
  }
  async function shutdown() {
    const errors = [];
    for (const id of [...order].reverse()) {
      const entry = entries.get(id);
      try { await hooks.deactivate?.(entry.item); } catch (error) { errors.push(error); }
      finally { entry.state = 'disposed'; entry.value = undefined; }
    }
    if (errors.length) throw new Error(`Extension shutdown failed: ${String(errors[0])}`);
  }
  add(items);
  return { add, activate, facade, extension, shutdown,
    accepts: owner => ['active', 'activating'].includes(entries.get(owner)?.state),
    statuses: () => [...entries].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([id, entry]) => ({ id, state: entry.state, error: entry.error?.message?.slice(0, 2048) })) };
}
module.exports = { createActivation };
