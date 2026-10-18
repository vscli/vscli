'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { CancellationTokenSource, CancellationToken } = require('./cancellation.cjs');

function clock(t) { t.mock.timers.enable({ apis: ['setTimeout'] }); }

test('public extension facade exposes the shared source constructor and keeps token helpers internal', () => {
  const { createApi } = require('./api.cjs');
  const { api } = createApi(() => { throw new Error('Unexpected native request'); }, () => {}, { session: 1 });
  assert.equal(api.CancellationTokenSource, CancellationTokenSource);
  const token = new api.CancellationTokenSource().token;
  assert.equal(token.isCancellationRequested, false);
  assert.throws(() => api.CancellationToken, /not implemented/);
});

test('tokens retain exact identity through cancel and dispose', () => {
  const source = new CancellationTokenSource(), token = source.token;
  assert.equal(token.isCancellationRequested, false);
  assert.equal(source.token, token);
  source.cancel(); source.cancel(); source.dispose(); source.dispose(true);
  assert.equal(source.token, token);
  assert.equal(token.isCancellationRequested, true);
});

test('cancel before first token creates the canonical canceled token', () => {
  const source = new CancellationTokenSource();
  source.cancel();
  assert.equal(source.token, CancellationToken.Cancelled);
  source.dispose(); source.cancel();
  assert.equal(source.token, CancellationToken.Cancelled);
  assert(Object.isFrozen(CancellationToken.Cancelled));
  assert(Object.isFrozen(CancellationToken.None));
});

test('live cancellation is synchronous once, with receiver and undefined argument', () => {
  const source = new CancellationTokenSource(), calls = [], receiver = {};
  source.token.onCancellationRequested(function (value) {
    calls.push({ receiver: this, value, argc: arguments.length, canceled: source.token.isCancellationRequested });
  }, receiver);
  source.token.onCancellationRequested(() => calls.push('second'));
  source.cancel(); source.cancel();
  assert.deepEqual(calls, [{ receiver, value: undefined, argc: 1, canceled: true }, 'second']);
  source.dispose();
});

test('live subscription disposables are registered and individually idempotent', () => {
  const source = new CancellationTokenSource(), bag = [], calls = [];
  const removed = source.token.onCancellationRequested(() => calls.push('removed'), undefined, bag);
  const retained = source.token.onCancellationRequested(() => calls.push('retained'), undefined, bag);
  assert.deepEqual(bag, [removed, retained]);
  removed.dispose(); removed.dispose();
  source.cancel(); retained.dispose(); retained.dispose(); source.dispose();
  assert.deepEqual(calls, ['retained']);
});

test('late subscribers run on a timer turn once, are bound and individually disposable', t => {
  clock(t);
  const source = new CancellationTokenSource(), receiver = {}, calls = [], bag = [];
  source.cancel();
  const retained = source.token.onCancellationRequested(function () { calls.push([this, arguments.length]); }, receiver, bag);
  const removed = source.token.onCancellationRequested(() => calls.push('removed'));
  removed.dispose(); removed.dispose();
  assert.deepEqual(calls, []);
  // Pinned shortcutEvent does not register its disposable in the optional bag.
  assert.deepEqual(bag, []);
  source.dispose();
  t.mock.timers.tick(0);
  assert.deepEqual(calls, [[receiver, 0]]);
  t.mock.timers.tick(0);
  assert.deepEqual(calls, [[receiver, 0]]);
  retained.dispose();
});

test('disposed-before-access source stays None even after later cancel or dispose(true)', () => {
  const source = new CancellationTokenSource();
  source.dispose();
  const token = source.token, bag = [];
  let calls = 0;
  assert.equal(token, CancellationToken.None);
  token.onCancellationRequested(() => calls++, undefined, bag).dispose();
  source.cancel(); source.dispose(true);
  assert.equal(source.token, token);
  assert.equal(token.isCancellationRequested, false);
  assert.equal(calls, 0);
  assert.deepEqual(bag, []);
});

test('disposed existing token does not cancel and later cancel invokes no retired listeners', () => {
  const source = new CancellationTokenSource(), token = source.token;
  let calls = 0;
  token.onCancellationRequested(() => calls++);
  source.dispose();
  assert.equal(token.isCancellationRequested, false);
  source.cancel();
  assert.equal(token.isCancellationRequested, true);
  assert.equal(calls, 0);
  assert.equal(source.token, token);
});

test('fresh event getter after disposal can subscribe but cached retired event cannot', () => {
  const source = new CancellationTokenSource(), token = source.token, retiredEvent = token.onCancellationRequested;
  const calls = [];
  retiredEvent(() => calls.push('original'));
  source.dispose();
  retiredEvent(() => calls.push('cached'));
  token.onCancellationRequested(() => calls.push('fresh'));
  source.cancel();
  assert.deepEqual(calls, ['fresh']);
  source.dispose();
});

test('cached live event after cancel remains retired while fresh getter supplies late event', t => {
  clock(t);
  const source = new CancellationTokenSource(), token = source.token, cached = token.onCancellationRequested;
  const calls = [];
  source.cancel();
  cached(() => calls.push('cached'));
  token.onCancellationRequested(() => calls.push('fresh'));
  assert.deepEqual(calls, []);
  t.mock.timers.tick(0);
  assert.deepEqual(calls, ['fresh']);
  source.dispose();
});

