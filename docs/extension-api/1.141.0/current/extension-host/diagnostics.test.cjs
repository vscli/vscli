'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const turn = () => new Promise(resolve => setImmediate(resolve));
const settle = () => new Promise(resolve => setTimeout(resolve, 70));
function harness() {
  const messages = [], runtime = createApi(() => { throw new Error('Unexpected native request'); }, (method, params) => messages.push({ method, params }), { session: 7, languageProviders: true });
  const api = runtime.forExtension('fixture.one'), other = runtime.forExtension('fixture.two');
  const uri = api.Uri.parse('untitled:a'), b = api.Uri.parse('untitled:b');
  const snapshots = [uri, b].map((uri, index) => ({ id: index + 1, uri: uri.toString(), version: 1, text: '猫🙂 text\r\nlast', languageId: 'plaintext', isDirty: true, savedGeneration: 0 }));
  let generation = 0;
  const sync = () => runtime.sync({ generation: ++generation, documents: snapshots, active: 1, selections: [] });
  sync();
  const diagnostic = message => new api.Diagnostic(new api.Range(0, 1, 0, 3), message, api.DiagnosticSeverity.Warning);
  return { runtime, api, other, messages, uri, b, snapshots, sync, diagnostic, wire: owner => messages.filter(message => message.method === 'diagnosticCollections' && message.params.owner === owner).at(-1)?.params };
}
test('collections preserve diagnostic identity, frozen copies, empty groups and distinct names', async () => {
  const h = harness(), one = h.api.languages.createDiagnosticCollection('same'), two = h.api.languages.createDiagnosticCollection('same'), d = h.diagnostic('original');
  const source = [d]; one.set(h.uri, source); source.length = 0;
  assert.equal(one.get(h.uri)[0], d); assert(Object.isFrozen(one.get(h.uri))); assert(!Object.isFrozen(one.get(h.b)));
  one.set([[h.b, []], [h.uri, [d]], [h.uri, undefined], [h.uri, [d]]]);
  assert.equal(one.get(h.uri)[0], d); assert(one.has(h.b));
  one.set([[h.uri, []], [h.b, [d]]]); assert(!one.has(h.uri));
  two.set(h.b, [d]); assert.equal(h.api.languages.getDiagnostics(h.b)[0], d); assert.equal(h.api.languages.getDiagnostics(h.b).length, 2);
  await turn(); const wire = h.wire('fixture.one'); assert.equal(wire.collections.length, 2); assert.notEqual(wire.collections[0].id, wire.collections[1].id);
  one.dispose(); one.dispose(); for (const read of [() => one.name, () => one.get(h.uri), () => one.has(h.uri), () => one.clear(), () => [...one]]) assert.throws(read, /illegal state - object is disposed/);
  two.dispose(); await settle();
});
test('wire retains UTF-16/CRLF metadata and original custom data stays local', async () => {
  const h = harness(), collection = h.api.languages.createDiagnosticCollection('rich'), d = h.diagnostic('猫🙂 warning');
  d.fixInfo = { original: d }; d.source = 'linter'; d.code = { value: 'W001', target: h.api.Uri.parse('https://example.invalid/code') };
  d.tags = [h.api.DiagnosticTag.Unnecessary]; d.relatedInformation = [new h.api.DiagnosticRelatedInformation(new h.api.Location(h.b, new h.api.Range(1, 0, 1, 2)), 'related')];
  collection.set(h.uri, [d]); await turn(); const entry = h.wire('fixture.one').collections[0].entries[0];
  assert.equal(entry.document, 1); assert.equal(entry.version, 1); assert.equal(entry.diagnostics[0].severity, 2);
  assert.equal(entry.diagnostics[0].range.end.character, 3); assert.equal(entry.diagnostics[0].codeDescription.href, 'https://example.invalid/code'); assert.equal(entry.diagnostics[0].fixInfo, undefined);
  assert.equal(collection.get(h.uri)[0].fixInfo.original, d);
  const bad = h.diagnostic('bad'); bad.range = new h.api.Range(0, 2, 0, 3); assert.throws(() => collection.set(h.uri, [bad]), /splits UTF-16/);
  assert.equal(collection.get(h.uri)[0], d); collection.dispose(); await settle();
});
test('changes invalidate all owners wire while original API objects survive edits, close and reopen', async () => {
  const h = harness(), one = h.api.languages.createDiagnosticCollection(), two = h.other.languages.createDiagnosticCollection(), d = h.diagnostic('kept');
  one.set(h.uri, [d]); two.set(h.uri, [d]); await turn();
  h.snapshots[0] = { ...h.snapshots[0], version: 2, text: 'changed' }; h.sync(); await turn();
  assert.equal(h.wire('fixture.one').collections[0].entries.length, 0); assert.equal(h.wire('fixture.two').collections[0].entries.length, 0);
  assert.equal(one.get(h.uri)[0], d); one.set(h.b, [h.diagnostic('other doc')]); await turn(); assert.equal(h.wire('fixture.one').collections[0].entries.length, 1);
  const closed = h.snapshots.shift(); h.sync(); await turn(); assert.equal(one.get(h.uri)[0], d);
  h.snapshots.push({ ...closed, id: 3, version: 1 }); h.sync(); await turn(); assert.equal(h.wire('fixture.two').collections[0].entries.length, 0);
  one.dispose(); two.dispose(); await settle();
});
test('same-turn changes are coalesced and owner retirement isolates other collections/listeners', async () => {
  const h = harness(), a = h.api.languages.createDiagnosticCollection(), b = h.other.languages.createDiagnosticCollection(), events = [];
  h.other.languages.onDidChangeDiagnostics(event => events.push(event));
  a.set(h.uri, [h.diagnostic('a')]); a.clear(); a.set(h.uri, [h.diagnostic('a')]); b.set(h.b, [h.diagnostic('b')]);
  await turn(); assert.equal(h.messages.filter(message => message.method === 'diagnosticCollections' && message.params.owner === 'fixture.one').length, 1);
  await settle(); assert.equal(events.length, 1); assert(Object.isFrozen(events[0].uris)); assert.equal(events[0].uris.length, 2);
  h.runtime.disposeOwner('fixture.one'); await turn(); assert.equal(h.wire('fixture.one').collections.length, 0); assert.equal(b.get(h.b).length, 1); assert.throws(() => a.name, /disposed/);
  h.runtime.disposeOwner('fixture.two'); await settle(); assert.equal(events.length, 1);
});
test('invalid bulk, count, message, source/resource bounds reject atomically', async () => {
  const h = harness(), collection = h.api.languages.createDiagnosticCollection(), d = h.diagnostic('keep'); collection.set(h.uri, [d]); await turn(); const count = h.messages.length;
  assert.throws(() => collection.set([[h.b, [d]], [h.uri, [h.diagnostic('x'.repeat(4097))]]]), /budget/);
  assert.equal(collection.get(h.b).length, 0); assert.equal(collection.get(h.uri)[0], d);
  assert.throws(() => collection.set(h.b, Array(5001).fill(d)), /5000/);
  assert.throws(() => collection.set(h.b, Array(600).fill(h.diagnostic('x'.repeat(4096)))), /2 MiB/);
  assert.throws(() => collection.set(Array.from({ length: 129 }, (_, i) => [h.api.Uri.parse(`untitled:${i}`), []])), /128/);
  const extra = []; for (let i = 0; i < 63; i++) extra.push(h.api.languages.createDiagnosticCollection());
  assert.throws(() => h.api.languages.createDiagnosticCollection(), /64/); await turn(); assert.equal(collection.get(h.uri)[0], d); assert(h.messages.length >= count);
  for (const c of extra) c.dispose(); collection.dispose(); await settle();
});
test('adversarial getters cannot resurrect disposed owners or publish stale/reentrant updates', async () => {
  const h = harness(), one = h.api.languages.createDiagnosticCollection(), two = h.other.languages.createDiagnosticCollection(), d = h.diagnostic('original');
  one.set(h.uri, [d]); two.set(h.b, [d]); await turn();
  const disposed = h.diagnostic('dispose'); Object.defineProperty(disposed, 'message', { get() { h.runtime.disposeOwner('fixture.one'); return 'stale'; } });
  assert.throws(() => one.set(h.uri, [disposed]), /disposed/); await turn(); assert.equal(h.wire('fixture.one').collections.length, 0); assert.equal(two.get(h.b)[0], d);
  const newer = h.diagnostic('newer'), reentrant = h.diagnostic('outer'); Object.defineProperty(reentrant, 'message', { get() { two.set(h.b, [newer]); return 'outer'; } });
  assert.throws(() => two.set(h.b, [reentrant]), /changed during normalization/); assert.equal(two.get(h.b)[0], newer);
  const changed = h.diagnostic('changed'); Object.defineProperty(changed, 'message', { get() { h.snapshots[1] = { ...h.snapshots[1], version: 2, text: 'newer bytes' }; h.sync(); return 'stale'; } });
  assert.throws(() => two.set(h.b, [changed]), /document changed/); await turn(); assert.equal(h.wire('fixture.two').collections[0].entries.length, 0);
  two.dispose(); await settle();
});
test('URI getter disposal rejects delete and owner retirement publishes despite saturated events', async () => {
  const h = harness(), collection = h.api.languages.createDiagnosticCollection(), d = h.diagnostic('original'); collection.set(h.uri, [d]); await settle();
  const evil = h.api.Uri.parse('untitled:evil'); evil.toString = () => { collection.dispose(); return 'untitled:evil'; };
  assert.throws(() => collection.delete(evil), /disposed/); await turn(); assert.equal(h.wire('fixture.one').collections.length, 0);
  const other = h.other.languages.createDiagnosticCollection(); other.set(h.b, [d]); await settle();
  for (let i = 0; i < 128; i++) other.delete(h.api.Uri.parse(`untitled:absent${i}`));
  h.runtime.disposeOwner('fixture.two'); await turn(); assert.equal(h.wire('fixture.two').collections.length, 0); await settle();
});
test('save notifications baseline once, update all mirrors first, retain text version and retire listeners', async () => {
  const h = harness(), events = []; h.api.workspace.onDidSaveTextDocument(document => events.push([document, h.api.workspace.textDocuments.map(d => d._snapshot.savedGeneration), document.version]));
  h.snapshots[0] = { ...h.snapshots[0], savedGeneration: 1 }; h.snapshots[1] = { ...h.snapshots[1], savedGeneration: 1 }; h.sync();
  assert.equal(events.length, 2); assert.deepEqual(events[0].slice(1), [[1, 1], 1]); assert.equal(events[0][0], h.api.workspace.textDocuments[0]);
  h.sync(); assert.equal(events.length, 2); h.runtime.disposeOwner('fixture.one'); h.snapshots[0].savedGeneration = 2; h.sync(); assert.equal(events.length, 2);
  const first = harness(), called = []; first.other.workspace.onDidSaveTextDocument(document => called.push(document)); first.snapshots.push({ ...first.snapshots[0], id: 3, uri: 'untitled:initial-save', savedGeneration: 12 }); first.sync(); assert.equal(called.length, 0);
});

