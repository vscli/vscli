'use strict';
// Source-derived cases from Microsoft VS Code stable uri.test.ts/uri.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// MIT permission and warranty notice are retained in uri.cjs.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { Uri, createUriForPlatform } = require('./uri.cjs');
const posix = createUriForPlatform(false), win = createUriForPlatform(true);
const components = value => ({ scheme: value.scheme, authority: value.authority, path: value.path,
  query: value.query, fragment: value.fragment });

test('native platform default and isolated branch factories expose the source class API', () => {
  assert.equal(Uri.file('/a/b').fsPath, process.platform === 'win32' ? '\\a\\b' : '/a/b');
  assert.equal(Uri.isUri(Uri.parse('untitled:example')), true);
  assert.equal(posix.isUri(win.file('C:\\a')), true); // Structural cross-module check.
  assert.throws(() => createUriForPlatform('win32'), TypeError);
});

test('file treats relative paths literally without cwd or dot normalization', () => {
  for (const api of [posix, win]) {
    assert.equal(api.file('foo/bar').path, '/foo/bar');
    assert.equal(api.file('./foo/bar').path, '/./foo/bar');
    assert.equal(api.file('foo/../bar').path, '/foo/../bar');
    assert.equal(api.file('foo/bar').toString(), 'file:///foo/bar');
    assert.equal(api.file('').path, '/');
    assert.equal(api.parse('file:foo/bar').path, '/foo/bar');
  }
});

test('file preserves reserved filename characters and platform-specific backslashes', () => {
  for (const api of [posix, win]) {
    const uri = api.file('/folder with space/猫#%?.rs');
    assert.equal(uri.path, '/folder with space/猫#%?.rs');
    assert.equal(uri.query, ''); assert.equal(uri.fragment, '');
    assert.equal(uri.toString(), 'file:///folder%20with%20space/%E7%8C%AB%23%25%3F.rs');
    assert.equal(api.parse(uri.toString()).path, uri.path);
  }
  assert.equal(posix.file('c:\\win\\path').toString(), 'file:///c%3A%5Cwin%5Cpath');
  assert.equal(win.file('C:\\win\\path').toString(), 'file:///c%3A/win/path');
  assert.equal(posix.file('/f\\oo/ba\\r.txt').path, '/f\\oo/ba\\r.txt');
  assert.equal(win.file('/f\\oo/ba\\r.txt').path, '/f/oo/ba/r.txt');
});

test('drive fsPath and trailing separators follow source platform semantics', () => {
  for (const value of ['c:/win/path', 'C:/win/path', '/C:/win/path']) {
    assert.equal(posix.file(value).fsPath, 'c:/win/path');
    assert.equal(win.file(value).fsPath, 'c:\\win\\path');
  }
  assert.equal(posix.file('C:/win/path/').fsPath, 'c:/win/path/');
  assert.equal(win.file('C:/win/path/').fsPath, 'c:\\win\\path\\');
  assert.equal(posix.file('./c/win/path').fsPath, '/./c/win/path');
  assert.equal(win.file('./c/win/path').fsPath, '\\.\\c\\win\\path');
});

test('UNC paths preserve authority separately and normalize separators only on Windows', () => {
  assert.deepEqual(components(win.file('\\\\server\\share\\file.txt')), {
    scheme: 'file', authority: 'server', path: '/share/file.txt', query: '', fragment: '',
  });
  assert.equal(win.file('\\\\server\\share\\file.txt').fsPath, '\\\\server\\share\\file.txt');
  assert.equal(posix.file('//server/share/file.txt').fsPath, '//server/share/file.txt');
  assert.equal(posix.file('\\\\server\\share\\file.txt').authority, '');
  for (const api of [posix, win]) {
    assert.equal(api.file('//server').path, '/');
    assert.equal(api.file('//server').authority, 'server');
    const unusual = api.parse('file://%2Fhome%2Ftest.cpp');
    assert.equal(unusual.authority, '/home/test.cpp');
    assert.equal(unusual.fsPath, api === win ? '\\' : '/');
  }
});

