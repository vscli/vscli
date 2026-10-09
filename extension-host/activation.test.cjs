'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createActivation } = require('./activation.cjs');
const item = (id, dependencies = []) => ({ id, folder: `/snapshots/${id}`, manifest: { extensionDependencies: dependencies } });
test('selected dependencies precede activation, coalesce and expose the same exports', async () => {
  const calls = [], value = { useful: true };
  const service = createActivation([item('test.a', ['test.b']), item('test.b')], {
    activate: async selected => { calls.push(selected.id); return value; },
  });
  const api = service.facade();
  assert.equal(api.getExtension('TEST.B').isActive, false);
  const [first, second] = await Promise.all([api.getExtension('test.a').activate(), api.getExtension('test.a').activate()]);
  assert.deepEqual(calls, ['test.b', 'test.a']);
  assert.equal(first, value); assert.equal(second, value);
  assert.equal(api.getExtension('test.b').exports, value);
  assert.equal(api.getExtension('test.b').isActive, true);
  assert.equal(api.getExtension('not.selected'), undefined);
  assert.throws(() => api.getExtension('test.b', true), /Cross-host/);
});
test('missing dependencies, cycles and cohort overflow reject before any code callback', () => {
  const hooks = { activate() { throw new Error('must not execute'); } };
  assert.throws(() => createActivation([item('test.a', ['test.missing'])], hooks), /not selected/);
  assert.throws(() => createActivation([item('test.a', ['test.b']), item('test.b', ['test.a'])], hooks), /dependency cycle/);
  assert.throws(() => createActivation(Array.from({ length: 9 }, (_, i) => item(`test.p${i}`)), hooks), /eight/);
});
test('runtime reverse waits reject rather than deadlock and failures do not retry', async () => {
  let calls = 0, service;
  service = createActivation([item('test.a', ['test.b']), item('test.b')], {
    activate: async selected => {
      calls++;
      if (selected.id === 'test.b') await service.extension('test.a').activate();
    },
  });
  await assert.rejects(service.activate('test.a'), /wait cycle/);
  await assert.rejects(service.activate('test.a'), /wait cycle/);
  assert.equal(calls, 1);
  assert.deepEqual(service.statuses().map(entry => entry.state), ['failed', 'failed']);
});
test('incremental descriptors preserve active exports and deactivate dependants first', async () => {
  const deactivated = [], value = {};
  const service = createActivation([item('test.b')], {
    activate: () => value, deactivate: selected => deactivated.push(selected.id),
  });
  await service.activate('test.b');
  const api = service.facade();
  let changed = 0;
  const disposable = api.onDidChange(() => changed++);
  service.add([item('test.a', ['test.b'])]);
  assert.equal(changed, 1);
  assert.equal(api.getExtension('test.b').exports, value);
  assert.deepEqual(api.all.map(entry => entry.id), ['test.a', 'test.b']);
  assert.throws(() => service.add([item('test.b')]), /already selected/);
  await service.activate('test.a');
  await service.shutdown();
  assert.deepEqual(deactivated, ['test.a', 'test.b']);
  disposable.dispose();
});