test('an edit and successful persistence in the same mirror update deliver change before save', () => {
  const h = harness(), events = []; h.api.workspace.onDidChangeTextDocument(() => events.push('change')); h.api.workspace.onDidSaveTextDocument(() => events.push('save'));
  h.snapshots[0] = { ...h.snapshots[0], version: 2, savedGeneration: 1, text: 'persisted', isDirty: false }; h.sync(); assert.deepEqual(events, ['change', 'save']);
});

test('ordinary and proxy array getters cannot expand any bounded normalization loop', async () => {
  const h = harness(), collection = h.api.languages.createDiagnosticCollection(), d = h.diagnostic('original'); collection.set(h.uri, [d]);
  const grow = value => { const array = [value]; Object.defineProperty(array, 0, { get() { array.length = 1000000000; return value; } }); return array; };
  assert.throws(() => collection.set(h.uri, grow(d)), /changed during normalization/);
  assert.throws(() => collection.set([[h.uri, grow(d)]]), /changed during normalization/);
  assert.throws(() => collection.set(grow([h.uri, [d]])), /changed during normalization/);
  const related = new h.api.DiagnosticRelatedInformation(new h.api.Location(h.b, new h.api.Range(0, 1, 0, 3)), 'related');
  for (const [property, value] of [['tags', 1], ['relatedInformation', related]]) { const bad = h.diagnostic('bad'); bad[property] = grow(value); assert.throws(() => collection.set(h.uri, [bad]), /changed during normalization/); }
  let reads = 0; const proxy = new Proxy([d], { get(target, key) { if (key === 'length') return reads++ === 0 ? 1 : 1000000000; return Reflect.get(target, key); } }); assert.throws(() => collection.set(h.uri, proxy), /changed during normalization/);
  assert.equal(collection.get(h.uri)[0], d); collection.dispose(); await settle();
});

test('bulk URI accessors are captured once so storage, publication and events agree', async () => {
  const h = harness(), collection = h.api.languages.createDiagnosticCollection(), events = [], d = h.diagnostic('captured');
  h.api.languages.onDidChangeDiagnostics(event => events.push(event));
  let reads = 0; const tuple = [h.uri, [d]];
  Object.defineProperty(tuple, 0, { get() { return reads++ === 0 ? h.uri : h.b; } });
  collection.set([tuple]); await turn(); assert.equal(reads, 1); assert.equal(collection.get(h.uri)[0], d); assert(!collection.has(h.b));
  assert.equal(h.wire('fixture.one').collections[0].entries[0].document, 1);
  await settle(); assert.deepEqual(events[0].uris.map(uri => uri.toString()), [h.uri.toString()]);
  collection.dispose(); await settle();
});
