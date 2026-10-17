'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { Position, Range, Selection, Uri, Disposable, EventEmitter, TextDocument } = require('./api-types.cjs');

test('document coordinates use UTF-16 and preserve CRLF line boundaries', () => {
  const doc = new TextDocument({ uri: 'untitled:example', text: '猫🙂x\r\n  second\n', version: 1, languageId: 'plaintext', isDirty: true });
  assert.equal(doc.lineCount, 3);
  assert.equal(doc.offsetAt(new Position(0, 3)), 3);
  assert.deepEqual(doc.positionAt(5), new Position(0, 4));
  assert.deepEqual(doc.positionAt(6), new Position(1, 0));
  assert.equal(doc.getText(new Range(0, 1, 0, 3)), '🙂');
  assert.equal(doc.lineAt(1).firstNonWhitespaceCharacterIndex, 2);
  assert.deepEqual(doc.lineAt(0).rangeIncludingLineBreak.end, new Position(1, 0));
  assert.throws(() => doc.lineAt(3), /^Error: Illegal value for `line`$/);
});

test('selection retains direction while ranges normalize endpoints', () => {
  const selection = new Selection(3, 2, 1, 0);
  assert.equal(selection.isReversed, true);
  assert.deepEqual(selection.start, new Position(1, 0));
  assert.equal(selection.contains(new Position(2, 8)), true);
  assert.equal(selection.intersection(new Range(5, 0, 6, 0)), undefined);
  assert.deepEqual(new Position(2, 3).translate({ characterDelta: 2 }), new Position(2, 5));
  assert.throws(() => new Position(-1, 0), /line must be non-negative/);
});

test('positions beyond the final line clamp to EOF and valid ranges retain identity', () => {
  const document = new TextDocument({ uri: 'untitled:test', text: 'first\nlast', version: 1 });
  assert.deepEqual(document.validatePosition(new Position(100, 0)), new Position(1, 4));
  assert.equal(document.offsetAt(new Position(100, 0)), 10);
  const range = new Range(0, 1, 1, 2);
  assert.equal(document.validateRange(range), range);
  assert.throws(() => document.validatePosition({ line: 1, character: 0 }), /^Error: Invalid argument$/);
});

test('file URI round trips spaces Unicode and reserved filename characters', () => {
  const file = path.resolve('folder with space', '猫#%.rs');
  const uri = Uri.file(file);
  assert.equal(Uri.parse(uri.toString()).fsPath, file);
  assert.match(uri.toString(), /%23%25/);
  assert.equal(Uri.joinPath(Uri.file(path.dirname(file)), 'other.rs').fsPath, path.join(path.dirname(file), 'other.rs'));
  assert.equal(Uri.joinPath(Uri.file(path.dirname(file)), 'other#%.rs').fsPath, path.join(path.dirname(file), 'other#%.rs'));
  assert.equal(Uri.from(uri.toJSON()).fsPath, file);
});

test('document updates are visible synchronously inside change callbacks', () => {
  const snapshot = { uri: 'untitled:example', text: 'before', version: 1, languageId: 'plaintext', isDirty: true };
  const doc = new TextDocument(snapshot);
  const failures = [];
  const emitter = new EventEmitter(error => failures.push(error));
  let seen = '';
  const subscription = emitter.event(() => { seen = doc.getText(); assert.equal(doc.version, 2); });
  emitter.event(() => { throw new Error('listener failure'); });
  doc._update({ ...snapshot, text: 'after', version: 2 });
  emitter.fire({ document: doc });
  assert.equal(seen, 'after');
  assert.equal(failures.length, 1);
  subscription.dispose(); subscription.dispose();
  seen = '';
  emitter.fire({ document: doc });
  assert.equal(seen, '');
});

