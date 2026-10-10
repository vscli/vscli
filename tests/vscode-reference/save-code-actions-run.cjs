'use strict';
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { supervise } = require('./supervisor.cjs');
async function main() {
  const output = path.resolve(process.argv[2] || 'target/save-code-actions-reference');
  const session = fs.mkdtempSync(path.join(process.platform === 'darwin' ? '/tmp' : os.tmpdir(), 'vscli-sca-'));
  try {
    await supervise(path.join(__dirname, 'save-code-actions-worker.cjs'), [output, session], { timeoutMs: 180000 });
  } finally { fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
}
main().catch(error => { console.error(error.stack || String(error)); process.exitCode = 1; });