test('parse supports opaque, relative and strict scheme forms without WHATWG normalization', () => {
  assert.deepEqual(components(posix.parse('untitled:α😀')), {
    scheme: 'untitled', authority: '', path: 'α😀', query: '', fragment: '',
  });
  assert.equal(posix.parse('relative/path').toString(), 'file:///relative/path');
  assert.equal(posix.parse('').toString(), 'file:///');
  assert.throws(() => posix.parse('relative/path', true), /Scheme is missing/);
  assert.throws(() => posix.parse('', true), /Scheme is missing/);
  assert.equal(posix.from({ scheme: 'foo+bar-2.test', path: 'p' }, true).toString(), 'foo+bar-2.test:p');
  assert.equal(posix.from({ scheme: '1foo', path: 'p' }).scheme, '1foo');
  assert.equal(posix.parse('http://EXAMPLE.COM:80/a/../b').path, '/a/../b');
  assert.equal(posix.parse('http://EXAMPLE.COM:80/a/../b').toString(), 'http://example.com:80/a/../b');
});

test('from validates source scheme and authority/path rules and preserves literal percent signs', () => {
  assert.throws(() => posix.from({ scheme: '' }, true), /Scheme is missing/);
  assert.equal(posix.from({ scheme: '' }).scheme, 'file');
  for (const scheme of ['fai:l', 'fäil']) assert.throws(() => posix.from({ scheme, path: 'p' }), /illegal characters/);
  assert.throws(() => posix.from({ scheme: 'custom', authority: 'host', path: 'relative' }), /begin with a slash/);
  assert.throws(() => posix.from({ scheme: 'custom', path: '//invalid' }), /two slash/);
  const value = posix.from({ scheme: 'custom', path: '/a%20b', query: 'x=1&y=猫', fragment: 'tag?' });
  assert.equal(value.path, '/a%20b');
  assert.equal(value.toString(), 'custom:/a%2520b?x%3D1%26y%3D%E7%8C%AB#tag%3F');
});

test('parse decodes encoded components gracefully without losing malformed escapes', () => {
  const value = posix.parse('custom://host/a%2Fb/%E7%8C%AB%F0%9F%99%82?x%3D1#t%23g');
  assert.equal(value.path, '/a/b/猫🙂'); assert.equal(value.query, 'x=1'); assert.equal(value.fragment, 't#g');
  assert.equal(posix.parse('file://some/%.txt').toString(), 'file://some/%25.txt');
  assert.equal(posix.parse('file://some/%A0.txt').toString(), 'file://some/%25A0.txt');
  assert.equal(posix.parse('custom:/%ZZ%20valid').path, '/%ZZ valid');
  for (const filename of ['/foo/%A0.txt', '/foo/%2e.txt', '/foo/猫🙂']) {
    const uri = posix.file(filename);
    assert.equal(posix.parse(uri.toString()).path, filename);
  }
});

test('authority formatting handles userinfo, Unicode and IPv6 without punycode or default-port removal', () => {
  const uri = posix.from({ scheme: 'http', authority: 'föö:bör@LÖCALHOST:80', path: '/far' });
  assert.equal(uri.authority, 'föö:bör@LÖCALHOST:80');
  assert.equal(uri.toString(), 'http://f%C3%B6%C3%B6:b%C3%B6r@l%C3%B6calhost:80/far');
  assert.equal(posix.parse('http://user@[FEDC:BA98::1]:80/index').toString(), 'http://user@[fedc:ba98::1]:80/index');
  assert.equal(posix.parse('http://us[er@[FEDC:BA98::1]:80/index').toString(), 'http://us%5Ber@[fedc:ba98::1]:80/index');
});

test('HTTP drive-looking paths retain uppercase while file and opaque drives format lowercase', () => {
  for (const scheme of ['http', 'https', 'HTTP', 'HTTPS']) {
    assert.equal(posix.from({ scheme, path: '/C:/Case' }).toString(), `${scheme}:/C%3A/Case`);
  }
  assert.equal(posix.from({ scheme: 'file', path: '/C:/Case' }).toString(), 'file:///c%3A/Case');
  assert.equal(posix.from({ scheme: 'custom', path: 'C:/Case' }).toString(), 'custom:c%3A/Case');
  assert.equal(posix.from({ scheme: 'http', path: '/C:/Case' }).fsPath, 'c:/Case'); // fsPath ignores scheme.
});

