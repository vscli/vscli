'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { supervise } = require('./supervisor.cjs');
const worker = path.join(__dirname, 'fixtures/leaking-worker.cjs');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-supervisor-'));
  const file = path.join(directory, 'pids');
  const pids = () => fs.existsSync(file) ? fs.readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(Number) : [];
  t.after(() => {
    for (const pid of pids()) {
      try { process.kill(pid, 'SIGKILL'); }
      catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
    fs.rmSync(directory, { recursive: true, force: true });
  });
  return { file, pids };
}

function alive(pid) {
  try { process.kill(pid, 0); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}

async function until(check) {
  const end = Date.now() + 5000;
  while (!check()) {
    assert.ok(Date.now() < end, 'Process condition did not settle within five seconds');
    await delay(25);
  }
}

for (const [mode, error] of [
  ['success', null], ['failure', /interrupted download/], ['invalid', /Invalid reference/],
]) {
  test(`${mode} result cleans up extraction child and grandchild`, { timeout: 15000 }, async t => {
    const state = fixture(t);
    const signals = ['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal));
    const result = supervise(worker, [mode, state.file], { timeoutMs: 5000, graceMs: 150, quiet: true });
    if (error) await assert.rejects(result, error); else await result;
    assert.equal(state.pids().length, 3, 'Worker, extraction child and grandchild must have started');
    await until(() => state.pids().every(pid => !alive(pid)));
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal)), signals);
  });
}

test('worker timeout fails even while extraction descendants are alive', { timeout: 15000 }, async t => {
  const state = fixture(t);
  await assert.rejects(supervise(worker, ['hang', state.file],
    { timeoutMs: 5000, graceMs: 150, quiet: true }), /exceeded/);
  assert.equal(state.pids().length, 3);
  await until(() => state.pids().every(pid => !alive(pid)));
});

test('zero exit without a completion result is a failure', { timeout: 15000 }, async t => {
  const state = fixture(t);
  await assert.rejects(supervise(worker, ['exit', state.file],
    { timeoutMs: 5000, graceMs: 150, quiet: true }), /without a completion result/);
  await until(() => state.pids().every(pid => !alive(pid)));
});

test('interrupting the supervisor fails and terminates its owned tree',
  { timeout: 15000, skip: process.platform === 'win32' && 'Windows kill() forcibly terminates instead of delivering SIGTERM' }, async t => {
    const state = fixture(t);
    const runner = spawn(process.execPath, [path.join(__dirname, 'fixtures/invoke.cjs'), 'hang', state.file],
      { stdio: 'ignore' });
    t.after(() => { if (runner.exitCode === null && runner.signalCode === null) runner.kill('SIGKILL'); });
    const exited = new Promise(resolve => runner.once('exit', (code, signal) => resolve({ code, signal })));
    await until(() => state.pids().length === 3);
    runner.kill('SIGTERM');
    const result = await exited;
    assert.equal(result.code, 1);
    assert.equal(result.signal, null);
    await until(() => state.pids().every(pid => !alive(pid)));
  });