test('Position overloads preserve unchanged identity and normalize nonnumeric object fields', () => {
  const position = new Position(4, 7);
  for (const unchanged of [position.translate(), position.translate(0, 0), position.translate({}),
    position.translate({ lineDelta: '2', characterDelta: null }), position.with(),
    position.with(4, 7), position.with({}), position.with({ line: '2', character: null })]) {
    assert.equal(unchanged, position);
  }
  assert.deepEqual(position.translate(undefined, 2), new Position(4, 9));
  assert.deepEqual(position.with(undefined, 2), new Position(4, 2));
  assert.deepEqual(position.translate({ lineDelta: -1, characterDelta: 3 }), new Position(3, 10));
  assert.deepEqual(position.with({ character: 0 }), new Position(4, 0));
  for (const call of [() => position.translate(null), () => position.translate(0, null),
    () => position.with(null), () => position.with(4, null)]) {
    assert.throws(call, { name: 'Error', message: 'Illegal argument' });
  }
  assert.deepEqual(position.toJSON(), { line: 4, character: 7 });
  assert.throws(() => { position.line = 2; }, TypeError);
  const missing = new Position(undefined, undefined);
  assert.equal(missing.with(), missing);
});

test('extension Position values follow source numeric admission without becoming wire authority', () => {
  const fractional = new Position(0.5, 1.25);
  assert.equal(fractional.line, 0.5);
  assert.equal(fractional.character, 1.25);
  const nan = new Position(NaN, NaN);
  assert.ok(Number.isNaN(nan.line));
  assert.equal(nan.compareTo(new Position(1, 1)), 0);
  assert.equal(nan.isBefore(new Position(1, 1)), false);
  assert.equal(nan.isBeforeOrEqual(new Position(1, 1)), false);
  assert.equal(nan.isAfter(new Position(1, 1)), true);
  assert.equal(nan.isAfterOrEqual(new Position(1, 1)), true);
  assert.equal(nan.isEqual(nan), false);
  assert.equal(new Position(Infinity, 0).isAfter(new Position(1, 1)), true);
  assert.throws(() => new Position(-1, 0), { name: 'Error', message: 'Illegal argument: line must be non-negative' });
  assert.throws(() => new Position(0, -1), { name: 'Error', message: 'Illegal argument: character must be non-negative' });
});

test('Position helpers preserve instances and stable ties while rejecting absent shapes', () => {
  const first = new Position(1, 2), same = new Position(1, 2), last = new Position(3, 0);
  assert.equal(Position.of(first), first);
  assert.deepEqual(Position.of({ line: 1, character: 2 }), first);
  assert.equal(Position.Min(first, same, last), first);
  assert.equal(Position.Max(last, first), last);
  assert.equal(Position.isPosition({ line: NaN, character: Infinity }), true);
  assert.equal(Position.isPosition({ line: '1', character: 2 }), false);
  assert.equal(Position.isPosition(null), false);
  assert.throws(() => Position.of({ line: 1 }), /NOT a position-like object/);
  assert.throws(() => Position.Min(), TypeError);
  assert.throws(() => Position.Max(), TypeError);
});

test('Range preserves positions, immutable endpoints and unchanged derivation identity', () => {
  const start = new Position(1, 2), end = new Position(4, 3), range = new Range(start, end);
  assert.equal(range.start, start);
  assert.equal(range.end, end);
  assert.equal(Range.of(range), range);
  assert.equal(range.with(), range);
  assert.equal(range.with({}), range);
  assert.equal(range.with(new Position(1, 2), new Position(4, 3)), range);
  assert.equal(range.with({ start: new Position(1, 2) }), range);
  assert.equal(range.union(new Range(2, 0, 3, 0)), range);
  const outer = new Range(0, 0, 5, 0);
  assert.equal(range.union(outer), outer);
  assert.equal(range.union(new Range(1, 2, 4, 3)), range);
  assert.throws(() => { range.start = end; }, TypeError);
  assert.equal(range.start, start);
  assert.equal(new Range(end, start).start, start);
  assert.deepEqual(JSON.parse(JSON.stringify(range)), [{ line: 1, character: 2 }, { line: 4, character: 3 }]);
});

