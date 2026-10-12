'use strict';

// Runtime-private caller authority. It is separate from host command-origin
// ALS: completing an invocation ends correlation, not extension ownership.
const { AsyncLocalStorage } = require('node:async_hooks');
const storage = new AsyncLocalStorage();
const callbackMethods = Object.freeze([
  'prepareRename', 'provideRenameEdits',
  'provideWorkspaceSymbols', 'resolveWorkspaceSymbol',
  'provideCodeActions', 'resolveCodeAction', 'provideCompletionItems', 'resolveCompletionItem',
  'provideHover', 'provideDefinition', 'provideReferences', 'provideDocumentFormattingEdits',
  'provideImplementation', 'provideTypeDefinition', 'provideDeclaration',
  'provideDocumentSymbols', 'provideSignatureHelp', 'getChildren', 'getTreeItem',
  'onDidChangeTreeData', 'watch', 'stat', 'readDirectory', 'createDirectory',
  'readFile', 'writeFile', 'delete', 'rename', 'copy', 'open', 'close', 'read', 'write',
  'onDidChangeFile', 'provideTextDocumentContent', 'onDidChange',
]);

// Value-object constructors remain shared and unchanged. Only listeners created
// under a trusted runtime scope capture it. No scope => no inferred owner.
function bindCurrentListener(listener) {
  const scope = storage.getStore();
  return scope && typeof listener === 'function' ? scope.bindListener(listener) : listener;
}
function createCallerContext(assertOwner) {
  if (typeof assertOwner !== 'function') throw new TypeError('Caller context requires owner validation');
  const identity = {}, leases = new Map(), providerScopes = new WeakMap();
  function assertCurrent(scope) {
    if (!scope || scope.identity !== identity || leases.get(scope.owner) !== scope) {
      throw new Error('Extension caller ownership is missing or retired');
    }
    assertOwner(scope.owner);
  }
  function assertInherited() {
    const inherited = storage.getStore();
    if (inherited?.identity === identity) assertCurrent(inherited);
  }
  function capture(owner) {
    if (typeof owner !== 'string' || !owner) throw new Error('Extension caller ownership requires an owner');
    // An old timer cannot acquire a renewed lease through its cached facade.
    // Live cross-owner calls remain allowed; their new callback belongs to the
    // explicitly selected facade, not to the invoking extension.
    assertInherited();
    assertOwner(owner);
    let scope = leases.get(owner);
    if (!scope) {
      scope = { identity, owner, bindListener: callback => bindListener(scope, callback) };
      leases.set(owner, scope);
    }
    return scope;
  }
  function invoke(scope, callback, receiver, args) {
    // Reject a retired descendant before replacing it with a live delegated lease.
    assertInherited();
    if (scope) assertCurrent(scope);
    return storage.run(scope, () => Reflect.apply(callback, receiver, args));
  }
  function bindScope(scope, callback) {
    if (typeof callback !== 'function') throw new TypeError('Caller callback requires a function');
    if (scope) assertCurrent(scope);
    return function (...args) { return invoke(scope, callback, this, args); };
  }
  function bindListener(scope, callback) {
    if (typeof callback !== 'function') return callback;
    if (scope) assertCurrent(scope);
    return function (...args) {
      // A retired scheduled cancellation/event callback is inert, not an
      // uncaught timer exception. Live extension callback errors still escape
      // to the original event/error policy.
      if (scope) { try { assertCurrent(scope); } catch { return undefined; } }
      return storage.run(scope, () => Reflect.apply(callback, this, args));
    };
  }
  function bind(owner, callback) {
    return bindScope(owner ? capture(owner) : undefined, callback);
  }
  function bindEvent(owner, source) {
    const scope = owner ? capture(owner) : undefined;
    return function (listener, thisArg, disposables) {
      if (scope) assertCurrent(scope);
      // Register under the event owner as well: a shared EventEmitter must not
      // capture an unrelated invoking extension as an outer listener scope.
      return invoke(scope, source, this, [bindListener(scope, listener), thisArg, disposables]);
    };
  }
  function bindProvider(owner, provider) {
    if (!provider || !['object', 'function'].includes(typeof provider)) return provider;
    const scope = owner ? capture(owner) : undefined;
    // A shadow adapter avoids Proxy.get invariants for frozen/nonconfigurable
    // providers. Methods/getters always see the ORIGINAL provider as receiver.
    const adapter = Object.create(provider);
    for (const key of callbackMethods) Object.defineProperty(adapter, key, {
      configurable: true,
      get() {
        const value = invoke(scope, Reflect.get, Reflect, [provider, key, provider]);
        return typeof value === 'function'
          ? function (...args) { return invoke(scope, value, provider, args); }
          : value;
      },
    });
    providerScopes.set(adapter, scope);
    return adapter;
  }
  function resultScope(provider) {
    if (!providerScopes.has(provider)) throw new Error('Provider caller lease is missing');
    return providerScopes.get(provider);
  }
  function bindProviderResult(provider, callback) {
    // Capture registration identity now, validate on invocation. Retiring the
    // owner before the queued callback runs must reject rather than reacquire.
    const scope = resultScope(provider);
    return function (...args) {
      const result = invoke(scope, callback, this, args);
      // Extension getters may retire/reacquire their owner during conversion.
      if (scope) assertCurrent(scope);
      return result;
    };
  }
  function invokeProviderResult(provider, callback) {
    // Promise.resolve reads a user Thenable's `then` under its original lease
    // and schedules its assimilation in that scope. Core finalizers are still
    // attached by the caller outside this scope.
    const scope = resultScope(provider);
    return invoke(scope, () => Promise.resolve(callback()).then(value => {
      if (scope) assertCurrent(scope);
      return value;
    }), undefined, []);
  }
  function bindProviderCleanup(provider, callback) {
    const scope = resultScope(provider);
    // Cleanup may run after retirement. Keep its ORIGINAL scope so API calls
    // still reject stale authority, but permit releasing foreign OS resources.
    return function (...args) {
      return storage.run(scope, () => Reflect.apply(callback, this, args));
    };
  }
  return Object.freeze({
    run(owner, call) { return invoke(capture(owner), call, undefined, []); },
    currentOwner() { const scope = storage.getStore(); assertCurrent(scope); return scope.owner; },
    bind, bindEvent, bindProvider, bindProviderResult, bindProviderCleanup, invokeProviderResult, assertInherited,
    unowned(call) { return storage.run(undefined, call); },
    bindAsync(owner, callback) {
      const scope = owner ? capture(owner) : undefined;
      return function (...args) {
        const receiver = this;
        return invoke(scope, () => Promise.resolve(Reflect.apply(callback, receiver, args)), undefined, []);
      };
    },
    retire(owner) { leases.delete(owner); },
  });
}
module.exports = { createCallerContext, bindCurrentListener };
