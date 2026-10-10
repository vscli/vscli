'use strict';
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { settleOutline, observeOutlineCommand } = require('./outline.cjs');

function clock({ eventAt = Infinity, activeUntil = 0, advance = 20 } = {}) {
  let milliseconds = 0;
  return {
    now: () => milliseconds,
    wait: async () => { milliseconds += advance; },
    snapshot: action => ({ action, primary: {
      anchor: milliseconds >= eventAt ? 74 : 0,
      cursor: milliseconds >= eventAt ? 74 : 0,
    } }),
    activeCallbacks: () => milliseconds < activeUntil,
  };
}

test('a dispatched command with a delayed350–500ms event runs exactly once and is not accepted early', async () => {
  for (const eventAt of [350, 500]) {
    const options = clock({ eventAt });
    const commands = [];
    const observed = await observeOutlineCommand('list.select', {
      ...options, execute: async command => { commands.push(command); },
    });
    assert.deepEqual(commands, ['list.select']);
    assert.equal(observed.elapsedMs, 1000);
    assert.deepEqual(observed.observed.primary, { anchor: 74, cursor: 74 });
  }
});

test('unchanged no-op results settle without waiting for a preferred selection', async () => {
  const options = clock();
  let commands = 0;
  const observed = await observeOutlineCommand('list.select', {
    ...options, execute: async () => { commands++; },
  });
  assert.equal(commands, 1);
  assert.equal(observed.elapsedMs, 1000);
  assert.deepEqual(observed.observed.primary, { anchor: 0, cursor: 0 });
});

test('a late public-state change must have the complete100ms quiet window', async () => {
  const observed = await settleOutline('list.select', clock({ eventAt: 960 }));
  assert.equal(observed.elapsedMs, 1060);
  assert.equal(observed.observed.primary.cursor, 74);
});

test('an active callback and its completion restart the quiet window even without editor changes', async () => {
  const observed = await settleOutline('outline.focus', clock({ activeUntil: 1060 }));
  assert.equal(observed.elapsedMs, 1160);
  assert.equal(observed.observed.primary.cursor, 0);
});

test('callback completion can settle just inside the original3second deadline', async () => {
  const observed = await settleOutline('outline.focus', clock({ activeUntil: 2880 }));
  assert.equal(observed.elapsedMs, 2980);
});

test('occupied callbacks cannot extend the original3second deadline', async () => {
  await assert.rejects(settleOutline('outline.focus', clock({ activeUntil: Infinity })),
    /Independent Outline fixture settlement exceeded3seconds/);
});

test('a delayed scheduling tick at the deadline cannot publish an otherwise quiet result', async () => {
  await assert.rejects(settleOutline('outline.focus', clock({ advance: 3000 })),
    /Independent Outline fixture settlement exceeded3seconds/);
});

test('failed command acknowledgment is not retried or replaced by a settlement result', async () => {
  let commands = 0, snapshots = 0;
  await assert.rejects(observeOutlineCommand('list.select', {
    ...clock(),
    execute: async () => { commands++; throw new Error('fixture command failure'); },
    snapshot: () => { snapshots++; return {}; },
  }), /fixture command failure/);
  assert.equal(commands, 1);
  assert.equal(snapshots, 0);
});
