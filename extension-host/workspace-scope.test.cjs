'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { createUriForPlatform } = require('./uri.cjs');
const { EventEmitter } = require('./api-types.cjs');
const { createWorkspaceScope, MAX_FOLDERS, MAX_URI_BYTES, MAX_NAME_BYTES, MAX_TOTAL_BYTES, MAX_METADATA_BYTES } = require('./workspace-scope.cjs');

function setup(platform = 'linux', pathCaseSensitive = {}) {
  const Uri = createUriForPlatform(platform === 'win32');
  const scope = createWorkspaceScope({ Uri, EventEmitter, platform, pathCaseSensitive });
  const folders = (...rows) => rows.map(([uri, name]) => ({ uri: typeof uri === 'string' ? Uri.file(uri) : uri, name }));
  const publish = (generation, rows, extra = {}, trusted = false) => scope.publish({
    generation, trusted, workspace: { name: 'Native workspace', folders: folders(...rows), ...extra },
  });
  return { Uri, scope, api: scope.api, folders, publish };
}
function uriParts(uri, overrides = {}) {
  return { scheme: uri.scheme, authority: uri.authority, path: uri.path, query: uri.query, fragment: uri.fragment, ...overrides };
}

test('no workspace, opened empty workspace, and single folder metadata are distinct', () => {
  const { scope, api, Uri, publish } = setup();
  const events = [];
  api.onDidChangeWorkspaceFolders(value => events.push(value));
  assert.equal(api.workspaceFolders, undefined);
  assert.equal(api.name, undefined);
  assert.equal(api.rootPath, undefined);
  assert.equal(api.workspaceFile, undefined);
  assert.equal(api.isTrusted, false);
  assert.equal(api.asRelativePath(undefined), '');
  scope.publish({ generation: 0, trusted: false, workspace: null });
  assert.equal(events.length, 0, 'initial authoritative baseline is quiet');
  publish(1, []);
  assert.deepEqual(api.workspaceFolders, []);
  assert.equal(api.name, 'Native workspace');
  publish(2, [['/work/猫', 'Project']]);
  assert.equal(api.rootPath, '/work/猫');
  const folder = api.workspaceFolders[0];
  assert.equal(folder.index, 0);
  assert.equal(folder.name, 'Project');
  assert.strictEqual(api.getWorkspaceFolder(Uri.file('/work/猫')), folder);
  assert.equal(api.asRelativePath(Uri.file('/work/猫/a.cpp')), 'a.cpp');
  assert.equal(api.asRelativePath(Uri.file('/work/猫')), '/work/猫', 'top-level folder itself resolves its parent');
  const detached = api.workspaceFolders;
  detached.length = 0;
  assert.equal(api.workspaceFolders.length, 1);
  scope.dispose();
});

test('lookup uses literal path segments, deepest prefix, and ignores query/fragment', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/work', 'Outer'], ['/work/pkg', 'Inner'], ['/work/../literal', 'Dots']]);
  assert.equal(api.getWorkspaceFolder(Uri.file('/work/pkg/a')).name, 'Inner');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'file', path: '/work//pkg///', query: 'q=1', fragment: 'x' })).name, 'Inner');
  assert.equal(api.getWorkspaceFolder(Uri.file('/workspace/a')), undefined);
  assert.equal(api.getWorkspaceFolder(Uri.file('/work/../literal/a')).name, 'Dots');
  assert.equal(api.getWorkspaceFolder(Uri.file('/literal/a')), undefined, 'lookup never resolves dot segments or canonicalizes files');
  publish(1, [[Uri.from({ scheme: 'remote', path: '' }), 'Root'], [Uri.from({ scheme: 'remote', path: '.' }), 'Dot']]);
  assert.equal(api.asRelativePath(Uri.from({ scheme: 'remote', path: '' })), 'Root/', 'dirname of an empty URI path preserves the resource');
  scope.dispose();
});

