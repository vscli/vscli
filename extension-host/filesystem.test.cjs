'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const disk = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { Uri, EventEmitter, Disposable } = require('./api-types.cjs');
const { createFileSystem, FileType, FilePermission, FileChangeType, FileSystemError } = require('./filesystem.cjs');
const { createApi } = require('./api.cjs');
async function local(t) {
  const root = await disk.mkdtemp(path.join(os.tmpdir(), 'vscli-fs-api-'));
  t.after(() => disk.rm(root, { recursive: true, force: true }));
  return { root, uri: Uri.file(root), fs: createFileSystem().forOwner('').fs };
}
function memory() {
  const values = new Map([['/', { type: FileType.Directory, bytes: new Uint8Array() }]]);
  const events = new EventEmitter();
  const calls = [];
  const get = uri => { const value = values.get(uri.path); if (!value) throw FileSystemError.FileNotFound(uri); return value; };
  const provider = {
    onDidChangeFile: events.event,
    watch: () => new Disposable(() => {}),
    stat(uri) { const value = get(uri); return { type: value.type, ctime: 1, mtime: 2, size: value.bytes.byteLength, permissions: value.permissions }; },
    readDirectory(uri) {
      if (!(get(uri).type & FileType.Directory)) throw FileSystemError.FileNotADirectory(uri);
      return [...values].filter(([name]) => name !== uri.path && path.posix.dirname(name) === uri.path).map(([name, value]) => [path.posix.basename(name), value.type]);
    },
    createDirectory(uri) {
      if (values.has(uri.path)) throw FileSystemError.FileExists(uri);
      if (!values.has(path.posix.dirname(uri.path))) throw FileSystemError.FileNotFound(uri);
      calls.push(['mkdir', uri.path]); values.set(uri.path, { type: FileType.Directory, bytes: new Uint8Array() });
    },
    readFile: uri => get(uri).bytes,
    writeFile(uri, bytes, options) {
      calls.push(['write', uri.path, options]);
      if (values.has(uri.path) && !options.overwrite) throw FileSystemError.FileExists(uri);
      values.set(uri.path, { type: FileType.File, bytes });
    },
    delete(uri, options) {
      get(uri); calls.push(['delete', uri.path, options]);
      for (const name of values.keys()) if (name === uri.path || options.recursive && name.startsWith(`${uri.path}/`)) values.delete(name);
    },
    rename(from, to, options) {
      const value = get(from); calls.push(['rename', from.path, to.path, options]);
      if (values.has(to.path) && !options.overwrite) throw FileSystemError.FileExists(to);
      values.set(to.path, value); values.delete(from.path);
    },
  };
  return { provider, values, events, calls };
}
const resource = name => Uri.parse(`mem:${name}`);
test('filesystem error factories preserve source names, codes and URI messages', () => {
  for (const [name, code] of Object.entries({ FileExists: 'EntryExists', FileNotFound: 'EntryNotFound', FileNotADirectory: 'EntryNotADirectory', FileIsADirectory: 'EntryIsADirectory', NoPermissions: 'NoPermissions', Unavailable: 'Unavailable' })) {
    const error = FileSystemError[name](resource('/a%20b'));
    assert.ok(error instanceof Error); assert.equal(error.code, name);
    assert.equal(error.name, `${code} (FileSystemError)`); assert.equal(error.message, 'mem:/a b');
  }
  assert.equal(new FileSystemError('unknown').code, 'Unknown');
  assert.equal(FileType[64], 'SymbolicLink'); assert.equal(FilePermission.Readonly, 1); assert.equal(FileChangeType.Created, 2);
});
test('disk APIs create parents, preserve arbitrary bytes and never alias write/read buffers', async t => {
  const { fs, uri, root } = await local(t);
  const target = Uri.joinPath(uri, 'nested', 'binary');
  const bytes = new Uint8Array([0, 255, 13, 10, 128]);
  const write = fs.writeFile(target, bytes); bytes.fill(77); await write;
  assert.deepEqual([...await disk.readFile(path.join(root, 'nested', 'binary'))], [0, 255, 13, 10, 128]);
  const first = await fs.readFile(target); first.fill(2);
  assert.deepEqual([...await fs.readFile(target)], [0, 255, 13, 10, 128]);
  const stat = await fs.stat(target); assert.equal(stat.type, FileType.File); assert.equal(stat.size, 5);
  assert.equal(typeof stat.ctime, 'number'); assert.equal(stat.permissions, undefined);
  assert.deepEqual(await fs.readDirectory(Uri.joinPath(uri, 'nested')), [['binary', FileType.File]]);
  await fs.createDirectory(Uri.joinPath(uri, 'nested')); // idempotent
  await assert.rejects(fs.createDirectory(target), { code: 'FileExists' });
});
test('overwrite failures retain both files; rename, copy and recursive delete acknowledge actual disk state', async t => {
  const { fs, uri } = await local(t);
  const a = Uri.joinPath(uri, 'a'), b = Uri.joinPath(uri, 'b'), c = Uri.joinPath(uri, 'dir', 'c');
  await fs.writeFile(a, new Uint8Array([1])); await fs.writeFile(b, new Uint8Array([2]));
  await assert.rejects(fs.rename(a, b), { code: 'FileExists' });
  await assert.rejects(fs.copy(a, b), { code: 'FileExists' });
  assert.deepEqual([...await fs.readFile(a)], [1]); assert.deepEqual([...await fs.readFile(b)], [2]);
  await fs.copy(a, c); await fs.rename(a, b, { overwrite: true });
  await assert.rejects(fs.stat(a), { code: 'FileNotFound' });
  assert.deepEqual([...await fs.readFile(b)], [1]); assert.deepEqual([...await fs.readFile(c)], [1]);
  await assert.rejects(fs.delete(Uri.joinPath(uri, 'dir')), FileSystemError);
  await fs.delete(Uri.joinPath(uri, 'dir'), { recursive: true });
  await assert.rejects(fs.stat(c), { code: 'FileNotFound' });
  await assert.rejects(fs.delete(b, { useTrash: true }), { code: 'Unavailable' });
  assert.deepEqual([...await fs.readFile(b)], [1]);
});
test('disk symlinks report combined target bits, preserve links when copied and unlink without deleting target', async t => {
  if (process.platform === 'win32') return t.skip('Windows symlink privileges require separate qualification');
  const { fs, uri, root } = await local(t);
  await fs.writeFile(Uri.joinPath(uri, 'target'), new Uint8Array([1]));
  await disk.symlink('target', path.join(root, 'link'));
  assert.equal((await fs.stat(Uri.joinPath(uri, 'link'))).type, FileType.File | FileType.SymbolicLink);
  await fs.copy(Uri.joinPath(uri, 'link'), Uri.joinPath(uri, 'copied'));
  assert.equal(await disk.readlink(path.join(root, 'copied')), 'target');
  await fs.delete(Uri.joinPath(uri, 'link'));
  assert.deepEqual([...await fs.readFile(Uri.joinPath(uri, 'target'))], [1]);
});
test('overwriting a directory replaces stale children for disk copy and rename', async t => {
  const { fs, uri } = await local(t);
  await fs.writeFile(Uri.joinPath(uri, 'source', 'fresh'), new Uint8Array([1]));
  await fs.writeFile(Uri.joinPath(uri, 'target', 'stale'), new Uint8Array([2]));
  await fs.copy(Uri.joinPath(uri, 'source'), Uri.joinPath(uri, 'target'), { overwrite: true });
  assert.deepEqual(await fs.readDirectory(Uri.joinPath(uri, 'target')), [['fresh', FileType.File]]);
  await fs.writeFile(Uri.joinPath(uri, 'target', 'stale'), new Uint8Array([2]));
  await fs.rename(Uri.joinPath(uri, 'source'), Uri.joinPath(uri, 'target'), { overwrite: true });
  assert.deepEqual(await fs.readDirectory(Uri.joinPath(uri, 'target')), [['fresh', FileType.File]]);
  await assert.rejects(fs.stat(Uri.joinPath(uri, 'source')), { code: 'FileNotFound' });
  await fs.copy(Uri.joinPath(uri, 'target'), Uri.joinPath(uri, 'target'), { overwrite: true });
  assert.deepEqual(await fs.readDirectory(Uri.joinPath(uri, 'target')), [['fresh', FileType.File]]);
});
test('ancestor overwrite and symlink aliases reject before deleting source or destination', async t => {
  const { fs, uri, root } = await local(t);
  const child = Uri.joinPath(uri, 'tree', 'child'), ancestor = Uri.joinPath(uri, 'tree');
  await fs.writeFile(Uri.joinPath(child, 'kept'), new Uint8Array([1, 2]));
  await fs.writeFile(Uri.joinPath(ancestor, 'other'), new Uint8Array([3]));
  for (const method of ['rename', 'copy']) await assert.rejects(fs[method](child, ancestor, { overwrite: true }), { code: 'Unavailable' });
  if (process.platform !== 'win32') {
    await disk.symlink('tree', path.join(root, 'alias'));
    for (const method of ['rename', 'copy']) await assert.rejects(fs[method](Uri.joinPath(uri, 'alias', 'child'), ancestor, { overwrite: true }), { code: 'Unavailable' });
  }
  assert.deepEqual([...await fs.readFile(Uri.joinPath(child, 'kept'))], [1, 2]);
  assert.deepEqual([...await fs.readFile(Uri.joinPath(ancestor, 'other'))], [3]);
});
test('malformed provider bytes cannot turn a cross-provider move into empty-file data loss', async () => {
  const scope = createFileSystem().forOwner('test'), source = memory(), target = memory();
  source.values.set('/kept', { type: FileType.File, bytes: new Uint8Array([1, 2]) });
  source.provider.readFile = () => undefined;
  scope.registerFileSystemProvider('mem', source.provider); scope.registerFileSystemProvider('target', target.provider);
  await assert.rejects(scope.fs.rename(resource('/kept'), Uri.parse('target:/copy')), { code: 'Unavailable' });
  assert.deepEqual([...source.values.get('/kept').bytes], [1, 2]);
  assert.equal(target.values.has('/copy'), false);
});
test('registration reserves a scheme before extension-owned option and event callbacks run', () => {
  const scope = createFileSystem().forOwner('test'), mem = memory(), nested = memory();
  const options = { get isReadonly() {
    assert.throws(() => scope.registerFileSystemProvider('mem', nested.provider), /already registered/);
    return false;
  } };
  mem.provider.onDidChangeFile = () => {
    assert.throws(() => scope.registerFileSystemProvider('mem', nested.provider), /already registered/);
    return new Disposable(() => {});
  };
  const registration = scope.registerFileSystemProvider('mem', mem.provider, options);
  assert.equal(scope.fs.isWritableFileSystem('mem'), true); registration.dispose();
});
test('retrying a throwing subscription disposal cannot remove a replacement provider', async () => {
  const scope = createFileSystem().forOwner('test'), mem = memory(), nested = memory();
  let disposed = 0;
  mem.provider.onDidChangeFile = () => ({ dispose() { if (++disposed === 1) throw new Error('dispose failed'); } });
  const old = scope.registerFileSystemProvider('mem', mem.provider);
  assert.throws(() => old.dispose(), /dispose failed/);
  scope.registerFileSystemProvider('mem', nested.provider);
  old.dispose();
  assert.equal(scope.fs.isWritableFileSystem('mem'), true);
  await scope.fs.writeFile(resource('/x'), new Uint8Array([1]));
  assert.deepEqual([...nested.values.get('/x').bytes], [1]);
});
test('provider registration validates completeness and ownership and releases capabilities on disposal', async () => {
  const service = createFileSystem(), { fs, registerFileSystemProvider } = service.forOwner('test');
  const mem = memory();
  assert.throws(() => registerFileSystemProvider('bad', {}), /watch/);
  assert.throws(() => registerFileSystemProvider('file', mem.provider), /already registered/);
  const registration = registerFileSystemProvider('mem', mem.provider, { isCaseSensitive: true });
  assert.equal(fs.isWritableFileSystem('file'), true); assert.equal(fs.isWritableFileSystem('mem'), true);
  assert.equal(fs.isWritableFileSystem('unknown'), undefined); assert.equal(service.pathCaseSensitive('mem'), true);
  await fs.writeFile(resource('/a/b'), new Uint8Array([7]));
  assert.deepEqual(mem.calls, [['mkdir', '/a'], ['write', '/a/b', { create: true, overwrite: true }]]);
  registration.dispose(); assert.equal(fs.isWritableFileSystem('mem'), undefined);
  await assert.rejects(fs.readFile(resource('/a/b')), { code: 'Unavailable' });
});
test('readonly providers support isolated read snapshots and reject all mutation entry points', async () => {
  const service = createFileSystem(), scope = service.forOwner('test'), mem = memory();
  mem.values.set('/x', { type: FileType.File, permissions: FilePermission.Readonly, bytes: new Uint8Array([9]) });
  scope.registerFileSystemProvider('mem', mem.provider, { isReadonly: { value: 'Read only' } });
  assert.equal(scope.fs.isWritableFileSystem('mem'), false);
  const stat = await scope.fs.stat(resource('/x')); stat.type = 0;
  assert.equal((await scope.fs.stat(resource('/x'))).type, FileType.File);
  assert.equal((await scope.fs.stat(resource('/x'))).permissions, FilePermission.Readonly);
  (await scope.fs.readFile(resource('/x'))).fill(0); assert.deepEqual([...mem.values.get('/x').bytes], [9]);
  for (const call of [() => scope.fs.writeFile(resource('/x'), new Uint8Array()), () => scope.fs.createDirectory(resource('/new')), () => scope.fs.delete(resource('/x')), () => scope.fs.rename(resource('/x'), resource('/y')), () => scope.fs.copy(resource('/x'), resource('/y'))]) {
    await assert.rejects(call(), { code: 'NoPermissions' });
  }
  assert.equal(mem.calls.length, 0);
});
test('same-resource writes are serialized; rejection does not poison subsequent writes', async () => {
  const service = createFileSystem(), scope = service.forOwner('test'), mem = memory();
  let release, entered;
  const started = new Promise(resolve => { entered = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  const original = mem.provider.writeFile;
  let count = 0;
  mem.provider.writeFile = async (...args) => {
    count++; if (count === 1) { entered(); await gate; throw FileSystemError.Unavailable('write failed'); }
    original(...args);
  };
  scope.registerFileSystemProvider('mem', mem.provider);
  const first = scope.fs.writeFile(resource('/x'), new Uint8Array([1]));
  const failure = assert.rejects(first, { code: 'Unavailable' });
  await started;
  const second = scope.fs.writeFile(resource('/x'), new Uint8Array([2]));
  await Promise.resolve(); assert.equal(count, 1);
  release(); await failure; await second;
  assert.deepEqual([...await scope.fs.readFile(resource('/x'))], [2]);
});
test('bounded admission remains occupied by real pending provider callbacks', async () => {
  const service = createFileSystem(), scope = service.forOwner('test'), mem = memory();
  const resolvers = [];
  mem.provider.stat = () => new Promise(resolve => resolvers.push(resolve));
  scope.registerFileSystemProvider('mem', mem.provider);
  const pending = Array.from({ length: 64 }, () => scope.fs.stat(resource('/x')));
  await Promise.resolve(); assert.equal(resolvers.length, 64);
  await assert.rejects(scope.fs.stat(resource('/x')), { code: 'Unavailable' });
  for (const resolve of resolvers) resolve({ type: 1, ctime: 0, mtime: 0, size: 0 });
  await Promise.all(pending);
  mem.provider.stat = () => ({ type: 1, ctime: 0, mtime: 0, size: 0 });
  assert.equal((await scope.fs.stat(resource('/x'))).type, 1);
});
test('a queued mutation cannot run against a replacement provider for the same scheme', async () => {
  const scope = createFileSystem().forOwner('test'), first = memory(), replacement = memory();
  let release, enter;
  const started = new Promise(resolve => { enter = resolve; });
  first.provider.writeFile = async () => { enter(); await new Promise(resolve => { release = resolve; }); };
  const registration = scope.registerFileSystemProvider('mem', first.provider);
  const active = scope.fs.writeFile(resource('/active'), new Uint8Array([1]));
  await started;
  const queued = scope.fs.writeFile(resource('/queued'), new Uint8Array([2]));
  const rejected = assert.rejects(queued, { code: 'Unavailable' });
  registration.dispose(); scope.registerFileSystemProvider('mem', replacement.provider);
  release(); await active; await rejected;
  assert.equal(replacement.calls.length, 0);
});
test('directory fallback copies recursively across providers and rejects self descendants before mutation', async t => {
  const { uri } = await local(t);
  const scope = createFileSystem().forOwner('test'), mem = memory();
  scope.registerFileSystemProvider('mem', mem.provider);
  await scope.fs.writeFile(resource('/source/child/bytes'), new Uint8Array([8, 0, 255]));
  await assert.rejects(scope.fs.copy(resource('/source'), resource('/source/recursive')), { code: 'Unavailable' });
  await scope.fs.copy(resource('/source'), Uri.joinPath(uri, 'copied'));
  assert.deepEqual([...await scope.fs.readFile(Uri.joinPath(uri, 'copied', 'child', 'bytes'))], [8, 0, 255]);
  assert.deepEqual([...await scope.fs.readFile(resource('/source/child/bytes'))], [8, 0, 255]);
});
test('cross-provider move does not delete source when destination write fails', async t => {
  const { fs: _localFs, uri } = await local(t);
  const service = createFileSystem(), scope = service.forOwner('test'), mem = memory();
  const source = Uri.joinPath(uri, 'source'); await scope.fs.writeFile(source, new Uint8Array([42]));
  mem.provider.writeFile = () => { throw FileSystemError.NoPermissions('denied'); };
  scope.registerFileSystemProvider('mem', mem.provider);
  await assert.rejects(scope.fs.rename(source, resource('/target')), { code: 'NoPermissions' });
  assert.deepEqual([...await scope.fs.readFile(source)], [42]);
  mem.provider.writeFile = (target, bytes) => mem.values.set(target.path, { type: FileType.File, bytes });
  await scope.fs.rename(source, resource('/target'));
  await assert.rejects(scope.fs.stat(source), { code: 'FileNotFound' });
  assert.deepEqual([...await scope.fs.readFile(resource('/target'))], [42]);
});
test('provider events filter schemes and stop after disposal; events never fabricate document saves', () => {
  const service = createFileSystem(), scope = service.forOwner('test'), mem = memory(), observed = [];
  service.onDidChangeFiles(events => observed.push(events));
  const registration = scope.registerFileSystemProvider('mem', mem.provider);
  mem.events.fire([{ uri: resource('/x'), type: 2 }, { uri: Uri.file('/x'), type: 2 }]);
  assert.equal(observed.length, 1); assert.equal(observed[0].length, 1); assert.equal(observed[0][0].uri.scheme, 'mem');
  registration.dispose(); mem.events.fire([{ uri: resource('/x'), type: 3 }]);
  assert.equal(observed.length, 1);
});
test('extension facade shares providers across owners and disposed owners cannot call or retain them', async () => {
  const active = new Set(['a', 'b']);
  const runtime = createApi(() => {}, () => {});
  runtime.setActivation({ accepts: owner => active.has(owner), facade: () => ({}) });
  const a = runtime.forExtension('a'), b = runtime.forExtension('b'), mem = memory();
  a.workspace.registerFileSystemProvider('mem', mem.provider);
  await b.workspace.fs.writeFile(resource('/x'), new Uint8Array([3]));
  assert.deepEqual([...await a.workspace.fs.readFile(resource('/x'))], [3]);
  runtime.disposeOwner('a'); active.delete('a');
  await assert.rejects(b.workspace.fs.readFile(resource('/x')), { code: 'Unavailable' });
  await assert.rejects(a.workspace.fs.stat(Uri.file('/')), /not active/);
});
