'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { Console } = require('node:console');
const { AsyncLocalStorage } = require('node:async_hooks');
const execution = new AsyncLocalStorage(), activationRequest = new AsyncLocalStorage();
async function inExecution(id, owner, call) {
  const origin = { id, owner, active: true };
  try { return await execution.run(origin, call); }
  finally { origin.active = false; }
}
const { createApi, supported } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');
const { createActivation } = require('./activation.cjs');
// Console output from extensions must never corrupt the protocol stream.
global.console = new Console(process.stderr, process.stderr);
const MAX = 16 * 1024 * 1024;
let buffer = Buffer.alloc(0), nextId = 0, initialized = false, ready = false, runtime, activation, session, root, configuration, activating = 0, activationBusy = false, languageProviders = false;
const pending = new Map(), packages = [];
function send(message) {
  const body = Buffer.from(JSON.stringify(message));
  if (body.length > MAX || process.stdout.writableLength + body.length > 64 * 1024 * 1024) {
    throw new Error('Extension protocol output limit exceeded');
  }
  process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
  process.stdout.write(body);
}
function request(method, params) {
  if (method === 'prompt' || ['nativeDocumentOpen', 'nativeDocumentShow', 'nativeCommand', 'nativeStateWrite'].includes(method)) {
    const context = execution.getStore();
    if (context?.active && Number.isSafeInteger(context.id)) params = { ...params, command: context.id, commandOwner: context.owner };
  }
  if (pending.size >= 64) return Promise.reject(new Error('Extension request limit exceeded'));
  const id = `host-${++nextId}`;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Native request timed out: ${method}`)); }, method === 'prompt' ? 360000 : 5000);
    pending.set(id, { resolve, reject, timer });
    try { send({ id, method, params }); }
    catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
  });
}
const originalLoad = Module._load;
Module._load = function(name, parent, isMain) {
  if (name === 'vscode') {
    const owner = [...packages].sort((a, b) => b.folder.length - a.folder.length).find(item => {
      const relative = path.relative(item.folder, parent?.filename || '');
      return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
    });
    if (!owner) throw new Error('Cannot identify the extension requesting vscode');
    return runtime.forExtension(owner.id);
  }
  return originalLoad.call(this, name, parent, isMain);
};
function nodes(value) {
  if (value === null || typeof value !== 'object') return 1;
  return 1 + Object.values(value).reduce((sum, item) => sum + nodes(item), 0);
}
function prepare(inputs, retained = []) {
  if (!Array.isArray(inputs) || !inputs.length || inputs.length > 8) throw new Error('Select between one and eight code extensions');
  let bytes = 0, count = 0, bindings = 0, contributions = 0;
  if (inputs.length + retained.length > 8) throw new Error('Extension cohort exceeds eight selected packages');
  const ids = new Set(retained.map(item => item.id));
  function account(manifest, size) {
    bytes += size; count += nodes(manifest);
    bindings += Array.isArray(manifest.contributes?.keybindings) ? manifest.contributes.keybindings.length : manifest.contributes?.keybindings ? 1 : 0;
    contributions += manifest.contributes?.commands?.length || 0;
    if (bytes > 8 * 1024 * 1024 || count > 50000 || bindings > 1024 || contributions > 1024) throw new Error('Extension session cumulative manifest or contribution limit exceeded');
  }
  for (const item of retained) account(item.manifest, item.manifestBytes);
  return inputs.map(input => {
    const folder = fs.realpathSync(input.path);
    const manifestPath = path.join(folder, 'package.json');
    if (!fs.statSync(manifestPath).isFile()) throw new Error('Extension manifest must be a regular file');
    const fd = fs.openSync(manifestPath, 'r');
    const contents = Buffer.alloc(1024 * 1024 + 1);
    let size = 0;
    try {
      while (size < contents.length) {
        const read = fs.readSync(fd, contents, size, contents.length - size, null);
        if (!read) break;
        size += read;
      }
    } finally { fs.closeSync(fd); }
    if (size > 1024 * 1024) throw new Error('Extension manifest exceeds 1 MiB');

    const manifest = JSON.parse(contents.subarray(0, size).toString('utf8'));
    account(manifest, size);
    const id = `${manifest.publisher}.${manifest.name}`.toLowerCase();
    if (id !== input.id || manifest.version !== input.version) throw new Error(`Extension identity changed: ${input.id}`);
    if (ids.has(id)) throw new Error(`Duplicate extension: ${id}`);
    ids.add(id);
    if (manifest.enabledApiProposals?.length) throw new Error(`${id}: proposed extension APIs are not implemented`);
    let entry;
    if (manifest.main !== undefined) {
      if (typeof manifest.main !== 'string') throw new Error(`${id}: a CommonJS extension main entry is required`);
      entry = require.resolve(path.resolve(folder, manifest.main));
      const relative = path.relative(folder, fs.realpathSync(entry));
      if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error('Extension main must be inside its directory');
    } else if (manifest.browser) throw new Error(`${id}: browser-only code is unsupported`);
    return { id, folder, entry, manifest, manifestBytes: size, subscriptions: [] };
  }).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}
function dispose(item) {
  for (const subscription of item.subscriptions.splice(0)) {
    try { subscription.dispose(); } catch (error) { console.error(error); }
  }
  runtime.disposeOwner(item.id);
}
function activationHooks() {
  return {
    async activate(item) {
      if (!item.entry) return undefined;
      const context = supported('ExtensionContext', {
        subscriptions: item.subscriptions, extensionPath: item.folder, extensionUri: Uri.file(item.folder),
        extensionMode: runtime.api.ExtensionMode.Production,
        asAbsolutePath: relative => path.join(item.folder, relative),
        ...runtime.contextForExtension(item.id),
      });
      const inherited = execution.getStore(), batch = activationRequest.getStore();
      // A completed invocation's timers must not borrow an unrelated batch ID.
      const origin = inherited ? (inherited.active ? inherited : undefined) : (batch?.active ? batch : undefined);
      activating++;
      try {
        return await inExecution(origin?.id ?? null, origin?.owner ?? item.id, async () => {
          item.extension = require(item.entry);
          return typeof item.extension.activate === 'function' ? await item.extension.activate(context) : item.extension;
        });
      } finally { activating--; }
    },
    failed(item) {
      activating++;
      try { dispose(item); } finally { activating--; }
    },
    changed() {
      if (ready && !activating) {
        send({ method: 'activation', params: { session, activation: activation.statuses() } });
        send({ method: 'commands', params: { session, commands: runtime.commandSnapshot() } });
        if (languageProviders) send({ method: 'languageProviders', params: { session, providers: runtime.providerSnapshot() } });
      }
    },
    listenerError: error => console.error(error),
    async deactivate(item) { try { await item.extension?.deactivate?.(); } finally { dispose(item); } },
  };
}
async function activateBatch(message, targets) {
  if (!Array.isArray(targets) || targets.length > 8 || targets.some(id => typeof id !== 'string' || !packages.some(item => item.id === id))) throw new Error('Invalid selected activation targets');
  const origin = { id: message.id, owner: message.params.owner, active: true };
  activationBusy = true;
  try { await activationRequest.run(origin, async () => { for (const id of [...new Set(targets)].sort()) await activation.activate(id); }); }
  finally { origin.active = false; activationBusy = false; }
}
function snapshot() {
  return { protocol: 4, session, commands: runtime.commandSnapshot(), activation: activation.statuses(),
    ...(languageProviders ? { languageProviders: runtime.providerSnapshot() } : {}),
    extensions: packages.map(item => ({ id: item.id, version: item.manifest.version,
      keybindings: item.manifest.contributes?.keybindings || [], contributions: item.manifest.contributes?.commands || [] })) };
}
async function dispatch(message) {
  if (!message.method) {
    const call = pending.get(message.id);
    if (!call) return;
    pending.delete(message.id); clearTimeout(call.timer);
    if (message.error) call.reject(new Error(message.error.message));
    else call.resolve(message.result);
    return;
  }
  let result = null;
  try {
    switch (message.method) {
      case 'initialize': {
        if (message.params.protocol !== 4) throw new Error('Unsupported native protocol version');
        if (initialized) throw new Error('Extension host already initialized');
        initialized = true; session = message.params.session; languageProviders = message.params.languageProviders === true;
        packages.push(...prepare(message.params.extensions));
        runtime = createApi(request, (method, params) => {
          if (method === 'nativeSurface') {
            const context = execution.getStore();
            if (context?.active) params = { ...params, command: context.id, commandOwner: context.owner };
          }
          // A failed startup must never publish partially activated command registries.
          if (!['commands', 'languageProviders'].includes(method) || (ready && !activating)) send({ method, params });
        }, { session, reservedCommands: message.params.reservedCommands, extensionState: message.params.extensionState, languageProviders });
        const schemas = packages.flatMap(item => Array.isArray(item.manifest.contributes?.configuration)
          ? item.manifest.contributes.configuration : [item.manifest.contributes?.configuration]);
        root = message.params.root; configuration = message.params.configuration;
        runtime.configure(root, schemas, configuration);
        runtime.configureSurfaces(packages);
        runtime.sync(message.params.state);
        activation = createActivation(packages, activationHooks());
        runtime.setActivation(activation);
        await activateBatch(message, message.params.activate ?? packages.map(item => item.id));
        ready = true;
        result = snapshot();
        break;
      }
      case 'state': if (runtime) runtime.sync(message.params); break;
      case 'configuration':
        configuration = message.params; if (runtime) runtime.updateConfiguration(configuration); break;
      case 'activate': {
        if (!ready || message.params.session !== session) throw new Error('Extension session is not ready');
        if (activationBusy) throw new Error('Another activation batch is in progress');
        const additions = message.params.extensions || [];
        const items = additions.length ? prepare(additions, packages) : [];
        const owner = message.params.owner;
        if (owner !== undefined && owner !== null && (typeof owner !== 'string' || ![...packages, ...items].some(item => item.id === owner))) throw new Error('Activation owner is not selected');
        if (items.length) {
          runtime.configureSurfaces(items);
          activation.add(items); packages.push(...items); packages.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
          runtime.configure(root, packages.flatMap(item => Array.isArray(item.manifest.contributes?.configuration) ? item.manifest.contributes.configuration : [item.manifest.contributes?.configuration]), configuration);
        }
        runtime.mergeExtensionState(message.params.extensionState || {});
        await activateBatch(message, message.params.activate);
        result = snapshot();
        break;
      }
      case 'execute':
        if (!ready || message.params.session !== session) throw new Error('Extension session is not ready');
        if (!runtime.commandSnapshot().some(item => item.id === message.params.command && item.owner === message.params.owner)) throw new Error('Extension command owner changed');
        result = await inExecution(message.id, message.params.owner, () => runtime.api.commands.executeCommand(message.params.command, ...message.params.args));
        break;
      case 'provideLanguage':
        if (!ready || message.params.session !== session || !activation.extension(message.params.owner)?.isActive) throw new Error('Language provider session/owner is not ready');
        result = await inExecution(message.id, message.params.owner, () => runtime.provideLanguage({ ...message.params, request: message.id }));
        break;
      case 'cancelLanguageProvider':
        runtime?.cancelLanguageProvider(message.params);
        break;
      case 'treeChildren':
      case 'surfaceAction':
        if (!ready || message.params.session !== session || !activation.extension(message.params.owner)?.isActive) throw new Error('Surface owner/session is not ready');
        result = await inExecution(message.id, message.params.owner, () => runtime[message.method](message.params));
        break;
      case 'treeEvent':
        // View refresh/disposal may race an already emitted visibility event.
        try { if (ready) runtime.treeEvent(message.params); } catch (error) { console.error(error); }
        break;
      case 'ping':
        if (!ready || message.params.session !== session) throw new Error('Extension session is not ready');
        break;
      case 'shutdown':
        ready = false;
        await activation.shutdown();
        break;
      default: throw new Error(`Unknown extension protocol method: ${message.method}`);
    }
    if (message.id !== undefined) send({ id: message.id, result: result ?? null });
  } catch (error) {
    if (message.id !== undefined) send({ id: message.id, error: { message: String(error.stack || error).slice(0, 8192) } });
    else { console.error(error); process.exit(1); }
  }
}
process.stdin.on('data', chunk => {
  try {
    if (buffer.length + chunk.length > MAX + 8192) throw new Error('Extension input limit exceeded');
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length) {
      const end = buffer.indexOf('\r\n\r\n');
      if (end < 0) { if (buffer.length > 8192) throw new Error('Protocol header limit exceeded'); break; }
      const header = buffer.subarray(0, end).toString('ascii');
      if (end > 8192 || !/^Content-Length: \d+$/i.test(header)) throw new Error('Invalid protocol header');
      const size = Number(header.split(':')[1]);
      if (!Number.isSafeInteger(size) || size > MAX) throw new Error('Protocol body limit exceeded');
      if (buffer.length < end + 4 + size) break;
      const message = JSON.parse(buffer.subarray(end + 4, end + 4 + size));
      buffer = buffer.subarray(end + 4 + size);
      // Replies and state updates must bypass awaited command callbacks.
      dispatch(message).catch(error => { console.error(error); process.exit(1); });
    }
  } catch (error) { console.error(error); process.exit(1); }
});
process.stdin.on('end', () => process.exit(0));