test('multi-root relative paths default to folder names and exact nested roots use the parent', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/work', 'Outer'], ['/work/nested', 'Inner'], ['/other', 'Other']]);
  assert.equal(api.asRelativePath(Uri.file('/work/nested/猫.cpp')), 'Inner/猫.cpp');
  assert.equal(api.asRelativePath(Uri.file('/work/nested/猫.cpp'), false), '猫.cpp');
  assert.equal(api.asRelativePath(Uri.file('/work/nested/猫.cpp'), true), 'Inner/猫.cpp');
  assert.equal(api.asRelativePath(Uri.file('/work/nested')), 'Outer/nested');
  assert.equal(api.asRelativePath('/outside/猫.cpp'), '/outside/猫.cpp');
  assert.equal(api.asRelativePath(Uri.file('/outside/猫.cpp')), '/outside/猫.cpp');
  assert.equal(api.rootPath, '/work', 'deprecated rootPath still returns the first folder in multi-root workspaces');
  scope.dispose();
});

test('relative string inputs use URI.file, never process cwd resolution', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/in', 'In']]);
  assert.equal(Uri.file('in/猫.cpp').path, '/in/猫.cpp');
  assert.equal(api.asRelativePath('in/猫.cpp'), '猫.cpp');
  assert.equal(api.asRelativePath('out/猫.cpp'), 'out/猫.cpp');
  assert.equal(api.getWorkspaceFolder(Uri.file('in/../out/猫.cpp')).name, 'In', 'lookup preserves literal dots');
  assert.equal(api.asRelativePath('in/../out/猫.cpp'), '../out/猫.cpp', 'relative-path math independently normalizes its result');
  scope.dispose();
});

test('Linux, macOS, and explicit file-provider path case capability control lookup', () => {
  for (const [platform, capability, matches] of [
    ['linux', {}, false], ['darwin', {}, true], ['linux', { file: false }, true], ['darwin', { file: true }, false],
  ]) {
    const { Uri, api, publish, scope } = setup(platform, capability);
    publish(0, [['/Work', 'Project']]);
    assert.equal(Boolean(api.getWorkspaceFolder(Uri.file('/work/file'))), matches);
    scope.dispose();
  }
});

test('Windows drive and UNC lookup preserves public URI identity and slash-relative output', () => {
  const { Uri, api, publish, scope } = setup('win32');
  publish(0, [['C:\\Work', 'Drive'], ['\\\\Server\\Share\\Project', 'UNC']]);
  assert.equal(api.rootPath, 'c:\\Work');
  assert.equal(api.getWorkspaceFolder(Uri.file('c:\\work\\猫.cpp')).name, 'Drive');
  assert.equal(api.asRelativePath('C:\\Work\\sub\\猫.cpp'), 'Drive/sub/猫.cpp');
  assert.equal(api.getWorkspaceFolder(Uri.file('\\\\server\\share\\project\\file')).name, 'UNC');
  assert.equal(api.getWorkspaceFolder(Uri.file('\\\\other\\Share\\Project\\file')), undefined);
  assert.equal(api.asRelativePath('D:\\outside\\file'), 'D:\\outside\\file');
  scope.dispose();
});

test('remote authority matches case-insensitively while capability and relative math stay distinct', () => {
  const { Uri, api, publish, scope } = setup('linux', { exact: true });
  publish(0, [[Uri.from({ scheme: 'remote', authority: 'HOST', path: '/Root' }), 'Remote'],
    [Uri.from({ scheme: 'exact', authority: 'host', path: '/Root' }), 'Exact']]);
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', authority: 'host', path: '/ROOT/file' })).name, 'Remote');
  assert.equal(api.asRelativePath(Uri.from({ scheme: 'remote', authority: 'host', path: '/ROOT/file' })), 'Remote/../ROOT/file');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', authority: 'other', path: '/Root/file' })), undefined);
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'exact', authority: 'HOST', path: '/root/file' })), undefined);
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'exact', authority: 'HOST', path: '/Root/file' })).name, 'Exact');
  scope.dispose();
});