test('minimal encoding retains delimiters and Unicode without decoding formatted percent escapes', () => {
  const cases = [
    ['', ''], ['plain/path', 'plain/path'], ['?#', '%3F%23'], ['#?', '%23%3F'],
    ['before?between#after', 'before%3Fbetween%23after'],
    ['%23/%3F?raw#', '%23/%3F%3Fraw%23'],
    ['é/🐱?終#', 'é/🐱%3F終%23'], ['\ud800x?\udc00y#\ud800', '\ud800x%3F\udc00y%23\ud800'],
  ];
  for (const [path, expected] of cases) assert.equal(posix.from({ scheme: 'test', path }).toString(true), `test:${expected}`);
  const value = posix.from({ scheme: 'foo', authority: 'us?er:pa#ss@EXAMPLE.com:8080',
    path: '/path#before?after', query: 'query#part?more', fragment: 'fragment?#' });
  const minimal = 'foo://us%3Fer:pa%23ss@example.com:8080/path%23before%3Fafter?query%23part%3Fmore#fragment?#';
  const normal = 'foo://us%3Fer:pa%23ss@example.com:8080/path%23before%3Fafter?query%23part%3Fmore#fragment%3F%23';
  assert.equal(value.toString(true), minimal); assert.equal(value.toString(), normal); assert.equal(value.toString(true), minimal);
});

test('with retains unchanged identity, clears nullable components and validates changed combinations', () => {
  const value = posix.parse('custom://host/path?query#fragment');
  for (const change of [null, undefined, {}, components(value)]) assert.equal(value.with(change), value);
  assert.equal(value.with({ query: undefined }), value);
  assert.equal(value.with({ query: null, fragment: null }).toString(), 'custom://host/path');
  assert.equal(value.with({ authority: null }).toString(), 'custom:/path?query#fragment');
  assert.equal(value.with({ path: null }).toString(), 'custom://host?query#fragment');
  assert.equal(value.with({ scheme: null }).scheme, 'file');
  assert.equal(posix.file('/old').with({ path: null }).path, '/');
  assert.throws(() => posix.parse('foo:relative').with({ authority: 'host' }), /begin with a slash/);
  assert.throws(() => value.with({ scheme: 'bad:scheme' }), /illegal characters/);
  assert.throws(() => value.with({ authority: null, path: '//bad' }), /two slash/);
  assert.equal(value.toString(), 'custom://host/path?query#fragment');
});

test('joinPath common cases normalize dot segments while preserving query and fragment', () => {
  for (const api of [posix, win]) {
    for (const [base, fragment, expected] of [
      ['file:///foo/bar/', './bazz', 'file:///foo/bar/bazz'],
      ['file:///foo/bar', 'bazz', 'file:///foo/bar/bazz'],
      ['file:', 'bazz', 'file:///bazz'], ['http://domain', 'bazz', 'http://domain/bazz'],
      ['foo:/', 'bazz', 'foo:/bazz'], ['foo://bar/', 'bazz', 'foo://bar/bazz'],
    ]) assert.equal(api.joinPath(api.parse(base), fragment).toString(true), expected);
    const base = api.parse('custom:/a/b?q#f');
    const joined = api.joinPath(base, '..', '猫#%.rs');
    assert.equal(joined.path, '/a/猫#%.rs'); assert.equal(joined.query, 'q'); assert.equal(joined.fragment, 'f');
    assert.equal(base.path, '/a/b');
    const unchanged = api.parse('custom:/a');
    assert.equal(api.joinPath(unchanged), unchanged);
    assert.throws(() => api.joinPath(api.parse('foo:'), 'b'), /without path/);
    assert.throws(() => api.joinPath(api.parse('foo://bar'), 'b'), /without path/);
    assert.throws(() => api.joinPath(base, undefined), error => error.code === 'ERR_INVALID_ARG_TYPE');
  }
});

