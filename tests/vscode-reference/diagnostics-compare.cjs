'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { isDeepStrictEqual } = require('node:util');
const { diagnosticTrace } = require('./diagnostics.cjs');
const { createApi } = require('../../extension-host/api.cjs');

const EXPECTED_COLLECTION_SNAPSHOTS = 21;
const DOCUMENT_PHASES = new Set(['document-set-before-edit', 'document-edit-retention',
  'document-edited-local-read', 'document-close-retention', 'closed-document-local-read']);

// API-owned collection semantics are compared independently of document event
// adapters and the reference main thread's marker mirror. Keep the full raw
// reference file as evidence; excluding this bridge is an explicit limitation.
function collectionProjection(trace) {
  assert.equal(trace.schema, 1, 'Unexpected diagnostic trace schema');
  const snapshots = trace.snapshots.filter(snapshot => !DOCUMENT_PHASES.has(snapshot.name))
    .map(snapshot => {
      const copy = structuredClone(snapshot);
      if (copy.one?.entries) copy.one.entries = copy.one.entries.filter(([uri]) => uri !== 'open-document');
      if (copy.aggregate) copy.aggregate = copy.aggregate.filter(([uri, values]) => uri !== 'open-document' && values.length > 0);
      return copy;
    });
  assert.equal(snapshots.length, EXPECTED_COLLECTION_SNAPSHOTS,
    'Collection observations must not silently disappear');
  const events = trace.events.filter(event => !DOCUMENT_PHASES.has(event.phase)).map(event => ({ ...event,
    aggregate: event.aggregate.filter(([uri, values]) => uri !== 'open-document' && values.length > 0),
  }));
  const retainedEventUris = trace.retainedEventUris.filter(event => event.uris.some(uri => uri !== 'open-document'))
    .map(event => ({ ...event, uris: event.uris.filter(uri => uri !== 'open-document') }));
  return { schema: 1, snapshots, events, retainedEventUris };
}

async function shimTrace() {
  const host = createApi(() => { throw new Error('Collection reference must not request native editing'); }, () => {},
    { session: 7, languageProviders: true });
  host.configure(process.cwd(), [], [{}, {}]);
  return diagnosticTrace(host.forExtension('fixture.diagnostic-reference'), { includeDocumentBridge: false });
}

async function main(directory) {
  assert.ok(directory, 'Usage: node diagnostics-compare.cjs <reference-output>');
  const inventory = JSON.parse(fs.readFileSync(path.join(directory, 'inventory.json'), 'utf8'));
  assert.equal(inventory.version, '1.95.0');
  assert.equal(inventory.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const referencePath = path.join(directory, 'diagnostics.json');
  const expected = collectionProjection(JSON.parse(fs.readFileSync(referencePath, 'utf8')));
  const actual = collectionProjection(await shimTrace());
  const report = { schema: 1,
    reference: { version: inventory.version, commit: inventory.commit, platform: inventory.platform,
      arch: inventory.arch, sha256: createHash('sha256').update(fs.readFileSync(referencePath)).digest('hex') },
    collections: { observations: actual.snapshots.length, events: actual.events.length,
      equal: isDeepStrictEqual(actual, expected) },
    limitations: ['Document edit/close and main-thread marker mirror events remain reference-only in this comparison.',
      'Empty resources retained only by the reference main-thread marker mirror are excluded from global aggregation; collection empty membership/get/iteration are compared exactly.',
      'This gate compares optional-host collection API semantics; native Problems rendering and linter compatibility require separate native/PTY qualification.'],
  };
  fs.writeFileSync(path.join(directory, 'diagnostics-vscli.json'), JSON.stringify(actual, null, 2) + '\n');
  fs.writeFileSync(path.join(directory, 'diagnostics-comparison.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
  assert.deepEqual(actual, expected, 'VSCLI diagnostic collections differ from the pinned executable observations');
}

if (require.main === module) main(...process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
module.exports = { collectionProjection, shimTrace };