test('later equal lookup key wins without discarding distinct query-bearing folder metadata', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [[Uri.from({ scheme: 'file', path: '/work', query: 'one' }), 'First'],
    [Uri.from({ scheme: 'file', path: '/work/', query: 'two' }), 'Last']]);
  assert.equal(api.workspaceFolders.length, 2);
  assert.equal(api.getWorkspaceFolder(Uri.file('/work/file')).name, 'Last');
  assert.notStrictEqual(api.workspaceFolders[0], api.workspaceFolders[1]);
  scope.dispose();
});

test('retained folder objects reflect rename and reorder; events are URI-only sorted deltas', () => {
  const { api, publish, scope } = setup();
  publish(0, [['/b', 'B'], ['/a', 'A']]);
  const [b, a] = api.workspaceFolders, events = [];
  api.onDidChangeWorkspaceFolders(value => events.push(value));
  publish(1, [['/a', 'A renamed'], ['/b', 'B renamed']]);
  assert.strictEqual(api.workspaceFolders[0], a);
  assert.strictEqual(api.workspaceFolders[1], b);
  assert.equal(a.name, 'A renamed'); assert.equal(a.index, 0);
  assert.equal(b.name, 'B renamed'); assert.equal(b.index, 1);
  assert.deepEqual(events[0], { added: [], removed: [] });
  assert(Object.isFrozen(events[0]));
  assert(!Object.isFrozen(events[0].added));
  publish(2, [['/z', 'Z'], ['/c', 'C']]);
  assert.deepEqual(events[1].added.map(folder => folder.name), ['C', 'Z']);
  assert.deepEqual(events[1].removed.map(folder => folder.name), ['A renamed', 'B renamed']);
  events[1].added.length = 0;
  assert.deepEqual(api.workspaceFolders.map(folder => folder.name), ['Z', 'C']);
  assert.throws(() => { a.name = 'Extension overwrite'; }, TypeError, 'native readonly backing protects publication from extension mutation');
  scope.dispose();
});

test('saved and untitled workspace configuration metadata use source-derived URIs', () => {
  const { Uri, api, publish, scope } = setup();
  const configuration = Uri.file('/storage/Workspace-ID/workspace.json');
  publish(0, [['/a', 'A']], { configuration });
  assert.equal(api.workspaceFile.toString(), configuration.toString());
  publish(1, [['/a', 'A']], { configuration, isUntitled: true });
  assert.equal(api.workspaceFile.scheme, 'untitled');
  assert.equal(api.workspaceFile.path, 'Workspace-ID');
  publish(2, [['/a', 'A']]);
  assert.equal(api.workspaceFile, undefined, 'single-folder native workspace has no saved workspaceFile');
  scope.dispose();
});

test('authoritative trust baseline is quiet and later grants fire once with undefined payload', () => {
  const { api, publish, scope } = setup();
  const grants = [];
  api.onDidGrantWorkspaceTrust(value => grants.push(value));
  publish(0, [['/a', 'A']], {}, false);
  publish(1, [['/a', 'A']], {}, true);
  publish(2, [['/a', 'A']], {}, true);
  assert.equal(api.isTrusted, true);
  assert.deepEqual(grants, [undefined]);
  scope.dispose();
  const already = setup();
  let initialGrants = 0;
  already.api.onDidGrantWorkspaceTrust(() => initialGrants++);
  already.publish(0, [], {}, true);
  assert.equal(already.api.isTrusted, true);
  assert.equal(initialGrants, 0);
  already.scope.dispose();
});

test('workspace update is explicitly backend-gated and fabricates neither success nor events', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/a', 'A']]);
  const original = api.workspaceFolders[0], events = [];
  api.onDidChangeWorkspaceFolders(value => events.push(value));
  assert.equal(api.updateWorkspaceFolders(0, 1, { uri: Uri.file('/b'), name: 'B' }), false);
  assert.equal(api.updateWorkspaceFolders(-1, 99), false);
  assert.strictEqual(api.workspaceFolders[0], original);
  assert.deepEqual(events, []);
  assert(Object.isFrozen(api));
  assert.throws(() => { api.onDidChangeWorkspaceFolders = () => {}; }, TypeError);
  scope.dispose();
});

