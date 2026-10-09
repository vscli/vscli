'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
function frame(message) { const body = Buffer.from(JSON.stringify(message)); return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]); }
async function harness(t) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-provider-wire-'));
  fs.writeFileSync(path.join(folder, 'package.json'), JSON.stringify({ name: 'provider', publisher: 'test', version: '1', main: 'extension.cjs' }));
  fs.writeFileSync(path.join(folder, 'extension.cjs'), `
    const v = require('vscode');
    exports.activate = context => {
      const registration = v.languages.registerCompletionItemProvider('cpp', {
        provideCompletionItems(document, position, token) {
          if (position.character === 4) {
            v.window.showInformationMessage('provider waiting');
            return new Promise(resolve => token.onCancellationRequested(() => resolve([])));
          }
          const item = new v.CompletionItem('name', v.CompletionItemKind.Variable);
          item.textEdit = v.TextEdit.replace(new v.Range(0, 0, 0, 4), 'replacement');
          return [item];
        },
      }, '.');
      context.subscriptions.push(registration);
      context.subscriptions.push(v.commands.registerCommand('provider.dispose', () => registration.dispose()));
    };
  `);
  const child = spawn(process.execPath, [path.join(__dirname, 'host.cjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  const messages = [], waiters = new Set(); let buffer = Buffer.alloc(0), stderr = '', failure;
  const wake = () => { for (const check of [...waiters]) check(); };
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-8192); });
  child.on('error', error => { failure = error; wake(); });
  child.on('exit', code => { failure = new Error(`Provider host exited ${code}: ${stderr}`); wake(); });
  child.stdout.on('data', chunk => {
    buffer = Buffer.concat([buffer, chunk]);
    while (true) {
      const end = buffer.indexOf('\r\n\r\n'); if (end < 0) break;
      const length = Number(buffer.subarray(0, end).toString().split(':')[1]); if (buffer.length < end + 4 + length) break;
      messages.push(JSON.parse(buffer.subarray(end + 4, end + 4 + length))); buffer = buffer.subarray(end + 4 + length);
    }
    wake();
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      const ended = once(child, 'exit'); child.kill('SIGKILL');
      await Promise.race([ended, new Promise((_, reject) => { const timer = setTimeout(() => reject(new Error('Provider fixture child did not exit')), 3000); ended.finally(() => clearTimeout(timer)); })]);
    }
    fs.rmSync(folder, { recursive: true, force: true });
  });
  const send = message => child.stdin.write(frame(message));
  const receive = predicate => new Promise((resolve, reject) => {
    const timer = setTimeout(() => { waiters.delete(check); reject(new Error(`Provider frame timed out: ${stderr}`)); }, 5000);
    const check = () => {
      const index = messages.findIndex(predicate); if (index < 0 && !failure) return;
      clearTimeout(timer); waiters.delete(check); if (index >= 0) resolve(messages.splice(index, 1)[0]); else reject(failure);
    }; waiters.add(check); check();
  });
  const state = { generation: 1, documents: [{ id: 1, uri: 'file:///test.cpp', text: '猫🙂x\r\n', version: 3, languageId: 'cpp', isDirty: true }], active: 1, selections: [] };
  send({ id: 1, method: 'initialize', params: { protocol: 4, session: 7, languageProviders: true, extensions: [{ id: 'test.provider', version: '1', path: folder }], root: folder, state, configuration: [] } });
  const initialized = await receive(message => message.id === 1); assert.equal(initialized.error, undefined);
  const provider = initialized.result.languageProviders[0]; assert.equal(provider.owner, 'test.provider');
  const params = { session: 7, owner: provider.owner, provider: provider.id, document: 1, version: 3, position: { line: 0, character: 3 } };
  return { send, receive, params, state };
}
test('host publishes activated providers and serves original edits with exact-owner cancellation', { timeout: 10000 }, async t => {
  const { send, receive, params } = await harness(t);
  send({ id: 2, method: 'provideLanguage', params });
  const result = await receive(message => message.id === 2); assert.equal(result.error, undefined);
  assert.equal(result.result.items[0].textEdit.newText, 'replacement'); assert.equal(result.result.items[0].kind, 6);
  send({ id: 3, method: 'provideLanguage', params: { ...params, position: { line: 0, character: 4 } } });
  await receive(message => message.method === 'message' && message.params.text === 'provider waiting');
  send({ id: 4, method: 'cancelLanguageProvider', params: { session: 7, owner: 'wrong.owner', request: 3 } });
  await receive(message => message.id === 4);
  send({ method: 'cancelLanguageProvider', params: { session: 7, owner: params.owner, request: 3 } });
  assert.match((await receive(message => message.id === 3)).error.message, /canceled|stale/);
  send({ id: 5, method: 'execute', params: { session: 7, owner: params.owner, command: 'provider.dispose', args: [] } });
  const removed = await receive(message => message.method === 'languageProviders'); assert.deepEqual(removed.params.providers, []);
  assert.equal((await receive(message => message.id === 5)).error, undefined);
  send({ id: 6, method: 'provideLanguage', params }); assert.match((await receive(message => message.id === 6)).error.message, /invalid provider/);
});
test('ordered document updates cancel an awaited language callback while the host remains responsive', { timeout: 10000 }, async t => {
  const { send, receive, params, state } = await harness(t);
  send({ id: 2, method: 'provideLanguage', params: { ...params, position: { line: 0, character: 4 } } });
  await receive(message => message.method === 'message' && message.params.text === 'provider waiting');
  send({ method: 'state', params: { ...state, generation: 2, documents: [{ ...state.documents[0], version: 4, text: 'new' }] } });
  assert.match((await receive(message => message.id === 2)).error.message, /canceled|stale/);
  send({ id: 3, method: 'ping', params: { session: 7 } }); assert.equal((await receive(message => message.id === 3)).error, undefined);
});
