'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createConfiguration } = require('./configuration.cjs');
const { Uri } = require('./api-types.cjs');
const schema = { properties: {
  'example.enabled': { type: 'boolean', default: false },
  'example.object': { default: { left: 1, nested: { a: 1 }, list: [1, 2] } },
  'example.null': { default: null },
  'example.language': { default: 1, scope: 'language-overridable' },
  'example.machine': { default: 'default', scope: 'machine' },
} };
function configured(layers = []) {
  const configuration = createConfiguration();
  configuration.initialize(schema, layers);
  return configuration;
}

test('user/workspace precedence recursively merges objects, replaces arrays and preserves null', () => {
  const configuration = configured([
    { 'example.enabled': true, 'example.object': { nested: { b: 2 }, list: [3] }, 'example.machine': 'user' },
    { 'example.object': { nested: { a: 4 } }, 'example.machine': 'workspace' },
  ]);
  const value = configuration.get('example');
  assert.equal(value.get('enabled'), true);
  assert.deepEqual(value.get('object'), { left: 1, nested: { a: 4, b: 2 }, list: [3] });
  assert.equal(value.get('null', 'fallback'), null);
  assert.equal(value.get('machine'), 'user');
  assert.equal(value.get('missing', 'fallback'), 'fallback');
  assert.equal(value.has(''), false);
  assert.equal(value.get(''), undefined);
  assert.equal(value.has('missing'), false);
  assert.equal(value.has('null'), true);
  assert.equal(value.inspect('machine').workspaceValue, 'workspace'); // Inspect includes the raw ignored scope.
  assert.equal(value.inspect('missing').defaultValue, undefined);
  assert.equal(value.inspect('enabled').languageIds, undefined);
});

test('configuration reads are snapshots while inspect and fresh event reads see current values', () => {
  const configuration = configured([{}, { 'example.enabled': true }]);
  const snapshot = configuration.get('example');
  let event, observed;
  configuration.onDidChange(change => { event = change; observed = configuration.get('example').get('enabled'); });
  configuration.update([{}, { 'example.enabled': false }]);
  assert.equal(snapshot.get('enabled'), true);
  assert.equal(snapshot.inspect('enabled').workspaceValue, false);
  assert.equal(observed, false);
  assert.equal(event.affectsConfiguration('example'), true);
  assert.equal(event.affectsConfiguration('exam'), false);
  assert.equal(event.affectsConfiguration('example.enabled.child'), false);
  configuration.update([{}, { 'example.enabled': true }]);
  assert.equal(event.affectsConfiguration('example.enabled', null), true);
});

test('unscoped events include shadowed changes while resource queries compare effective values', () => {
  const configuration = configured([{ 'example.enabled': false }, { 'example.enabled': true }]);
  let event, count = 0;
  const subscription = configuration.onDidChange(change => { event = change; count++; });
  configuration.update([{ 'example.enabled': true }, { 'example.enabled': true }]);
  assert.equal(event.affectsConfiguration('example.enabled'), true);
  assert.equal(event.affectsConfiguration('example.enabled', Uri.file('/workspace/a.txt')), false);
  assert.equal(event.affectsConfiguration('example.enabled', null), false);
  const retained = event;
  configuration.update([{}, { 'example.enabled': false }]);
  assert.equal(retained.affectsConfiguration('example.enabled', null), false);
  configuration.update([{}, { 'example.enabled': false }]);
  assert.equal(count, 2);
  subscription.dispose();
  configuration.update([]);
  assert.equal(count, 2);
});

test('single-language overrides win over combined blocks; URI alone does not infer language', () => {
  const configuration = configured([
    { '[ javascript ]': { 'example.language': 2 } },
    { 'example.language': 3, '[javascript][typescript]': { 'example.language': 4 } },
  ]);
  assert.equal(configuration.get('example').get('language'), 3);
  assert.equal(configuration.get('example', Uri.file('/workspace/a.js')).get('language'), 3);
  const scoped = configuration.get('example', { uri: Uri.file('/workspace/a.js'), languageId: 'javascript' });
  assert.equal(scoped.get('language'), 2);
  assert.equal(configuration.get('example', { languageId: 'typescript' }).get('language'), 4);
  assert.deepEqual(scoped.inspect('language'), { key: 'example.language', defaultValue: 1,
    globalValue: undefined, workspaceValue: 3, workspaceFolderValue: undefined,
    defaultLanguageValue: undefined, globalLanguageValue: 2, workspaceLanguageValue: 4,
    workspaceFolderLanguageValue: undefined, languageIds: ['javascript', 'typescript'] });
});

