'use strict';
const path = require('node:path');
const { supervise } = require('../supervisor.cjs');
supervise(path.join(__dirname, 'leaking-worker.cjs'), process.argv.slice(2),
  { timeoutMs: 5000, graceMs: 100, quiet: true })
  .catch(error => { console.error(error.message); process.exitCode = 1; });
