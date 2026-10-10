'use strict';
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { supervise } = require('./supervisor.cjs');

async function main() {
  const output = path.resolve(process.argv[2] || path.join(__dirname, '../../target/vscode-reference/language-configuration', process.platform));
  // macOS's default /var/folders temp path can exceed the editor's Unix
  // socket path limit once a profile and its IPC filename are appended.
  const sessionRoot = process.platform === 'darwin' ? '/tmp' : os.tmpdir();
  const session = fs.mkdtempSync(path.join(sessionRoot, 'vscli-lc-'));
  try {
    await supervise(path.join(__dirname, 'language-configuration-worker.cjs'),
      [output, session, ...process.argv.slice(3)], { timeoutMs: 300000 });
  } finally {
    fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
