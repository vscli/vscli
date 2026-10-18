'use strict';
// Node 24 needs explicit test paths; discover the same flat host contract suite
// without relying on shell wildcard expansion or newer directory discovery.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const directory = path.join(root, 'extension-host');
const tests = fs.readdirSync(directory, { withFileTypes: true })
  .filter(entry => entry.isFile() && entry.name.endsWith('.test.cjs'))
  .map(entry => path.join(directory, entry.name)).sort();
if (!tests.length) throw new Error('No extension host contract files found');
const result = spawnSync(process.execPath, ['--test', ...tests], { cwd: root, stdio: 'inherit' });
if (result.error) throw result.error;
if (result.signal) console.error(`Host contract runner terminated by ${result.signal}`);
process.exitCode = result.status ?? 1;
