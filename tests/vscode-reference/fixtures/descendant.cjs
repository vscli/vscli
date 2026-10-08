'use strict';
const fs = require('node:fs');
const { fork } = require('node:child_process');
const [pidFile, role] = process.argv.slice(2);
fs.appendFileSync(pidFile, `${process.pid}\n`);
// Deliberately resist graceful shutdown to exercise escalation/tree termination.
process.on('SIGTERM', () => {});
process.stdin.resume();
setInterval(() => {}, 1000);
if (role === 'leaf') {
  process.send('ready');
} else {
  const child = fork(__filename, [pidFile, 'leaf'], {
    execArgv: [], stdio: ['pipe', 'inherit', 'inherit', 'ipc'],
  });
  child.once('message', () => process.stdout.write('ready\n'));
}