test('null workspace source branch clears inventory with empty change-event arrays', () => {
  const { api, publish, scope } = setup();
  publish(0, [['/a', 'A']]);
  const events = [];
  api.onDidChangeWorkspaceFolders(event => events.push(event));
  scope.publish({ generation: 1, trusted: false, workspace: null });
  assert.equal(api.workspaceFolders, undefined);
  assert.equal(api.name, undefined);
  assert.deepEqual(events, [{ added: [], removed: [] }]);
  scope.dispose();
});

test('stale publication is ignored and reentrant normalization cannot overwrite newer metadata', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/a', 'A']]);
  assert.equal(publish(0, [['/x', 'Stale']]), false);
  const folders = [{ get uri() { publish(2, [['/b', 'New']]); return Uri.file('/c'); }, name: 'Retired' }];
  assert.throws(() => scope.publish({ generation: 1, trusted: false, workspace: { name: 'Old', folders } }), /became stale/);
  assert.equal(scope.generation, 2);
  assert.equal(api.workspaceFolders[0].name, 'New');
  scope.dispose();
});

test('publication bounds and dynamically growing arrays refuse atomically before live bindings change', () => {
  const { Uri, api, publish, scope } = setup();
  publish(0, [['/a', 'A']]);
  const original = api.workspaceFolders[0];
  const assertPreserved = () => {
    assert.equal(scope.generation, 0); assert.strictEqual(api.workspaceFolders[0], original);
    assert.equal(original.name, 'A'); assert.equal(original.index, 0);
  };
  assert.throws(() => publish(1, Array.from({ length: MAX_FOLDERS + 1 }, () => ['/a', 'A'])), /budget/);
  assertPreserved();
  assert.throws(() => publish(1, [['/a', 'Renamed'], ['/b', 'x'.repeat(MAX_NAME_BYTES + 1)]]), /bounded/);
  assertPreserved();
  assert.throws(() => publish(1, [[Uri.from({ scheme: 'file', path: '/' + 'a'.repeat(MAX_URI_BYTES) }), 'A']]), /aggregate|bounded/);
  assertPreserved();
  const growing = [{ name: 'A', get uri() { growing.push({ uri: Uri.file('/b'), name: 'B' }); return Uri.file('/a'); } }];
  assert.throws(() => scope.publish({ generation: 1, trusted: false, workspace: { name: 'Growth', folders: growing } }), /changed during/);
  assertPreserved();
  const wideRows = Array.from({ length: MAX_FOLDERS }, (_, i) => [Uri.file('/' + i + 'x'.repeat(7000)), 'X']);
  assert.throws(() => publish(1, wideRows), /aggregate metadata/);
  assertPreserved();
  assert.equal(MAX_TOTAL_BYTES, 512 * 1024); assert.equal(MAX_METADATA_BYTES, 2 * 1024 * 1024);
  scope.dispose();
});

test('URI properties and folder URI are each captured once, including adversarial accessors', () => {
  const { Uri, api, scope } = setup();
  const original = Uri.file('/a'), reads = new Map(), parts = uriParts(original), resource = {};
  for (const [name, value] of Object.entries(parts)) Object.defineProperty(resource, name, {
    get() { reads.set(name, (reads.get(name) || 0) + 1); return value; },
  });
  let uriReads = 0;
  scope.publish({ generation: 0, trusted: false, workspace: { name: 'Once', folders: [{ get uri() { uriReads++; return resource; }, name: 'A' }] } });
  assert.equal(uriReads, 1);
  assert.deepEqual([...reads.values()], [1, 1, 1, 1, 1]);
  assert.equal(api.getWorkspaceFolder(Uri.file('/a/b')).name, 'A');
  scope.dispose();
});

test('listener disposal and scope disposal suppress further delivery and retired trust grants', () => {
  const { api, publish, scope } = setup();
  publish(0, [['/a', 'A']]);
  let changes = 0, grants = 0;
  const listener = api.onDidChangeWorkspaceFolders(() => changes++);
  listener.dispose();
  api.onDidGrantWorkspaceTrust(() => grants++);
  api.onDidChangeWorkspaceFolders(() => scope.dispose());
  publish(1, [['/b', 'B']], {}, true);
  assert.equal(changes, 0); assert.equal(grants, 0);
  assert.throws(() => publish(2, []), /disposed/);
  api.onDidChangeWorkspaceFolders(() => changes++);
  assert.equal(changes, 0);
});

