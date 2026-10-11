'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const disk = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { Uri } = require('./api-types.cjs');
const { watchLocal, _testing } = require('./local-file-watcher.cjs');

async function fixture(t, baseSuffix = '') {
  const root = await disk.mkdtemp(path.join(os.tmpdir(), 'vscli-local-watch-'));
  const events = [], errors = [];
  let handle;
  t.after(async () => { handle?.dispose(); await handle?.closed; await disk.rm(root, { recursive: true, force: true }); });
  return { root, base: path.join(root, baseSuffix), events, errors,
    start(options = {}) {
      handle = watchLocal(Uri.file(this.base), options, event => events.push({ type: event.type, path: event.uri.fsPath }), error => errors.push(error));
      return handle;
    },
  };
}
async function until(predicate, label) {
  const deadline = Date.now() + 8000;
  while (!predicate()) {
    if (Date.now() >= deadline) assert.fail('Timed out: ' + label);
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
function has(f, type, name) { return f.events.some(event => event.type === type && event.path === path.join(f.root, name)); }

test('real create/change/delete/rename notifications have no initial create flood', async t => {
  const f = await fixture(t);
  await disk.writeFile(path.join(f.root, 'old.txt'), 'old 猫\r\n');
  const handle = f.start({ recursive: true });
  await handle.ready;
  assert.deepEqual(f.events, []);
  await disk.writeFile(path.join(f.root, 'new.txt'), 'created');
  await until(() => has(f, 2, 'new.txt'), 'created');
  await disk.appendFile(path.join(f.root, 'new.txt'), ' changed');
  await until(() => has(f, 1, 'new.txt'), 'changed');
  await disk.rename(path.join(f.root, 'new.txt'), path.join(f.root, 'renamed.txt'));
  await until(() => has(f, 3, 'new.txt') && has(f, 2, 'renamed.txt'), 'rename delete+create');
  await disk.unlink(path.join(f.root, 'renamed.txt'));
  await until(() => has(f, 3, 'renamed.txt'), 'deleted');
  assert.deepEqual(f.errors, []);
  assert.equal(await disk.readFile(path.join(f.root, 'old.txt'), 'utf8'), 'old 猫\r\n');
});

test('missing/deleted/recreated directory resumes its original base and future writes', async t => {
  const f = await fixture(t, 'missing/deep');
  const handle = f.start({ recursive: true });
  await handle.ready;
  assert.deepEqual(f.events, []);
  await disk.mkdir(f.base, { recursive: true });
  await disk.writeFile(path.join(f.base, 'one.txt'), 'one');
  await until(() => has(f, 2, 'missing/deep/one.txt'), 'initial missing base becomes present');
  await disk.rm(path.join(f.root, 'missing'), { recursive: true });
  await until(() => has(f, 3, 'missing/deep/one.txt'), 'removed root descendants');
  await disk.mkdir(f.base, { recursive: true });
  await disk.writeFile(path.join(f.base, 'two.txt'), 'two');
  await until(() => has(f, 2, 'missing/deep/two.txt'), 'recreated original root');
  await disk.writeFile(path.join(f.base, 'three.txt'), 'three');
  await until(() => has(f, 2, 'missing/deep/three.txt'), 'new inode actual watch reattached');
  assert.deepEqual(f.errors, []);
});

test('RelativePattern file base tracks writes, deletion, replacement and later edits', async t => {
  const f = await fixture(t, 'single.txt');
  await disk.writeFile(f.base, 'initial');
  const handle = f.start({ recursive: false });
  await handle.ready;
  assert.deepEqual(f.events, []);
  await disk.appendFile(f.base, '+');
  await until(() => has(f, 1, 'single.txt'), 'file changed');
  await disk.unlink(f.base);
  await until(() => has(f, 3, 'single.txt'), 'file deleted');
  await disk.writeFile(f.base, 'replacement');
  await until(() => has(f, 2, 'single.txt'), 'file recreated');
  const before = f.events.filter(e => e.type === 1).length;
  await disk.appendFile(f.base, ' later');
  await until(() => f.events.filter(e => e.type === 1).length > before, 'replacement inode change');
  assert.deepEqual(f.errors, []);
});

test('child symlinks are not followed; an explicit symlink base follows and preserves spelling', async t => {
  if (process.platform === 'win32') { t.skip('symlink privilege dependent; Unix behavior qualified'); return; }
  const f = await fixture(t);
  const outside = await disk.mkdtemp(path.join(os.tmpdir(), 'vscli-local-watch-outside-'));
  t.after(() => disk.rm(outside, { recursive: true, force: true }));
  await disk.symlink(outside, path.join(f.root, 'alias'));
  const handle = f.start({ recursive: true });
  await handle.ready;
  await disk.writeFile(path.join(outside, 'outside.txt'), 'outside');
  await disk.writeFile(path.join(f.root, 'barrier.txt'), 'barrier');
  await until(() => has(f, 2, 'barrier.txt'), 'same-worker positive barrier');
  assert.ok(!has(f, 2, 'alias/outside.txt'));
  handle.dispose(); await handle.closed;
  const aliased = [];
  const explicit = watchLocal(Uri.file(path.join(f.root, 'alias')), { recursive: true }, e => aliased.push(e), e => f.errors.push(e));
  t.after(async () => { explicit.dispose(); await explicit.closed; });
  await explicit.ready;
  await disk.writeFile(path.join(outside, 'followed.txt'), 'followed');
  await until(() => aliased.some(e => e.type === 2 && e.uri.fsPath === path.join(f.root, 'alias/followed.txt')), 'base symlink spelling');
  assert.deepEqual(f.errors, []);
});

test('nonrecursive excludes bypass and recursive subtree-root pruning', async t => {
  const f = await fixture(t);
  await disk.mkdir(path.join(f.root, 'ignored'));
  // Excluded tree exceeds recursive watch handle capacity; it must be pruned at
  // its root, not enumerated and only filtered at the delivered-event layer.
  for (let n = 0; n < 257; n++) await disk.mkdir(path.join(f.root, 'ignored', 'd' + n));
  const h = f.start({ recursive: false, excludes: ['**/*.txt'] });
  await h.ready;
  await disk.writeFile(path.join(f.root, 'direct.txt'), 'direct');
  await until(() => has(f, 2, 'direct.txt'), 'nonrecursive excludes bypassed');
  h.dispose(); await h.closed;
  f.events.length = 0;
  const recursive = f.start({ recursive: true, excludes: ['**/ignored/**'] });
  await recursive.ready;
  await disk.writeFile(path.join(f.root, 'ignored/hidden.txt'), 'hidden');
  await disk.writeFile(path.join(f.root, 'visible.txt'), 'visible');
  await until(() => has(f, 2, 'visible.txt'), 'positive unexcluded barrier');
  assert.ok(!has(f, 2, 'ignored/hidden.txt'));
  assert.deepEqual(f.errors, []);
});

test('dispose suppresses callbacks immediately and capacity releases only actual exit', async t => {
  const f = await fixture(t);
  const handles = [];
  t.after(async () => { for (const h of handles) h.dispose(); await Promise.all(handles.map(h => h.closed)); });
  for (let i = 0; i < 32; i++) handles.push(watchLocal(Uri.file(f.root), {}, e => f.events.push(e), e => f.errors.push(e)));
  assert.equal(_testing.actualWorkers(), 32);
  assert.throws(() => watchLocal(Uri.file(f.root), {}, () => {}, () => {}), /worker limit/);
  for (const handle of handles) handle.dispose();
  assert.equal(_testing.actualWorkers(), 32, 'logical dispose does not free original workers');
  assert.throws(() => watchLocal(Uri.file(f.root), {}, () => {}, () => {}), /worker limit/);
  await Promise.all(handles.map(h => h.closed));
  assert.equal(_testing.actualWorkers(), 0);
  await disk.writeFile(path.join(f.root, 'after.txt'), 'after');
  const replacement = f.start();
  await replacement.ready;
  assert.deepEqual(f.events, []);
  replacement.dispose(); await replacement.closed;
});

test('recursive scan overflow is explicit and actual worker exits without initial flood', async t => {
  const f = await fixture(t);
  for (let i = 0; i < 257; i++) await disk.mkdir(path.join(f.root, 'dir-' + i));
  const handle = f.start({ recursive: true });
  await assert.rejects(handle.ready, /directory.*budget/);
  await handle.closed;
  assert.equal(f.errors.length, 1);
  assert.deepEqual(f.events, []);
});
