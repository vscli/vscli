'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createSurfaces, TreeItem, ThemeColor, limits } = require('./surfaces.cjs');
const { EventEmitter } = require('./api-types.cjs');
function fixture() {
  const messages = [], calls = [], owned = new Map();
  const runtime = createSurfaces((method, params) => messages.push({ method, params }), 7,
    (owner, disposable) => { if (!owned.has(owner)) owned.set(owner, []); owned.get(owner).push(disposable); return disposable; },
    async (...args) => calls.push(args));
  runtime.configure([{ id: 'one.extension', manifest: { contributes: { views: { explorer: [{ id: 'one.tree', name: 'First Tree' }] } } } },
    { id: 'two.extension', manifest: { contributes: { views: { explorer: [{ id: 'two.tree', name: 'Other Tree' }] } } } }]);
  return { runtime, messages, calls, api: runtime.forExtension('one.extension'),
    dispose(owner) { for (const item of owned.get(owner) || []) item.dispose(); } };
}
test('output lifecycle is scoped, bounded and preserves focus intent without document calls', () => {
  const f = fixture(), channel = f.api.createOutputChannel('Compiler', 'cpp');
  channel.append('猫'); channel.appendLine('done'); channel.replace('next'); channel.clear();
  channel.show(true); channel.show(2, false); channel.hide(); channel.dispose(); channel.append('late');
  assert.deepEqual(f.messages.map(m => m.params.op), ['outputCreate', 'outputAppend', 'outputAppend', 'outputReplace', 'outputClear', 'outputShow', 'outputShow', 'outputHide', 'outputDispose']);
  assert.equal(f.messages[2].params.text, 'done\n');
  assert.equal(f.messages[5].params.preserveFocus, true);
  assert.equal(f.messages[6].params.preserveFocus, false);
  assert(f.messages.every(m => m.method === 'nativeSurface' && m.params.owner === 'one.extension' && m.params.session === 7));
  assert.equal(f.calls.length, 0);
  assert.throws(() => f.api.createOutputChannel('log', { log: true }), /Invalid output language/);
  const channels = Array.from({ length: limits.channels }, (_, i) => f.api.createOutputChannel(`channel${i}`));
  assert.throws(() => f.api.createOutputChannel('overflow'), /limit/);
  assert.throws(() => channels[0].append('x'.repeat(limits.updateBytes + 1)), /Invalid output update/);
  f.dispose('one.extension');
  assert.doesNotThrow(() => f.api.createOutputChannel('released'));
});
test('status properties and opaque commands validate ownership and dispose independently', async () => {
  const f = fixture(), item = f.api.createStatusBarItem('build', 2, 12);
  const payload = { private: true }; payload.self = payload;
  item.text = '$(sync) Building'; item.tooltip = 'Compiler state'; item.command = { command: 'build.run', arguments: [payload] };
  item.backgroundColor = new ThemeColor('statusBarItem.warningBackground'); item.show();
  const state = f.messages.at(-1).params;
  assert.equal(state.alignment, 2); assert.equal(state.priority, 12); assert.equal(state.hasCommand, true);
  assert(!JSON.stringify(state).includes('private'));
  assert.throws(() => { item.tooltip = { value: 'unsupported markdown' }; }, /plain text/);
  assert.equal(item.tooltip, 'Compiler state');
  await assert.rejects(() => f.runtime.action({ session: 7, owner: 'two.extension', id: state.id, generation: state.generation, kind: 'status' }), /Stale/);
  await f.runtime.action({ session: 7, owner: 'one.extension', id: state.id, generation: state.generation, kind: 'status' });
  assert.equal(f.calls[0][2][0], payload);
  item.hide();
  await assert.rejects(() => f.runtime.action({ session: 7, owner: 'one.extension', id: state.id, generation: state.generation, kind: 'status' }), /Stale/);
  item.dispose(); item.show();
  assert.equal(f.messages.at(-1).params.op, 'statusDispose');
});
test('lazy trees retain element argument identity, invalidate late results and reject foreign views', async () => {
  const f = fixture(), changed = new EventEmitter(), root = { label: 'Root' }, leaf = { label: 'Leaf' };
  root.self = root;
  let unblock;
  const provider = { onDidChangeTreeData: changed.event,
    getChildren: node => node ? [leaf] : [root],
    getTreeItem: node => Object.assign(new TreeItem(node.label, node === root ? 1 : 0), {
      id: node.label, command: { command: 'tree.open', arguments: [node] },
    }) };
  assert.throws(() => f.api.createTreeView('two.tree', { treeDataProvider: provider }), /not contributed/);
  const view = f.api.createTreeView('one.tree', { treeDataProvider: provider });
  const params = { session: 7, owner: 'one.extension', id: 'one.tree', generation: f.messages.at(-1).params.generation };
  const roots = await f.runtime.treeChildren(params);
  assert.equal(roots.items[0].label, 'Root');
  assert(!JSON.stringify(roots).includes('self'));
  const children = await f.runtime.treeChildren({ ...params, node: roots.items[0].node });
  assert.equal(children.items[0].label, 'Leaf');
  await f.runtime.action({ ...params, node: roots.items[0].node });
  assert.equal(f.calls[0][2][0], root);
  assert.equal(view.selection[0], root);
  provider.getTreeItem = () => new Promise(resolve => { unblock = resolve; });
  const late = f.runtime.treeChildren(params);
  await new Promise(resolve => setImmediate(resolve));
  changed.fire(); unblock(new TreeItem('Obsolete'));
  await assert.rejects(() => late, /changed while/);
  await assert.rejects(() => f.runtime.action({ ...params, node: roots.items[0].node }), /Stale/);
  view.dispose();
  assert.equal(f.messages.at(-1).params.op, 'treeDispose');
});
test('tree limits reject malformed or oversized replies before publishing handles', async () => {
  const f = fixture();
  const provider = { getChildren: () => Array(257).fill(1), getTreeItem: value => new TreeItem(String(value)) };
  f.api.createTreeView('one.tree', { treeDataProvider: provider });
  const params = { session: 7, owner: 'one.extension', id: 'one.tree', generation: f.messages.at(-1).params.generation };
  await assert.rejects(() => f.runtime.treeChildren(params), /256/);
  provider.getChildren = () => [1, 1];
  await assert.rejects(() => f.runtime.treeChildren(params), /Duplicate/);
  provider.getChildren = () => [1]; provider.getTreeItem = () => new TreeItem('x'.repeat(1025));
  await assert.rejects(() => f.runtime.treeChildren(params), /tree label/);
  provider.getTreeItem = value => new TreeItem(String(value));
  const valid = await f.runtime.treeChildren(params);
  assert.equal(valid.items[0].node, 'node-1');
  assert.equal(valid.items.length, 1);
});
test('new tree batches never retarget rendered handles and reject ancestor cycles atomically', async () => {
  const f = fixture(), element = {}, child = {};
  let action = 'old.command', cycle = false;
  const view = f.api.createTreeView('one.tree', { treeDataProvider: {
    getChildren(parent) { return parent ? [cycle ? element : child] : [element]; },
    getTreeItem(value) { return { label: value === element ? 'Parent' : 'Child', collapsibleState: 1, command: { command: action } }; },
  } });
  const generation = f.messages.at(-1).params.generation;
  const request = { session: 7, owner: 'one.extension', id: 'one.tree', generation };
  const first = await f.runtime.treeChildren(request);
  action = 'new.command';
  const second = await f.runtime.treeChildren(request);
  assert.notEqual(first.items[0].node, second.items[0].node);
  // Even if native validation rejects second, the displayed first handle has
  // not changed command meaning and still uses its original opaque callback.
  await f.runtime.action({ ...request, kind: 'tree', node: first.items[0].node });
  assert.equal(f.calls.at(-1)[1], 'old.command');
  await f.runtime.action({ ...request, kind: 'tree', node: second.items[0].node });
  assert.equal(f.calls.at(-1)[1], 'new.command');
  cycle = true;
  await assert.rejects(f.runtime.treeChildren({ ...request, node: first.items[0].node }), /multiple parents|Cyclic/);
  view.dispose();
});
test('lifecycle owner guard rejects fresh resources and retained handles while disposal still clears them', async () => {
  let active = true;
  const owned = [], messages = [];
  const runtime = createSurfaces((method, params) => messages.push(params), 1,
    (_owner, disposable) => { owned.push(disposable); return disposable; },
    async () => { throw new Error('must not execute'); },
    () => { if (!active) throw new Error('Owner retired'); });
  const api = runtime.forExtension('owner.extension');
  const output = api.createOutputChannel('log');
  const status = api.createStatusBarItem('status'); status.command = 'owned.command'; status.show();
  const item = messages.at(-1);
  active = false;
  assert.throws(() => output.append('late'), /Owner retired/);
  assert.throws(() => api.createOutputChannel('late'), /Owner retired/);
  assert.throws(() => api.createStatusBarItem('late'), /Owner retired/);
  await assert.rejects(runtime.action({ session: 1, owner: 'owner.extension', id: item.id, generation: item.generation, kind: 'status' }), /Owner retired/);
  for (const disposable of owned) disposable.dispose();
  assert.deepEqual(messages.slice(-2).map(message => message.op), ['outputDispose', 'statusDispose']);
});