test('live parent propagation is synchronous and single-shot without canceling the parent from child', () => {
  const parent = new CancellationTokenSource(), child = new CancellationTokenSource(parent.token), childToken = child.token;
  let parentCalls = 0, childCalls = 0;
  parent.token.onCancellationRequested(() => parentCalls++);
  childToken.onCancellationRequested(() => childCalls++);
  parent.cancel(); parent.cancel(); child.cancel();
  assert.equal(child.token, childToken);
  assert.equal(childToken.isCancellationRequested, true);
  assert.equal(parentCalls, 1); assert.equal(childCalls, 1);
  child.dispose(); parent.dispose();
  const independentParent = new CancellationTokenSource(), independentChild = new CancellationTokenSource(independentParent.token);
  independentChild.cancel();
  assert.equal(independentParent.token.isCancellationRequested, false);
  independentChild.dispose(); independentParent.dispose();
});

test('already canceled parent propagates on the next timer turn, never eagerly', t => {
  clock(t);
  const parent = new CancellationTokenSource(); parent.cancel();
  const child = new CancellationTokenSource(parent.token), token = child.token;
  let count = 0;
  token.onCancellationRequested(() => count++);
  assert.equal(token.isCancellationRequested, false);
  assert.equal(count, 0);
  t.mock.timers.tick(0);
  assert.equal(token.isCancellationRequested, true);
  assert.equal(count, 1);
  assert.equal(child.token, token);
  child.dispose(); parent.dispose();
});

test('child disposal detaches live and pending-late parent propagation', t => {
  clock(t);
  for (const alreadyCanceled of [false, true]) {
    const parent = new CancellationTokenSource();
    if (alreadyCanceled) parent.cancel();
    const child = new CancellationTokenSource(parent.token), token = child.token;
    let count = 0;
    token.onCancellationRequested(() => count++);
    child.dispose(); parent.cancel(); t.mock.timers.tick(0);
    assert.equal(token.isCancellationRequested, false);
    assert.equal(count, 0);
    child.cancel();
    assert.equal(token.isCancellationRequested, true);
    assert.equal(count, 0);
    parent.dispose();
  }
});

test('dispose(true) cancels existing token once and detaches parent', () => {
  const parent = new CancellationTokenSource(), source = new CancellationTokenSource(parent.token);
  let calls = 0;
  source.token.onCancellationRequested(() => calls++);
  source.dispose(true); source.dispose(true); parent.cancel();
  assert.equal(calls, 1);
  assert.equal(source.token.isCancellationRequested, true);
  parent.dispose();
});

test('listener removal during dispatch skips that listener and still delivers later live listeners', () => {
  const source = new CancellationTokenSource(), calls = [];
  let second;
  source.token.onCancellationRequested(() => { calls.push('first'); second.dispose(); source.cancel(); });
  second = source.token.onCancellationRequested(() => calls.push('removed'));
  source.token.onCancellationRequested(() => calls.push('third'));
  source.cancel();
  assert.deepEqual(calls, ['first', 'third']);
  source.dispose();
});

test('source disposal inside listener stops remaining delivery without resetting cancellation', () => {
  const source = new CancellationTokenSource(), calls = [];
  source.token.onCancellationRequested(() => { calls.push('first'); source.dispose(); });
  source.token.onCancellationRequested(() => calls.push('retired'));
  source.cancel();
  assert.deepEqual(calls, ['first']);
  assert.equal(source.token.isCancellationRequested, true);
});

test('delivery excludes cached-event additions but schedules fresh late subscriptions', t => {
  clock(t);
  const source = new CancellationTokenSource(), calls = [], cached = source.token.onCancellationRequested;
  cached(() => {
    calls.push('first');
    cached(() => calls.push('cached-added'));
    source.token.onCancellationRequested(() => calls.push('late-added'));
  });
  cached(() => calls.push('second'));
  source.cancel();
  assert.deepEqual(calls, ['first', 'second']);
  t.mock.timers.tick(0);
  assert.deepEqual(calls, ['first', 'second', 'late-added']);
  source.dispose();
});

test('throwing active listener is reported and does not starve remaining cancellation listeners', t => {
  const errors = [], thrown = new Error('controlled cancellation listener error');
  t.mock.method(console, 'error', error => errors.push(error));
  const source = new CancellationTokenSource(); let calls = 0;
  source.token.onCancellationRequested(() => { throw thrown; });
  source.token.onCancellationRequested(() => calls++);
  source.cancel(); source.cancel();
  assert.deepEqual(errors, [thrown]);
  assert.equal(calls, 1);
  assert.equal(source.token.isCancellationRequested, true);
  source.dispose();
});

test('independent sources share no listeners or cancellation state', () => {
  const first = new CancellationTokenSource(), second = new CancellationTokenSource();
  let firstCount = 0, secondCount = 0;
  for (let i = 0; i < 256; i++) {
    first.token.onCancellationRequested(() => firstCount++);
    second.token.onCancellationRequested(() => secondCount++);
  }
  first.cancel();
  assert.equal(firstCount, 256); assert.equal(secondCount, 0);
  assert.equal(second.token.isCancellationRequested, false);
  second.cancel();
  assert.equal(firstCount, 256); assert.equal(secondCount, 256);
  first.dispose(); second.dispose();
});

test('structural token check accepts valid objects without invoking their event', () => {
  let calls = 0;
  const custom = { isCancellationRequested: false, onCancellationRequested() { calls++; return { dispose() {} }; } };
  for (const value of [CancellationToken.None, CancellationToken.Cancelled, new CancellationTokenSource().token, custom]) {
    assert.equal(CancellationToken.isCancellationToken(value), true);
  }
  for (const value of [undefined, null, false, 0, '', {}, { isCancellationRequested: 0, onCancellationRequested() {} }]) {
    assert.equal(CancellationToken.isCancellationToken(value), false);
  }
  assert.equal(calls, 0);
});