test('combined-language groups keep their first insertion position across scopes', () => {
  const configuration = configured([
    { '[javascript][typescript]': { 'example.language': 2 }, '[javascript][python]': { 'example.language': 3 } },
    { '[javascript][typescript]': { 'example.language': 4 } },
  ]);
  assert.equal(configuration.get('example', { languageId: 'javascript' }).get('language'), 3);
  let event;
  configuration.onDidChange(change => { event = change; });
  configuration.update([
    { '[javascript][python]': { 'example.language': 3 }, '[javascript][typescript]': { 'example.language': 2 } },
    { '[javascript][typescript]': { 'example.language': 4 } },
  ]);
  assert.equal(configuration.get('example', { languageId: 'javascript' }).get('language'), 4);
  assert.equal(event.affectsConfiguration('example.language'), true);
  assert.equal(event.affectsConfiguration('example.language', { languageId: 'javascript' }), true);
  assert.equal(event.affectsConfiguration('example.language', { languageId: 'typescript' }), false);
});

test('held configurations inspect current values using their original language scope', () => {
  const configuration = configured([{}, {
    '[python]': { 'example.language': 2 }, '[javascript]': { 'example.language': 3 },
  }]);
  const scope = { languageId: 'python' };
  const held = configuration.get('example', scope);
  scope.languageId = 'javascript';
  configuration.update([{}, {
    '[python]': { 'example.language': 4 }, '[javascript]': { 'example.language': 5 },
  }]);
  assert.equal(held.get('language'), 2);
  assert.equal(held.inspect('language').workspaceLanguageValue, 4);
  assert.equal(configuration.get('example', scope).get('language'), 5);
});

test('language-only changes are observable and events retain both original snapshots', () => {
  const configuration = configured([{}, { '[python]': { 'example.language': 8 } }]);
  let event;
  configuration.onDidChange(change => { event = change; });
  configuration.update([{}, { '[python]': { 'example.language': 9 } }]);
  assert.equal(event.affectsConfiguration('example.language'), true);
  assert.equal(event.affectsConfiguration('example.language', Uri.file('/workspace/a.py')), false);
  assert.equal(event.affectsConfiguration('example.language', { languageId: 'python' }), true);
  const retained = event;
  configuration.update([{}, { '[python]': { 'example.language': 8 } }]);
  assert.equal(retained.affectsConfiguration('example.language', { languageId: 'python' }), true);
});

test('defaults derive from schema types; reads and inspect values cannot mutate stored configuration', async () => {
  const configuration = configured();
  const snapshot = configuration.get('example');
  snapshot.get('object').nested.a = 500;
  snapshot.inspect('object').defaultValue.nested.a = 600;
  assert.equal(snapshot.get('object').nested.a, 1);
  assert.throws(() => { snapshot.object.nested.a = 100; }, TypeError);
  await assert.rejects(snapshot.update('enabled', true), /writes are not implemented/);
  configuration.initialize({ properties: { 'types.string': { type: 'string' }, 'types.object': { type: 'object' },
    'types.array': { type: 'array' }, 'types.number': { type: ['number', 'null'] }, 'types.unknown': {} } });
  assert.deepEqual(configuration.get().get('types'), { string: '', object: {}, array: [], number: 0, unknown: null });
});

test('prototype-like setting names remain inert data through merging and dotted lookup', () => {
  const configuration = configured([{}, JSON.parse('{"example.object":{"__proto__":{"polluted":true}},"constructor.prototype.value":5}')]);
  assert.equal({}.polluted, undefined);
  assert.equal(configuration.get('example').get('object').__proto__.polluted, true);
  assert.equal(configuration.get('constructor').get('prototype.value'), 5);
  assert.equal({}.value, undefined);
});
