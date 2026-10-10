'use strict';
const assert = require('node:assert/strict');
const path = require('node:path');
const test = require('node:test');
const { fixtureFileResource } = require('./editor-sticky-tabs-suite.cjs');

test('Windows canonical sticky resources accept mixed root case and separators', () => {
  const workspace = 'D:\\a\\vscli\\fixture';
  for (const filename of ['d:\\a\\vscli\\fixture\\a.txt',
    'd:/A/VSCLI/FIXTURE/a.txt', 'D:\\a/VSCLI\\fixture/a.txt']) {
    assert.equal(fixtureFileResource(workspace, filename, path.win32), 'a.txt');
  }
  assert.equal(fixtureFileResource('\\server\\share\\fixture',
    '\\SERVER\\SHARE/FiXtUrE/h.txt', path.win32), 'h.txt');
});

test('Windows sticky classification rejects outside and unknown canonical files', () => {
  const workspace = 'D:\\a\\vscli\\fixture';
  for (const filename of ['D:\\a\\vscli\\fixture-other\\a.txt',
    'D:\\a\\vscli\\other\\a.txt', 'E:\\a\\vscli\\fixture\\a.txt',
    'D:\\a\\vscli\\fixture\\unrelated.txt', 'D:\\a\\vscli\\fixture\\nested\\a.txt',
    'D:\\a\\vscli\\fixture\\A.txt', 'D:\\a\\vscli\\fixture']) {
    assert.throws(() => fixtureFileResource(workspace, filename, path.win32),
      /Editor escaped sticky resource inventory/);
  }
  assert.throws(() => fixtureFileResource('\\server\\share\\fixture',
    '\\server\\other/fixture/a.txt', path.win32), /Editor escaped sticky resource inventory/);
});

test('POSIX sticky classification stays case-sensitive and within the exact inventory', () => {
  for (const name of 'abcdefgh') {
    assert.equal(fixtureFileResource('/tmp/fixture', `/tmp/fixture/${name}.txt`, path.posix), `${name}.txt`);
  }
  for (const filename of ['/tmp/FIXTURE/a.txt', '/tmp/fixture/A.txt',
    '/tmp/other/a.txt', '/tmp/fixture-other/a.txt', '/tmp/fixture/nested/a.txt',
    '/tmp/fixture/unrelated.txt', '/tmp/fixture']) {
    assert.throws(() => fixtureFileResource('/tmp/fixture', filename, path.posix),
      /Editor escaped sticky resource inventory/);
  }
});
