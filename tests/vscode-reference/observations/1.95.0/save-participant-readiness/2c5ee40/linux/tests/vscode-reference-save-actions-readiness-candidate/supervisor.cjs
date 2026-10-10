'use strict';
const { fork, execFile } = require('node:child_process');
const { promisify } = require('node:util');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

function signalGroup(pid, signal) {
  try { process.kill(-pid, signal); }
  catch (error) { if (error.code !== 'ESRCH') throw error; }
}

async function stopTree(child, graceMs) {
  if (!child.pid) return;
  if (process.platform === 'win32') {
    // Keep the worker alive until this call so taskkill can enumerate its tree.
    if (child.exitCode !== null || child.signalCode !== null) return;
    await promisify(execFile)('taskkill', ['/PID', String(child.pid), '/T', '/F'],
      { windowsHide: true, timeout: 10000 });
  } else {
    signalGroup(child.pid, 'SIGTERM');
    // The worker stays alive to reap extraction children during this interval.
    // Escalation also covers a descendant which ignores SIGTERM.
    await delay(graceMs);
    signalGroup(child.pid, 'SIGKILL');
  }
}

async function supervise(worker, args, { timeoutMs = 300000, graceMs = 1000, quiet = false } = {}) {
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || !Number.isFinite(graceMs) || graceMs < 0) {
    throw new Error('Supervisor timeouts must be finite and positive');
  }
  const child = fork(worker, args, {
    detached: process.platform !== 'win32', windowsHide: true, execArgv: [],
    stdio: ['ignore', quiet ? 'ignore' : 'inherit', quiet ? 'ignore' : 'inherit', 'ipc'],
  });
  const exited = new Promise(resolve => {
    child.once('exit', resolve);
    child.once('error', resolve);
  });
  let timeout, interrupt;
  try {
    const outcome = await new Promise(resolve => {
      let settled = false;
      const finish = error => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        resolve(error);
      };
      timeout = setTimeout(() => finish(new Error(`Reference worker exceeded ${timeoutMs} ms`)), timeoutMs);
      child.once('error', finish);
      child.once('exit', (code, signal) => finish(new Error(
        `Reference worker exited without a completion result (${signal || code})`)));
      child.on('message', message => {
        if (message?.type !== 'complete' || typeof message.ok !== 'boolean') {
          finish(new Error('Invalid reference worker completion result'));
        } else {
          finish(message.ok ? null : new Error(message.error || 'Reference worker failed'));
        }
      });
      interrupt = signal => finish(new Error(`Reference run interrupted by ${signal}`));
      process.on('SIGINT', interrupt);
      process.on('SIGTERM', interrupt);
    });
    let cleanupError;
    try {
      await stopTree(child, graceMs);
      let reapTimeout;
      try {
        await Promise.race([exited, new Promise((_resolve, reject) => {
          reapTimeout = setTimeout(() => reject(new Error('Reference worker did not exit after cleanup')), 5000);
        })]);
      } finally { clearTimeout(reapTimeout); }
    } catch (error) { cleanupError = error; }
    if (outcome && cleanupError) throw new AggregateError([outcome, cleanupError], 'Reference run and cleanup failed');
    if (cleanupError) throw cleanupError;
    if (outcome) throw outcome;
  } finally {
    clearTimeout(timeout);
    if (interrupt) {
      process.removeListener('SIGINT', interrupt);
      process.removeListener('SIGTERM', interrupt);
    }
    if (child.connected) child.disconnect();
  }
}

// Workers use an explicit result instead of exiting: leaked download/extraction
// handles must not prevent reporting completion, and Windows tree cleanup needs
// a live parent. A disconnected supervisor is never permission to keep running.
function workerLifetime() {
  if (!process.send) throw new Error('Reference workers must be launched by the supervisor');
  process.on('message', () => {});
  process.on('SIGTERM', () => {});
  process.on('SIGINT', () => {});
  process.on('disconnect', () => {
    if (process.platform === 'win32') {
      execFile('taskkill', ['/PID', String(process.pid), '/T', '/F'], { windowsHide: true }, () => process.exit(1));
    } else {
      signalGroup(process.pid, 'SIGKILL');
    }
  });
}

module.exports = { supervise, workerLifetime };