test('invalid provider capabilities and malformed publication input are bounded refusals', () => {
  const Uri = createUriForPlatform(false);
  assert.throws(() => createWorkspaceScope({ Uri, EventEmitter, pathCaseSensitive: { file: 'false' } }), /boolean/);
  assert.throws(() => createWorkspaceScope({ Uri, EventEmitter, pathCaseSensitive: Object.fromEntries(Array.from({ length: 129 }, (_, i) => ['s' + i, false])) }), /budget/);
  const { scope } = setup();
  assert.throws(() => scope.publish({ generation: -1, trusted: false, workspace: null }), /Invalid/);
  assert.throws(() => scope.publish({ generation: 0, trusted: 'true', workspace: null }), /Invalid/);
  assert.throws(() => scope.publish({ generation: 0, trusted: false, workspace: { name: 'Bad', folders: {} } }), /budget/);
  assert.equal(scope.generation, -1);
  scope.dispose();
});


test('live root filesystem capabilities are captured once per URI and never per path segment', () => {
  const Uri = createUriForPlatform(false);
  let sensitive, reads = 0;
  const scope = createWorkspaceScope({ Uri, EventEmitter, platform: 'linux', pathCaseSensitive: scheme => { reads++; return scheme === 'remote' ? sensitive : undefined; } });
  scope.publish({ generation: 0, trusted: false, workspace: { name: 'Remote', folders: [{ uri: Uri.from({ scheme: 'remote', authority: 'host', path: '/Root' }), name: 'R' }] } });
  const query = Uri.from({ scheme: 'remote', authority: 'host', path: '/root/a/b/c' });
  reads = 0;
  assert.equal(scope.api.getWorkspaceFolder(query).name, 'R');
  assert.equal(reads, 1);
  sensitive = true; reads = 0;
  assert.equal(scope.api.getWorkspaceFolder(query), undefined);
  assert.equal(reads, 1);
  sensitive = false;
  assert.equal(scope.api.getWorkspaceFolder(query).name, 'R', 'capability A-B-A is observed independently of cached metadata');
  const retained = scope.api.workspaceFolders[0];
  sensitive = true;
  scope.publish({ generation: 1, trusted: false, workspace: { name: 'Renamed', folders: [{ uri: Uri.from({ scheme: 'remote', authority: 'host', path: '/Root' }), name: 'Renamed' }] } });
  assert.strictEqual(scope.api.workspaceFolders[0], retained, 'live equality is recomputed when capabilities change');
  assert.equal(retained.name, 'Renamed');
  sensitive = 'malformed';
  assert.throws(() => scope.api.getWorkspaceFolder(query), /boolean/);
  assert.equal(scope.api.workspaceFolders.length, 1);
  scope.dispose();
});


test('case-insensitive path lookup preserves pinned UTF-16 substring comparison bounds', () => {
  const { Uri, publish, api, scope } = setup();
  publish(0, [[Uri.from({ scheme: 'remote', path: '/i' }), 'Outer'],
    [Uri.from({ scheme: 'remote', path: '/i/x' }), 'Inner'],
    [Uri.from({ scheme: 'remote', path: '/i/é' }), 'Accent']]);
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/İ' })).name, 'Outer');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/İ/x' })).name, 'Inner',
    'equal original x characters bypass the lower-case fallback entirely');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/İ/É' })).name, 'Outer',
    'different non-ASCII second-segment characters trigger fallback at original UTF-16 bounds');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/i/É' })).name, 'Accent',
    'without an earlier expansion the same non-ASCII fallback matches the deeper folder');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/İ/é' })).name, 'Accent',
    'equal original non-ASCII characters also bypass lower-case fallback');
  assert.equal(api.getWorkspaceFolder(Uri.from({ scheme: 'remote', path: '/I/x' })).name, 'Inner');
  scope.dispose();
});