test('joinPath Windows drive/UNC roots differ intentionally from POSIX roots', () => {
  assert.equal(posix.joinPath(posix.parse('file:///c:/foo/'), '../../bazz').toString(true), 'file:///bazz');
  assert.equal(win.joinPath(win.parse('file:///c:/foo/'), '../../bazz').toString(true), 'file:///c:/bazz');
  assert.equal(posix.joinPath(posix.parse('file://server/share/c:/'), '../../bazz').toString(true), 'file://server/bazz');
  assert.equal(win.joinPath(win.parse('file://server/share/c:/'), '../../bazz').toString(true), 'file://server/share/bazz');
  assert.equal(win.joinPath(win.parse('file://ser/foo'), '../../bazz').toString(true), 'file://ser/foo/bazz');
  assert.equal(win.joinPath(win.parse('file:///c:/foo/bar'), './other/foo.img').toString(true), 'file:///c:/foo/bar/other/foo.img');
  assert.equal(win.joinPath(win.parse('file:///C:/base'), 'child').path, '/C:/base/child'); // Preserve drive casing before formatting.
  assert.equal(win.joinPath(win.parse('remote:/C:/foo/'), '../../bazz').path, '/bazz'); // Non-file uses POSIX join.
});

test('JSON exposes source marshalled marker and only nonempty components before caches populate', () => {
  assert.deepEqual(posix.parse('untitled:').toJSON(), { $mid: 1, scheme: 'untitled' });
  const value = posix.parse('file:///a%20b');
  assert.deepEqual(value.toJSON(), { $mid: 1, path: '/a b', scheme: 'file' });
  assert.equal(value.toString(), 'file:///a%20b');
  assert.deepEqual(value.toJSON(), { $mid: 1, external: 'file:///a%20b', path: '/a b', scheme: 'file' });
  assert.equal(value.fsPath, '/a b');
  assert.deepEqual(value.toJSON(), { $mid: 1, fsPath: '/a b', _sep: undefined, external: 'file:///a%20b', path: '/a b', scheme: 'file' });
  const changed = value.with({ path: '/new' });
  assert.deepEqual(changed.toJSON(), { $mid: 1, path: '/new', scheme: 'file' });
});

test('revive preserves URI identity and same-platform caches without validating trusted components', () => {
  assert.equal(posix.revive(null), null); assert.equal(posix.revive(undefined), undefined);
  const value = posix.parse('custom:/source'); assert.equal(posix.revive(value), value);
  assert.equal(posix.from({ scheme: '' }).scheme, 'file');
  assert.equal(posix.revive({ scheme: '' }).scheme, '');
  const file = win.file('C:\\a\\b'); file.toString(); file.fsPath;
  const state = JSON.parse(JSON.stringify(file));
  assert.equal(state._sep, 1);
  assert.equal(win.revive(state).fsPath, 'c:\\a\\b');
  assert.equal(posix.revive(state).fsPath, 'c:/a/b'); // Reject foreign separator cache.
  assert.equal(posix.revive(state).toString(), state.external);
  assert.equal(posix.revive({ scheme: 'custom', path: '//trusted' }).path, '//trusted');
  assert.throws(() => posix.from({ scheme: 'custom', path: '//trusted' }), /two slash/);
});

test('isUri uses exact source structural shape, distinct from serialized components', () => {
  const value = posix.file('/foo');
  assert.equal(posix.isUri(value), true); assert.equal(posix.isUri(value.toJSON()), false);
  assert.equal(posix.isUri({ scheme: 'file', authority: '', path: '/foo', query: '', fragment: '',
    fsPath: '/foo', with() { return this; }, toString() { return ''; } }), true);
  for (const invalid of [undefined, null, 0, 'file:///foo', {}, { scheme: 'file', path: '/foo' }]) {
    assert.equal(posix.isUri(invalid), false);
  }
});

test('normal serialization/revive remains idempotent across literal Unicode/reserved filename corpus', () => {
  for (const api of [posix, win]) {
    for (const path of ['/猫🙂/space here', '/a#b?c%d', "/apostrophe's/(x)[y]!$&+;=", '/a/../b', '/%ZZ/%A0']) {
      const source = api.from({ scheme: 'custom', path, query: 'x=1&y=猫', fragment: 'tag#' });
      const encoded = source.toString();
      assert.equal(api.parse(encoded).toString(), encoded);
      const revived = api.revive(JSON.parse(JSON.stringify(source)));
      assert.deepEqual(components(revived), components(source));
      assert.equal(revived.toString(), encoded);
    }
  }
});
