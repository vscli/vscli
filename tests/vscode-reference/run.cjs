'use strict';
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { supervise } = require('./supervisor.cjs');

async function main() {
  const root = path.resolve(__dirname, '../..');
  const output = path.resolve(process.argv[2] || path.join(root, 'target/vscode-reference', process.platform));
  const session = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-reference-'));
  try {
    await supervise(path.join(__dirname, 'worker.cjs'), [output, session]);
  } finally {
    fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
