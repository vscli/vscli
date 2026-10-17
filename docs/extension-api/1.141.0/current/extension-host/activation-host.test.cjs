'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
function frame(message) {
  const body = Buffer.from(JSON.stringify(message));
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
}
function harness(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-activation-'));
  const child = spawn(process.execPath, [path.join(__dirname, 'host.cjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  let buffer = Buffer.alloc(0), stderr = '', failure;
  const messages = [], waiting = new Set();
  const wake = () => { for (const callback of [...waiting]) callback(); };
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-8192); });
  child.on('error', error => { failure = error; wake(); });
  child.on('exit', code => { failure = new Error(`Host exited ${code}: ${stderr}`); wake(); });
  child.stdout.on('data', chunk => {
    buffer = Buffer.concat([buffer, chunk]);
    while (true) {
      const end = buffer.indexOf('\r\n\r\n');
      if (end < 0) break;
      const length = Number(buffer.subarray(0, end).toString().split(':')[1]);
      if (buffer.length < end + 4 + length) break;
      messages.push(JSON.parse(buffer.subarray(end + 4, end + 4 + length)));
      buffer = buffer.subarray(end + 4 + length);
    }
    wake();
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) { const exited = once(child, 'exit'); child.kill(); await exited; }
    fs.rmSync(root, { recursive: true, force: true });
  });
  function receive(predicate) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { waiting.delete(check); reject(new Error(`No matching frame: ${stderr}`)); }, 5000);
      function check() {
        const index = messages.findIndex(predicate);
        if (index >= 0 || failure) {
          clearTimeout(timer); waiting.delete(check);
          if (index >= 0) resolve(messages.splice(index, 1)[0]); else reject(failure);
        }
      }
      waiting.add(check); check();
    });
  }
  function item(name, source, manifest = {}) {
    const folder = path.join(root, name); fs.mkdirSync(folder);
    fs.writeFileSync(path.join(folder, 'package.json'), JSON.stringify({ publisher: 'test', name, version: '1', ...(source ? { main: 'main.cjs' } : {}), ...manifest }));
    if (source) fs.writeFileSync(path.join(folder, 'main.cjs'), source);
    return { id: `test.${name}`, version: '1', path: folder };
  }
  const send = message => child.stdin.write(frame(message));
  async function call(id, method, params) { send({ id, method, params }); return receive(message => message.id === id); }
  async function initialize(extensions, activate = []) {
    return call(1, 'initialize', { protocol: 4, session: 7, extensions, activate, root, state: { generation: 1, documents: [], selections: [] }, configuration: [] });
  }
  return { root, item, send, call, receive, initialize };
}
test('lazy host activation preserves cached exports and native origin across dependencies and append', { timeout: 10000 }, async t => {
  const h = harness(t);
  const b = h.item('b', `
    const vscode = require('vscode'); let count = 0;
    exports.activate = async context => {
      count++;
      await vscode.window.showInputBox({ prompt: 'Dependency consented input' });
      return { count, contextId: context.extension.id };
    };
  `);
  const a = h.item('a', `
    const vscode = require('vscode');
    exports.activate = async context => {
      const b = vscode.extensions.getExtension('test.b');
      if (!b.isActive) throw new Error('dependency must precede dependent');
      context.subscriptions.push(vscode.commands.registerCommand('a.read', () => ({ count: b.exports.count, context: b.exports.contextId })));
      return { same: (await b.activate()) === b.exports };
    };
  `, { extensionDependencies: ['test.b'] });
  const init = await h.initialize([a, b]);
  assert.equal(init.error, undefined); assert.deepEqual(init.result.commands, []);
  assert.deepEqual(init.result.activation.map(item => item.state), ['dormant', 'dormant']);
  h.send({ id: 2, method: 'activate', params: { session: 7, owner: 'test.a', extensions: [], activate: ['test.a'] } });
  const prompt = await h.receive(message => message.method === 'prompt');
  assert.equal(prompt.params.owner, 'test.b'); assert.equal(prompt.params.commandOwner, 'test.a'); assert.equal(prompt.params.command, 2);
  h.send({ id: prompt.id, result: null });
  const activated = await h.receive(message => message.id === 2);
  assert.equal(activated.error, undefined);
  assert.deepEqual(activated.result.activation.map(item => item.state), ['active', 'active']);
  assert.deepEqual((await h.call(3, 'execute', { session: 7, owner: 'test.a', command: 'a.read', args: [] })).result, { count: 1, context: 'test.b' });
  const c = h.item('c', `const vscode = require('vscode'); exports.activate = context => {
    const b = vscode.extensions.getExtension('test.b');
    context.subscriptions.push(vscode.commands.registerCommand('c.read', () => b.exports.count));
  };`, { extensionDependencies: ['test.b'] });
  assert.equal((await h.call(4, 'activate', { session: 7, owner: 'test.c', extensions: [c], activate: ['test.c'] })).error, undefined);
  assert.equal((await h.call(5, 'execute', { session: 7, owner: 'test.c', command: 'c.read', args: [] })).result, 1);
});
test('activation failure removes partial registrations while keeping older active package', { timeout: 10000 }, async t => {
  const h = harness(t);
  const a = h.item('a', `const vscode = require('vscode'); exports.activate = context => {
    context.subscriptions.push(vscode.commands.registerCommand('a.read', () => 'retained'));
  };`);
  assert.equal((await h.initialize([a], ['test.a'])).error, undefined);
  const b = h.item('b', `const vscode = require('vscode'); exports.activate = () => {
    vscode.commands.registerCommand('partial', () => 'must be removed'); throw new Error('bounded activation failure');
  };`);
  assert.match((await h.call(2, 'activate', { session: 7, extensions: [b], activate: ['test.b'] })).error.message, /bounded activation failure/);
  assert.equal((await h.call(3, 'execute', { session: 7, owner: 'test.a', command: 'a.read', args: [] })).result, 'retained');
  assert.match((await h.call(4, 'execute', { session: 7, owner: 'test.b', command: 'partial', args: [] })).error.message, /owner changed/);
  assert.match((await h.call(5, 'activate', { session: 7, extensions: [], activate: ['test.b'] })).error.message, /bounded activation failure/);
});