test('Range changes support either endpoint and reject invalid construction or explicit null', () => {
  const range = new Range(0, 0, 2, 3), newEnd = new Position(3, 4);
  assert.equal(range.with(undefined, newEnd).end, newEnd);
  assert.equal(range.with({ end: newEnd }).end, newEnd);
  assert.deepEqual(Range.of({ start: { line: 0, character: 0 }, end: { line: 2, character: 3 } }), range);
  for (const call of [() => range.with(null), () => range.with(undefined, null)]) {
    assert.throws(call, { name: 'Error', message: 'Illegal argument' });
  }
  for (const args of [[], [1, 2], [null, null], [{ line: 0 }, { line: 1, character: 2 }]]) {
    assert.throws(() => new Range(...args), { name: 'Error', message: 'Invalid arguments' });
  }
  for (const invalid of [null, undefined, {}, 0, 'text', { start: new Position(0, 0) }]) {
    assert.equal(range.contains(invalid), false);
    assert.equal(Range.isRange(invalid), false);
  }
  assert.throws(() => Range.of({}), /NOT a range-like object/);
  const callable = () => {};
  Object.assign(callable, { start: range.start, end: range.end });
  assert.equal(Range.isRange(callable), false); // Stable 1.141 object-only guard.
  assert.equal(range.contains(new Position(2, 3)), true);
  const edge = range.intersection(new Range(2, 3, 4, 0));
  assert.equal(edge.isEmpty, true);
  assert.deepEqual(edge.start, new Position(2, 3));
  assert.equal(range.intersection(new Range(2, 4, 4, 0)), undefined);
});

test('Selection preserves directional endpoint identity and source equal-endpoint behavior', () => {
  const anchor = new Position(2, 4), active = new Position(0, 1);
  const selection = new Selection(anchor, active);
  assert.equal(selection.anchor, anchor);
  assert.equal(selection.active, active);
  assert.equal(selection.start, active);
  assert.equal(selection.end, anchor);
  assert.equal(selection.isReversed, true);
  assert.equal(new Selection(active, anchor).isReversed, false);
  assert.equal(new Selection(new Position(0, 0), new Position(0, 0)).isReversed, true);
  assert.equal(new Selection(active, active).isReversed, true);
  assert.equal(selection.with(), selection);
  assert.equal(selection.with({ end: new Position(4, 0) }) instanceof Selection, false);
  assert.equal(Selection.isSelection(selection), true);
  assert.equal(Selection.isSelection({ start: active, end: anchor, anchor, active, isReversed: true }), true);
  assert.equal(Selection.isSelection({ start: active, end: anchor, anchor, active }), false);
  assert.equal(Selection.isSelection(null), false);
  const callable = () => {};
  Object.assign(callable, selection.toJSON(), { isReversed: true });
  assert.equal(Selection.isSelection(callable), false);
  assert.throws(() => new Selection(0, 0), { name: 'Error', message: 'Invalid arguments' });
  assert.throws(() => { selection.active = anchor; }, TypeError);
  assert.deepEqual(JSON.parse(JSON.stringify(selection)), {
    start: { line: 0, character: 1 }, end: { line: 2, character: 4 },
    active: { line: 0, character: 1 }, anchor: { line: 2, character: 4 },
  });
});

test('Disposable invokes callbacks once on success with disposable receiver and no promise waiting', () => {
  let calls = 0, receiver;
  const pending = new Promise(() => {});
  const disposable = new Disposable(function () { calls++; receiver = this; return pending; });
  assert.equal(disposable.dispose(), undefined);
  disposable.dispose();
  assert.equal(calls, 1);
  assert.equal(receiver, disposable);
  assert.doesNotThrow(() => new Disposable(undefined).dispose());
  assert.doesNotThrow(() => new Disposable({}).dispose());
});

