'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { collectionProjection } = require('./diagnostics-compare.cjs');

function sha256(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function validateCapture(inventory, provenance, traceBytes, observerBytes) {
  assert.equal(inventory.version, '1.95.0');
  assert.equal(inventory.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  assert.ok(['linux', 'darwin', 'win32'].includes(inventory.platform));
  assert.equal(provenance.schema, 1);
  for (const key of ['version', 'commit', 'platform', 'arch']) assert.equal(provenance.reference[key], inventory[key]);
  assert.equal(provenance.observerSha256, sha256(observerBytes), 'Observer changed after executable capture');
  assert.equal(provenance.traceSha256, sha256(traceBytes), 'Observed trace artifact changed');
  const trace = JSON.parse(traceBytes);
  assert.equal(trace.snapshots.length, 26, 'Full executable observations must not disappear');
  assert.equal(provenance.observations, trace.snapshots.length);
  assert.equal(provenance.events, trace.events.length);
  const projected = collectionProjection(trace);
  assert.equal(projected.events.length, 15, 'Scoped collection event observations must not disappear');
  return projected;
}

function record(directory, destination = path.join(__dirname, 'baselines/1.95.0/diagnostics')) {
  assert.ok(directory, 'Usage: node diagnostics-record.cjs <reference-output> [baseline-directory]');
  const inventory = JSON.parse(fs.readFileSync(path.join(directory, 'inventory.json'), 'utf8'));
  const provenance = JSON.parse(fs.readFileSync(path.join(directory, 'diagnostics-provenance.json'), 'utf8'));
  const bytes = fs.readFileSync(path.join(directory, 'diagnostics.json'));
  const projected = validateCapture(inventory, provenance, bytes, fs.readFileSync(path.join(__dirname, 'diagnostics.cjs')));
  fs.mkdirSync(destination, { recursive: true });
  const manifestPath = path.join(destination, 'provenance.json');
  const manifest = fs.existsSync(manifestPath) ? JSON.parse(fs.readFileSync(manifestPath, 'utf8')) : {
    schema: 1, reference: { version: inventory.version, commit: inventory.commit }, platforms: {},
    limitations: ['Only recorded executable platforms are included; missing platforms are not inferred.',
      'Raw traces retain main-thread mirror bookkeeping and actual document edit/close observations.',
      'The shared optional-host gate compares collection-local state, coalesced event URI sets and nonempty global aggregate diagnostics; global empty-resource bookkeeping and native document adapters are excluded.'],
  };
  assert.equal(manifest.reference.version, inventory.version);
  assert.equal(manifest.reference.commit, inventory.commit);
  fs.writeFileSync(path.join(destination, `${inventory.platform}.json`), bytes);
  manifest.platforms[inventory.platform] = { ...provenance, collectionSnapshots: projected.snapshots.length,
    collectionEvents: projected.events.length,
    projectionSha256: sha256(fs.readFileSync(path.join(__dirname, 'diagnostics-compare.cjs'))),
    capture: 'Actual pinned executable under the existing supervised isolated reference harness; local Linux uses an isolated Xvfb display.' };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  return manifest.platforms[inventory.platform];
}
if (require.main === module) console.log(JSON.stringify(record(...process.argv.slice(2)), null, 2));
module.exports = { validateCapture, record };
