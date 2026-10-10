'use strict';
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { supervise } = require('./supervisor.cjs');
async function main() {
  const output = path.resolve(process.argv[2] || 'target/editor-sticky-tabs-reference');
  const session = fs.mkdtempSync(path.join(process.platform === 'darwin' ? '/tmp' : os.tmpdir(), 'vscli-est-'));
  try {
    await supervise(path.join(__dirname, 'editor-sticky-tabs-worker.cjs'), [output, session, ...(process.argv[3] ? [process.argv[3]] : [])],
      { timeoutMs: 240000 });
  } finally { fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
}
main().catch(error => { console.error(error.stack || String(error)); process.exitCode = 1; });
