'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

function frame(message) {
  const body = Buffer.from(JSON.stringify(message));
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
}

test('ordered document and configuration notifications are visible before command replies', { timeout: 10000 }, async t => {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-host-'));
  fs.writeFileSync(path.join(folder, 'package.json'), JSON.stringify({ name: 'wire', publisher: 'test', version: '1', main: 'extension.cjs' }));
  fs.writeFileSync(path.join(folder, 'extension.cjs'), `
    const vscode = require('vscode');
    exports.activate = context => {
      const initial = vscode.workspace.getConfiguration('wire').get('count');
      let changes = 0;
      context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
        if (event.affectsConfiguration('wire.count')) changes++;
      }));
      context.subscriptions.push(vscode.commands.registerCommand('configuration', () => ({
        initial, current: vscode.workspace.getConfiguration('wire').get('count'), changes,
      })));
      context.subscriptions.push(vscode.commands.registerCommand('edit', async () => {
      const editor = vscode.window.activeTextEditor;
      const applied = await editor.edit(edit => edit.insert(new vscode.Position(0, 0), 'X'));
      return { applied, text: editor.document.getText(), version: editor.document.version, cursor: editor.selection.active.character };
      }));
    };
  `);
  const child = spawn(process.execPath, [path.join(__dirname, 'host.cjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  let stderr = '', buffer = Buffer.alloc(0), failure;
  const messages = [], waiters = new Set();
  child.stderr.on('data', chunk => { stderr += chunk; });
  const wake = () => { for (const notify of [...waiters]) notify(); };
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
    if (child.exitCode === null && child.signalCode === null) {
      const ended = once(child, 'exit');
      child.kill(); await ended;
    }
    fs.rmSync(folder, { recursive: true, force: true });
  });
  function receive(predicate) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { waiters.delete(check); reject(new Error(`No matching frame: ${stderr}`)); }, 5000);
      const check = () => {
        const index = messages.findIndex(predicate);
        if (index >= 0 || failure) {
          clearTimeout(timer); waiters.delete(check);
          if (index >= 0) resolve(messages.splice(index, 1)[0]); else reject(failure);
        }
      };
      waiters.add(check); check();
    });
  }
  const state = { generation: 1, documents: [{ id: 1, uri: 'untitled:wire', text: 'old', version: 1, languageId: 'plaintext', isDirty: true }],
    active: 1, selections: [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 0 } }] };
  child.stdin.write(frame({ id: 1, method: 'initialize', params: { protocol: 3, extension: folder, root: folder, state,
    configuration: [{}, { 'wire.count': 2 }] } }));
  const initialized = await receive(message => message.id === 1);
  assert.equal(initialized.error, undefined);
  child.stdin.write(frame({ id: 2, method: 'execute', params: { command: 'edit', args: [] } }));
  const edit = await receive(message => message.method === 'edit');
  const updated = { ...state, generation: 2, documents: [{ ...state.documents[0], version: 2, text: 'Xold' }] };
  const { text, ...metadata } = updated.documents[0];
  const moved = { ...updated, generation: 3, documents: [metadata],
    selections: [{ anchor: { line: 0, character: 2 }, active: { line: 0, character: 2 } }] };
  child.stdin.write(Buffer.concat([
    frame({ method: 'state', params: updated }),
    frame({ id: edit.id, result: { applied: true } }),
    frame({ method: 'state', params: moved }),
  ]));
  const result = await receive(message => message.id === 2);
  assert.equal(result.error, undefined);
  assert.equal(result.result.applied, true);
  assert.equal(result.result.text, 'Xold');
  assert.equal(result.result.version, 2);
  // Pipes may split the write; either selection is valid when the promise resumes.
  assert.ok([0, 2].includes(result.result.cursor));
  child.stdin.write(Buffer.concat([
    frame({ method: 'configuration', params: [{}, { 'wire.count': 3 }] }),
    frame({ id: 3, method: 'execute', params: { command: 'configuration', args: [] } }),
  ]));
  const configured = await receive(message => message.id === 3);
  assert.equal(configured.error, undefined);
  assert.deepEqual(configured.result, { initial: 2, current: 3, changes: 1 });
});