test('Disposable keeps throwing callback retryable and from preserves ordered stop-on-error semantics', () => {
  let calls = 0;
  const single = new Disposable(() => { if (++calls === 1) throw new Error('retry cleanup'); });
  assert.throws(() => single.dispose(), /retry cleanup/);
  single.dispose(); single.dispose();
  assert.equal(calls, 2);
  const seen = [], first = { dispose() { assert.equal(this, first); seen.push('first'); } };
  let fail = true;
  const all = Disposable.from(null, {}, first, { dispose() { seen.push('second'); if (fail) throw new Error('stop'); } },
    { dispose() { seen.push('third'); } });
  assert.throws(() => all.dispose(), /stop/);
  assert.deepEqual(seen, ['first', 'second']);
  fail = false; all.dispose(); all.dispose();
  assert.deepEqual(seen, ['first', 'second', 'first', 'second', 'third']);
  assert.doesNotThrow(() => Disposable.from().dispose());
});

test('EventEmitter removes pending listeners and does not deliver new listeners the current event', () => {
  const emitter = new EventEmitter(), seen = [];
  let second, added = false;
  emitter.event(value => {
    seen.push(`first:${value}`);
    second.dispose();
    if (!added) { added = true; emitter.event(next => seen.push(`new:${next}`)); }
  });
  second = emitter.event(value => seen.push(`second:${value}`));
  emitter.fire(1); emitter.fire(2);
  assert.deepEqual(seen, ['first:1', 'first:2', 'new:2']);
});

test('EventEmitter drains the outstanding outer delivery before a nested fire', () => {
  const emitter = new EventEmitter(), seen = [];
  emitter.event(value => { seen.push(`first:${value}`); if (value === 'outer') emitter.fire('inner'); });
  emitter.event(value => seen.push(`second:${value}`));
  emitter.event(value => seen.push(`third:${value}`));
  emitter.fire('outer');
  assert.deepEqual(seen, ['first:outer', 'second:outer', 'third:outer', 'first:inner', 'second:inner', 'third:inner']);
  emitter.fire('later');
  assert.deepEqual(seen.slice(-3), ['first:later', 'second:later', 'third:later']);
});

test('EventEmitter disposal during delivery prevents remaining and all later registrations', () => {
  const emitter = new EventEmitter(), seen = [], owned = [];
  emitter.event(() => { seen.push('first'); emitter.dispose(); });
  const second = emitter.event(() => seen.push('second'), undefined, owned);
  emitter.fire(1);
  assert.deepEqual(seen, ['first']);
  const event = emitter.event;
  const late = event(() => seen.push('late'), undefined, owned);
  assert.equal(owned.length, 1); // Disposed-emitter inert subscription is not appended upstream.
  late.dispose(); second.dispose(); emitter.dispose(); emitter.fire(2);
  assert.deepEqual(seen, ['first']);
});

test('EventEmitter duplicate subscriptions dispose independently and use original thisArg semantics', () => {
  const emitter = new EventEmitter(), owned = [], seen = [], receiver = { name: 'owner' };
  function listener(value) { seen.push([this, value]); }
  const first = emitter.event(listener, receiver, owned), second = emitter.event(listener, receiver, owned);
  assert.equal(owned[0], first); assert.equal(owned[1], second);
  first.dispose(); first.dispose(); emitter.fire('one');
  assert.deepEqual(seen, [[receiver, 'one']]);
  second.dispose();
  let falsyReceiver = 'not called';
  emitter.event(function () { falsyReceiver = this; }, false);
  assert.doesNotThrow(() => emitter.fire('two'));
  assert.equal(falsyReceiver, undefined);
});

test('EventEmitter listener exceptions remain isolated and options error reporting remains synchronous', () => {
  const errors = [], seen = [], emitter = new EventEmitter({ onListenerError: error => errors.push(error) });
  emitter.event(() => { throw new Error('first failed'); });
  emitter.event(value => seen.push(value));
  emitter.fire(1); emitter.fire(2);
  assert.deepEqual(seen, [1, 2]);
  assert.deepEqual(errors.map(error => error.message), ['first failed', 'first failed']);
  emitter.dispose();
});
