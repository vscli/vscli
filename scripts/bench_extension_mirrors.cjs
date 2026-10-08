'use strict';
// Non-gating microbenchmark: mirror updates only, excluding transport and Rust.
// VSCLI_BENCH_HOST can point at a previous checkout; --full-text exercises v1.
const { performance } = require('node:perf_hooks');
const path = require('node:path');
const { createApi } = require(path.resolve(process.env.VSCLI_BENCH_HOST || path.join(__dirname, '../extension-host'), 'api.cjs'));
const runtime = createApi(() => {}, () => {});
const text = '0123456789abcdef\n'.repeat(100000);
const state = { generation: 1, documents: [{ id: 1, uri: 'untitled:bench', text, version: 1, languageId: 'plaintext', isDirty: false }],
  active: 1, selections: [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 0 } }] };
runtime.sync(state);
if (!process.argv.includes('--full-text')) delete state.documents[0].text;
const samples = [];
for (let i = 0; i < 30; i++) {
  state.generation++;
  state.selections[0].active.line = i;
  const start = performance.now();
  runtime.sync(state);
  samples.push(performance.now() - start);
}
samples.sort((a, b) => a - b);
console.log(JSON.stringify({ runtime: process.version, platform: process.platform, arch: process.arch,
  bytes: Buffer.byteLength(text), updates: samples.length, payloadBytes: Buffer.byteLength(JSON.stringify(state)),
  meanMs: samples.reduce((a,b) => a+b, 0) / samples.length, p95Ms: samples[Math.ceil(samples.length * .95) - 1] }, null, 2));
