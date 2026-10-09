'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { Console } = require('node:console');
const { AsyncLocalStorage } = require('node:async_hooks');
const execution = new AsyncLocalStorage();
const { createApi, supported } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');
// Console output from extensions must never corrupt the protocol stream.
global.console = new Console(process.stderr, process.stderr);
const MAX = 16 * 1024 * 1024;
let buffer = Buffer.alloc(0), nextId = 0, initialized = false, ready = false, runtime, session;
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
  if (method === 'prompt') {
    const context = execution.getStore();
    if (context) params = { ...params, command: context.id, commandOwner: context.owner };
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
function prepare(inputs) {
  if (!Array.isArray(inputs) || !inputs.length || inputs.length > 8) throw new Error('Select between one and eight code extensions');
  let bytes = 0, count = 0, bindings = 0, contributions = 0;
  const ids = new Set();
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
    bytes += size;
    if (bytes > 8 * 1024 * 1024) throw new Error('Extension session manifests exceed 8 MiB');
    const manifest = JSON.parse(contents.subarray(0, size).toString('utf8'));
    count += nodes(manifest);
    if (count > 50000) throw new Error('Extension session manifests exceed 50,000 nodes');
    const id = `${manifest.publisher}.${manifest.name}`.toLowerCase();
    if (id !== input.id || manifest.version !== input.version) throw new Error(`Extension identity changed: ${input.id}`);
    if (ids.has(id)) throw new Error(`Duplicate extension: ${id}`);
    ids.add(id);
    if (manifest.enabledApiProposals?.length) throw new Error(`${id}: proposed extension APIs are not implemented`);
    if (manifest.extensionDependencies?.length) throw new Error(`${id}: extension dependencies are not implemented`);
    if (typeof manifest.main !== 'string') throw new Error(`${id}: a CommonJS extension main entry is required`);
    const entry = require.resolve(path.resolve(folder, manifest.main));
    const relative = path.relative(folder, fs.realpathSync(entry));
    if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error('Extension main must be inside its directory');
    bindings += Array.isArray(manifest.contributes?.keybindings) ? manifest.contributes.keybindings.length : manifest.contributes?.keybindings ? 1 : 0;
    contributions += manifest.contributes?.commands?.length || 0;
    if (bindings > 1024 || contributions > 1024) throw new Error('Extension session contribution limit exceeded');
    return { id, folder, entry, manifest, subscriptions: [] };
  }).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
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
        initialized = true; session = message.params.session;
        packages.push(...prepare(message.params.extensions));
        runtime = createApi(request, (method, params) => {
          // A failed startup must never publish partially activated command registries.
          if (method !== 'commands' || ready) send({ method, params });
        }, { session, reservedCommands: message.params.reservedCommands });
        const schemas = packages.flatMap(item => Array.isArray(item.manifest.contributes?.configuration)
          ? item.manifest.contributes.configuration : [item.manifest.contributes?.configuration]);
        runtime.configure(message.params.root, schemas, message.params.configuration);
        runtime.sync(message.params.state);
        for (const item of packages) {
          const context = supported('ExtensionContext', {
            subscriptions: item.subscriptions, extensionPath: item.folder, extensionUri: Uri.file(item.folder),
            extensionMode: runtime.api.ExtensionMode.Production,
            asAbsolutePath: relative => path.join(item.folder, relative),
          });
          try {
            item.extension = require(item.entry);
            if (typeof item.extension.activate === 'function') await execution.run({ id: message.id, owner: item.id }, () => item.extension.activate(context));
          } catch (error) { console.error(error); throw new Error(`${item.id}: ${error.message || error}`, { cause: error }); }
        }
        ready = true;
        result = { protocol: 4, session, commands: runtime.commandSnapshot(),
          extensions: packages.map(item => ({ id: item.id, version: item.manifest.version,
            keybindings: item.manifest.contributes?.keybindings || [], contributions: item.manifest.contributes?.commands || [] })) };
        break;
      }
      case 'state': if (runtime) runtime.sync(message.params); break;
      case 'configuration': if (runtime) runtime.updateConfiguration(message.params); break;
      case 'execute':
        if (!ready || message.params.session !== session) throw new Error('Extension session is not ready');
        if (!runtime.commandSnapshot().some(item => item.id === message.params.command && item.owner === message.params.owner)) throw new Error('Extension command owner changed');
        result = await execution.run({ id: message.id, owner: message.params.owner }, () => runtime.api.commands.executeCommand(message.params.command, ...message.params.args));
        break;
      case 'ping':
        if (!ready || message.params.session !== session) throw new Error('Extension session is not ready');
        break;
      case 'shutdown':
        ready = false;
        for (const item of [...packages].reverse()) {
          if (item.extension?.deactivate) await item.extension.deactivate();
          for (const subscription of item.subscriptions) subscription.dispose();
          runtime.disposeOwner(item.id);
        }
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
