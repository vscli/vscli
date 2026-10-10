'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { sourceApiDiagnostics } = require('./save-code-actions.cjs');
function token(cancelled = false) {
  const listeners = new Set(); let disposals = 0;
  return { isCancellationRequested: cancelled,
    onCancellationRequested(listener) { listeners.add(listener);
      if (this.isCancellationRequested) listener();
      return { dispose() { if (listeners.delete(listener)) disposals += 1; } };
    },
    cancel() { this.isCancellationRequested = true; for (const listener of listeners) listener(); },
    inspect() { return { listeners: listeners.size, disposals }; }
  };
}
const input = { command: 'vscode.executeCodeActionProvider', resource: 'main.txt',
  uri: 'file:///fixture/main.txt', range: [0, 0, 5, 0], kind: 'source', itemResolveCount: 100,
  before: { text: '猫🙂\r\n', version: 1, dirty: false, selections: [[1, 0, 1, 0]], disk: '猫🙂\r\n' } };
const actions = () => ['source.organizeImports', 'source.fixAll.child', 'source.fixAll', 'source.fixAllX']
  .map((kind, index) => ({ title: `Original action ${index}`, kind: { value: kind } }));
async function invoke(recorder, api) {
  recorder.begin(input);
  let result;
  try { result = await api(); } catch (error) { recorder.rejected(error); throw error; }
  recorder.resolved(result); recorder.current(() => input.before);
  // Exact unchanged original count predicate; no diagnostic outcome retries.
  assert.ok(Array.isArray(result) && result.length === 4, 'Positive source-action API readiness failed');
  assert.ok(!recorder.proof().recordingError, 'Setup API diagnostic recording failed');
  return result;
}
test('one successful public API result and unchanged proof are retained without provider mutation', async () => {
  let time = 10, issued = 0; const recorder = sourceApiDiagnostics(() => time), current = token();
  const result = actions();
  assert.equal(await invoke(recorder, async () => { issued += 1; time = 20; recorder.callback(current); time = 30; return result; }), result);
  const raw = recorder.proof();
  assert.equal(issued, 1); assert.equal(raw.protocol, 'source-api-readiness-diagnostics-v1');
  assert.equal(raw.status, 'resolved'); assert.equal(raw.elapsedMs, 20);
  assert.deepEqual(raw.input, input); assert.deepEqual(raw.after, input.before);
  assert.deepEqual(raw.result, { type: 'array', count: 4, actions: result.map(value => ({ title: value.title, kind: value.kind.value, disabled: null })) });
  assert.deepEqual(raw.callbacks, [{ sequence: 1, elapsedMs: 10, cancelledAtEntry: false, cancelledAtEnd: false }]);
  assert.deepEqual(current.inspect(), { listeners: 0, disposals: 1 });
  recorder.dispose(); assert.equal(current.inspect().disposals, 1);
});
for (const [description, value, expected] of [
  ['short array', actions().slice(0, 2), { type: 'array', count: 2 }],
  ['undefined', undefined, { type: 'undefined' }], ['null', null, { type: 'null' }],
  ['non-array object', {}, { type: 'object' }]
]) {
  test(`actual ${description} is recorded before the unchanged assertion`, async () => {
    let issued = 0, targets = 0; const recorder = sourceApiDiagnostics(() => 0), current = token();
    await assert.rejects(async () => { await invoke(recorder, async () => { issued += 1; recorder.callback(current); return value; }); targets += 1; },
      /Positive source-action API readiness failed/);
    const raw = recorder.proof();
    for (const [key, field] of Object.entries(expected)) assert.equal(raw.result[key], field);
    assert.equal(issued, 1); assert.equal(targets, 0); assert.equal(raw.status, 'resolved');
    assert.equal(raw.callbacks[0].cancelledAtEnd, false); assert.equal(current.inspect().listeners, 0);
  });
}
test('late public cancellation differs from callback-entry state and survives failed count', async () => {
  let time = 100; const recorder = sourceApiDiagnostics(() => time), current = token();
  await assert.rejects(invoke(recorder, async () => { time = 120; recorder.callback(current); time = 135; current.cancel(); time = 140; return []; }),
    /Positive source-action API readiness failed/);
  const raw = recorder.proof();
  assert.deepEqual(raw.callbacks, [{ sequence: 1, elapsedMs: 20, cancelledAtEntry: false, cancelledAtEnd: true }]);
  assert.deepEqual(raw.cancellationEvents, [{ callback: 1, elapsedMs: 35 }]);
  assert.equal(raw.result.count, 0); current.cancel(); assert.equal(recorder.proof().cancellationEvents.length, 1);
  assert.deepEqual(current.inspect(), { listeners: 0, disposals: 1 });
});
test('already-cancelled synchronous subscription is owned and cleaned exactly once', () => {
  const recorder = sourceApiDiagnostics(() => 1), current = token(true);
  recorder.begin(input); recorder.callback(current); recorder.resolved([]);
  assert.deepEqual(recorder.proof().cancellationEvents, [{ callback: 1, elapsedMs: 0 }]);
  assert.equal(recorder.proof().callbacks[0].cancelledAtEnd, true);
  assert.deepEqual(current.inspect(), { listeners: 0, disposals: 1 });
});
test('API rejection and setup deadline preserve their original errors and issued record', async () => {
  for (const message of ['Actual API transport rejection', 'Combined save setup deadline exceeded']) {
    const original = new Error(message), recorder = sourceApiDiagnostics(() => 50), current = token(); let issued = 0;
    await assert.rejects(invoke(recorder, async () => { issued += 1; recorder.callback(current); throw original; }), error => error === original);
    const raw = recorder.proof(); assert.equal(raw.status, 'rejected'); assert.deepEqual(raw.input, input);
    assert.deepEqual(raw.error, { name: 'Error', message }); assert.equal(raw.result, undefined);
    assert.equal(issued, 1); assert.equal(current.inspect().listeners, 0); recorder.dispose();
  }
});
test('summary overflow retains real count and original failed assertion without silent truncation', async () => {
  const recorder = sourceApiDiagnostics(() => 0), value = Array.from({ length: 129 }, () => actions()[0]);
  await assert.rejects(invoke(recorder, async () => value), /Positive source-action API readiness failed/);
  const raw = recorder.proof(); assert.equal(raw.result.count, 129);
  assert.equal(raw.result.actions, undefined); assert.match(raw.recordingError, /summary exceeds bounds/);
  assert.ok(Buffer.byteLength(JSON.stringify(raw)) <= raw.limits.maxBytes);
});
test('callback and cancellation budgets latch failure and release all owned subscriptions', () => {
  const recorder = sourceApiDiagnostics(() => 0), current = token(); recorder.begin(input);
  for (let index = 0; index < 257; index += 1) recorder.callback(current);
  assert.match(recorder.proof().recordingError, /record budget exceeded/);
  assert.equal(recorder.proof().callbacks.length, 256);
  recorder.rejected(new Error('Original failure')); recorder.dispose();
  assert.deepEqual(current.inspect(), { listeners: 0, disposals: 256 });
  const second = sourceApiDiagnostics(() => 0), repeating = token(); second.begin(input); second.callback(repeating);
  for (let index = 0; index < 513; index += 1) repeating.cancel();
  assert.equal(second.proof().cancellationEvents.length, 512);
  assert.match(second.proof().recordingError, /record budget exceeded/);
  second.dispose(); assert.equal(repeating.inspect().listeners, 0);
});
test('diagnostic snapshot/getter failures never substitute for actual API error', async () => {
  const recorder = sourceApiDiagnostics(() => 0), original = new Error('Actual API failure');
  const current = token(); recorder.begin(input); recorder.callback(current);
  recorder.current(() => { throw new Error('Secondary snapshot failed'); });
  recorder.rejected(original);
  assert.equal(recorder.proof().error.message, original.message);
  assert.match(recorder.proof().recordingError, /Secondary snapshot failed/);
  assert.equal(current.inspect().listeners, 0);
});
test('idle/target callbacks and cleanup do not create setup authority or retain tokens', () => {
  const recorder = sourceApiDiagnostics(() => 0), current = token(); recorder.callback(current);
  assert.equal(recorder.proof().callbacks.length, 0);
  recorder.begin(input); recorder.resolved(actions()); recorder.callback(current); recorder.dispose();
  assert.equal(recorder.proof().callbacks.length, 0); assert.deepEqual(current.inspect(), { listeners: 0, disposals: 0 });
});