test('dynamic admission seeds Mementos before activation and only publishes active providers', { timeout: 10000 }, async t => {
  const h = harness(t);
  const a = h.item('a', `exports.activate = context => ({ saved: context.globalState.get('saved') });`);
  const init = await h.call(1, 'initialize', { protocol: 4, session: 7, extensions: [a], activate: ['test.a'],
    languageProviders: true, extensionState: { 'test.a': { global: { saved: 'original' } } }, root: h.root,
    state: { generation: 1, documents: [], selections: [] }, configuration: [] });
  assert.equal(init.error, undefined); assert.deepEqual(init.result.languageProviders, []);
  const b = h.item('b', `const v = require('vscode'); exports.activate = context => {
    if (context.globalState.get('saved') !== 'new' || context.workspaceState.get('saved') !== 'workspace') throw Error('missing state');
    if (v.extensions.getExtension('test.a').exports.saved !== 'original') throw Error('old state replaced');
    v.languages.registerHoverProvider('sql', { provideHover: () => new v.Hover('state loaded') });
  };`);
  const appended = await h.call(2, 'activate', { session: 7, extensions: [b], activate: ['test.b'], extensionState: {
    'test.a': { global: { saved: 'must not replace' } }, 'test.b': { global: { saved: 'new' }, workspace: { saved: 'workspace' } }
  } });
  assert.equal(appended.error, undefined); assert.equal(appended.result.languageProviders.length, 1);
  assert.equal(appended.result.languageProviders[0].owner, 'test.b');
  const bad = h.item('bad', `const v = require('vscode'); exports.activate = () => {
    v.languages.registerHoverProvider('sql', { provideHover: () => new v.Hover('never') }); throw Error('intentional failure');
  };`);
  assert.match((await h.call(3, 'activate', { session: 7, extensions: [bad], activate: ['test.bad'] })).error.message, /intentional failure/);
  const registry = await h.receive(message => message.method === 'languageProviders');
  assert.deepEqual(registry.params.providers.map(item => item.owner), ['test.b']);
  assert.match((await h.call(4, 'provideLanguage', { session: 7, owner: 'test.bad', provider: 2, document: 1, version: 1 })).error.message, /not ready/);
});
