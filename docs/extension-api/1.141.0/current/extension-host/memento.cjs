'use strict';
function cloneValue(value) {
  let count = 0;
  const pending = [[value, 0]];
  while (pending.length) {
    const [item, depth] = pending.pop();
    if (++count > 10000 || depth > 32) throw new Error('Memento value exceeds 10,000 values or 32 nesting levels');
    if (item && typeof item === 'object') {
      if (!Array.isArray(item) && Object.getPrototypeOf(item) !== Object.prototype && Object.getPrototypeOf(item) !== null) throw new Error('Memento values must contain JSON objects');
      for (const key in item) {
        if (!Object.hasOwn(item, key)) continue;
        if (pending.length + count >= 10000) throw new Error('Memento value exceeds 10,000 values');
        pending.push([item[key], depth + 1]);
      }
    } else if (!['string', 'boolean', 'number'].includes(typeof item) && item !== null) throw new Error('Memento values must be JSON values');
    else if (typeof item === 'number' && !Number.isFinite(item)) throw new Error('Memento numbers must be finite');
  }
  const json = JSON.stringify(value);
  if (Buffer.byteLength(json) > 64 * 1024) throw new Error('Memento value exceeds 64 KiB');
  return JSON.parse(json);
}
function createMementos(request, session, initial = {}) {
  const owners = new Map();
  let pendingCount = 0;
  function forOwner(owner) {
    if (owners.has(owner)) return owners.get(owner);
    function scopeState(scope) {
      let confirmed = new Map(Object.entries(initial[owner]?.[scope] || {}));
      const pending = [];
      function view() {
        const values = new Map(confirmed);
        for (const patch of pending) { if (patch.remove) values.delete(patch.key); else values.set(patch.key, patch.value); }
        return values;
      }
      return Object.freeze({
        get(key, fallback) { const values = view(); return values.has(key) ? values.get(key) : fallback; },
        keys() { return [...view().keys()]; },
        update(key, value) {
          try {
            if (typeof key !== 'string' || Buffer.byteLength(key) > 1024) throw new Error('Memento keys must be strings of at most 1 KiB');
            if (pendingCount >= 8) throw new Error('Memento update queue limit reached');
            const remove = value === undefined;
            const snapshot = remove ? undefined : cloneValue(value);
            const proposed = view(); if (remove) proposed.delete(key); else proposed.set(key, snapshot);
            if (proposed.size > 1024 || Buffer.byteLength(JSON.stringify(Object.fromEntries(proposed))) > 256 * 1024) throw new Error('Memento state exceeds 1,024 keys or 256 KiB');
            const patch = { key, value: snapshot, remove };
            pending.push(patch); pendingCount++;
            const finish = () => { pending.splice(pending.indexOf(patch), 1); pendingCount--; };
            // Send in the caller's AsyncLocalStorage context. The native FIFO
            // serializes commits; a background JS drain would inherit the wrong
            // originating command when updates arrive from different commands.
            return request('nativeStateWrite', { session, owner, args: { scope, key, remove, value: remove ? null : snapshot } }).then(result => {
              if (!result || !result.values || typeof result.values !== 'object' || Array.isArray(result.values)) throw new Error('Invalid native Memento acknowledgement');
              confirmed = new Map(Object.entries(result.values));
            }).finally(finish);
          } catch (error) { return Promise.reject(error); }
        },
        ...(scope === 'global' ? { setKeysForSync() { throw new Error('Memento cloud synchronization is not implemented'); } } : {}),
      });
    }
    const context = { globalState: scopeState('global'), workspaceState: scopeState('workspace') };
    owners.set(owner, context);
    return context;
  }
  return { forOwner, merge(states) {
    if (!states || typeof states !== 'object' || Array.isArray(states)) throw new Error('Invalid extension state initialization');
    // Existing live Mementos own their pending overlays; dynamic activation only
    // supplies snapshots for newly admitted package owners.
    for (const [owner, value] of Object.entries(states)) {
      if (!owners.has(owner)) Object.defineProperty(initial, owner, { value, configurable: true, enumerable: true, writable: true });
    }
  } };
}
module.exports = { createMementos };
