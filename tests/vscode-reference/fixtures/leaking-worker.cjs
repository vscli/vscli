'use strict';
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { PassThrough } = require('node:stream');
const { workerLifetime } = require('../supervisor.cjs');
workerLifetime();
const [mode, pidFile] = process.argv.slice(2);
fs.appendFileSync(pidFile, `${process.pid}\n`);
if (mode === 'exit') process.exit(0);

async function interruptedExtraction() {
  const child = spawn(process.execPath, [path.join(__dirname, 'descendant.cjs'), pidFile, 'branch'],
    { stdio: ['pipe', 'pipe', 'inherit'] });
  const input = new PassThrough();
  // Reproduce the dependency's failed-stream behavior: reject without closing
  // child stdin or awaiting/terminating the extraction child.
  const extraction = new Promise((resolve, reject) => {
    input.on('error', reject);
    input.pipe(child.stdin);
    child.once('error', reject);
    child.once('exit', resolve);
  });
  child.stdout.once('data', () => input.destroy(new Error('interrupted download')));
  await extraction;
}

interruptedExtraction().catch(error => {
  if (mode === 'hang') return;
  if (mode === 'invalid') { process.send({ ok: true }); return; }
  process.send({ type: 'complete', ok: mode === 'success', error: error.message });
});
