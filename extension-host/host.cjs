'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { Console } = require('node:console');
const { createApi, supported } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');
// Console output from extensions must never corrupt the protocol stream.
global.console = new Console(process.stderr, process.stderr);
const MAX = 16 * 1024 * 1024;
let buffer = Buffer.alloc(0), nextId = 0, initialized = false, extension;
const pending = new Map(), subscriptions = [];
function send(message) {
  const body = Buffer.from(JSON.stringify(message));
  if (body.length > MAX || process.stdout.writableLength + body.length > 64 * 1024 * 1024) {
    throw new Error('Extension protocol output limit exceeded');
  }
  process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
  process.stdout.write(body);
}
function request(method, params) {
  if (pending.size >= 64) return Promise.reject(new Error('Extension request limit exceeded'));
  const id = `host-${++nextId}`;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Native request timed out: ${method}`)); }, 5000);
    pending.set(id, { resolve, reject, timer });
    try { send({ id, method, params }); }
    catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
  });
}
const runtime = createApi(request, (method, params) => send({ method, params }));
const originalLoad = Module._load;
Module._load = function(name, parent, isMain) {
  if (name === 'vscode') return runtime.api;
  return originalLoad.call(this, name, parent, isMain);
};
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
        if (initialized) throw new Error('Extension host already initialized');
        initialized = true;
        const folder = fs.realpathSync(message.params.extension);
        const manifestPath = path.join(folder, 'package.json');
        if (fs.statSync(manifestPath).size > 1024 * 1024) throw new Error('Extension manifest exceeds 1 MiB');
        const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
        if (manifest.extensionDependencies?.length) throw new Error('Extension dependencies are not implemented');
        if (typeof manifest.main !== 'string') throw new Error('A CommonJS extension main entry is required');
        const entry = require.resolve(path.resolve(folder, manifest.main));
        const relative = path.relative(folder, fs.realpathSync(entry));
        if (relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('Extension main must be inside its directory');
        runtime.configure(message.params.root, manifest.contributes?.configuration);
        runtime.sync(message.params.state);
        const context = supported('ExtensionContext', {
          subscriptions, extensionPath: folder, extensionUri: Uri.file(folder),
          extensionMode: runtime.api.ExtensionMode.Production,
          asAbsolutePath: relative => path.join(folder, relative),
        });
        extension = require(entry);
        if (typeof extension.activate === 'function') await extension.activate(context);
        result = { id: `${manifest.publisher}.${manifest.name}`, version: manifest.version,
          commands: await runtime.api.commands.getCommands(), contributions: manifest.contributes?.commands || [] };
        break;
      }
      case 'state': runtime.sync(message.params); break;
      case 'execute': result = await runtime.api.commands.executeCommand(message.params.command, ...message.params.args); break;
      case 'shutdown':
        if (extension?.deactivate) await extension.deactivate();
        for (const subscription of subscriptions) subscription.dispose();
        break;
      default: throw new Error(`Unknown extension protocol method: ${message.method}`);
    }
    if (message.id !== undefined) send({ id: message.id, result: result ?? null });
  } catch (error) {
    if (message.id !== undefined) send({ id: message.id, error: { message: String(error.stack || error).slice(0, 8192) } });
    else console.error(error);
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
      dispatch(message).catch(error => { console.error(error); process.exit(1); });
    }
  } catch (error) { console.error(error); process.exit(1); }
});
process.stdin.on('end', () => process.exit(0));
