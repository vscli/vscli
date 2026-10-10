'use strict';
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { supervise } = require('./supervisor.cjs');

async function main() {
  const output = path.resolve(process.argv[2] || path.join(__dirname, '../../target/vscode-reference/advanced', process.platform));
  const session = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-advanced-reference-'));
  try {
    // This qualification has its own bounded worker; the ordinary reference
    // suite retains its existing shorter deadline and corpus.
    await supervise(path.join(__dirname, 'advanced-indentation-worker.cjs'), [output, session], { timeoutMs: 300000 });
  } finally {
    fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