test('oversized public result fields remain explicit and never authorize targets', async () => {
  const recorder = sourceApiDiagnostics(() => 0), result = actions(); result[0].title = 'x'.repeat(1025);
  let targets = 0;
  await assert.rejects(async () => { await invoke(recorder, async () => result); targets += 1; },
    /Setup API diagnostic recording failed/);
  const raw = recorder.proof(); assert.equal(raw.result.type, 'array'); assert.equal(raw.result.count, 4);
  assert.equal(raw.result.actions, undefined); assert.match(raw.recordingError, /string exceeds bounds/);
  assert.equal(targets, 0); assert.ok(Buffer.byteLength(JSON.stringify(raw)) <= raw.limits.maxBytes);
});

test('setup boundary timeout never invents API rejection and ignores late settlement', () => {
  let time = 100; const recorder = sourceApiDiagnostics(() => time), current = token();
  recorder.begin(input); recorder.callback(current); time = 15100;
  recorder.interrupted(new Error('Combined save setup deadline exceeded'));
  const interrupted = recorder.proof();
  assert.equal(interrupted.status, 'interrupted'); assert.equal(interrupted.apiSettlement, 'unobserved');
  assert.equal(interrupted.result, undefined); assert.equal(interrupted.error, undefined);
  assert.equal(interrupted.boundaryError.message, 'Combined save setup deadline exceeded');
  assert.equal(current.inspect().listeners, 0);
  recorder.resolved(actions()); recorder.rejected(new Error('Late actual rejection')); current.cancel();
  assert.deepEqual(recorder.proof(), interrupted);
  const neverIssued = sourceApiDiagnostics(() => 0);
  neverIssued.interrupted(new Error('Combined save setup deadline exceeded'));
  assert.equal(neverIssued.proof().status, 'not-issued'); assert.equal(neverIssued.proof().input, undefined);
});
