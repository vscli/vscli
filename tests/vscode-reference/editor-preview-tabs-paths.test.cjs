'use strict';
const assert = require('node:assert/strict');
const path = require('node:path');
const test = require('node:test');
const { fixtureFileResource } = require('./editor-preview-tabs.cjs');

test('Windows canonical file identity accepts drive/root case and separator variants', () => {
  const workspace = 'D:\\a\\vscli\\fixture';
  for (const filename of ['d:\\a\\vscli\\fixture\\a.txt',
    'd:/A/VSCLI/FIXTURE/a.txt', 'D:\\a/VSCLI\\fixture/a.txt']) {
    assert.equal(fixtureFileResource(workspace, filename, path.win32), 'a.txt');
  }
});

test('Windows canonical file identity rejects different roots and nonfixture files', () => {
  const workspace = 'D:\\a\\vscli\\fixture';
  for (const filename of ['D:\\a\\vscli\\other\\a.txt',
    'E:\\a\\vscli\\fixture\\a.txt', 'D:\\a\\vscli\\fixture\\unrelated.txt',
    'D:\\a\\vscli\\fixture\\nested\\a.txt']) {
    assert.throws(() => fixtureFileResource(workspace, filename, path.win32),
      /Editor escaped fixture resource/);
  }
});

test('POSIX canonical file identity remains case-sensitive and confined', () => {
  assert.equal(fixtureFileResource('/tmp/fixture', '/tmp/fixture/b.txt', path.posix), 'b.txt');
  for (const filename of ['/tmp/FIXTURE/b.txt', '/tmp/other/b.txt', '/tmp/fixture/unrelated.txt']) {
    assert.throws(() => fixtureFileResource('/tmp/fixture', filename, path.posix),
      /Editor escaped fixture resource/);
  }
});

const { eventGuard, supplementalRecorder } = require('./editor-preview-tabs.cjs');

test('callback failures are retained and rethrown by explicit snapshot guard', () => {
  const guard = eventGuard(), expected = new Error('strict fixture classification failed');
  let laterCalls = 0;
  assert.doesNotThrow(() => guard.listen(() => { throw expected; })());
  guard.listen(() => { laterCalls++; })();
  assert.equal(laterCalls, 0);
  assert.throws(() => guard.check(), error => error === expected);
});

test('supplemental evidence preserves URI, readiness/target phase and exact Unicode changes', () => {
  const log = supplementalRecorder();
  const first = { kind: 'document-change', phase: { kind: 'readiness' }, fixtureEventCount: 0,
    document: { uri: 'output:extension-host', scheme: 'output', documentObject: 1,
      languageId: 'Log', dirty: false, version: 2 },
    changes: [{ text: '猫🙂\r\n', rangeOffset: 0, rangeLength: 0 }] };
  const second = { kind: 'document-close', phase: { kind: 'target', index: 1 },
    fixtureEventCount: 3, document: first.document };
  log.record(first); log.record(second);
  assert.deepEqual(log.events, [first, second]);
});

test('supplemental evidence limits reject whole events without rewriting prior evidence', () => {
  const log = supplementalRecorder();
  log.record({ kind: 'initial' });
  const prior = JSON.stringify(log.events);
  assert.throws(() => log.record({ text: 'x'.repeat(72 * 1024) }), /byte budget/);
  assert.equal(JSON.stringify(log.events), prior);
  for (let i = 1; i < 256; i++) log.record({ kind: 'document-close' });
  assert.throws(() => log.record({ kind: 'document-close' }), /count budget/);
  assert.equal(log.events.length, 256);
});

test('aggregate supplemental byte limit and callback guard preserve prior evidence', () => {
  const log = supplementalRecorder(), guard = eventGuard();
  const event = { text: 'x'.repeat(70 * 1024) };
  for (let i = 0; i < 7; i++) log.record(event);
  const prior = JSON.stringify(log.events);
  guard.listen(() => log.record(event))();
  assert.throws(() => guard.check(), /byte budget/);
  assert.equal(JSON.stringify(log.events), prior);
});
