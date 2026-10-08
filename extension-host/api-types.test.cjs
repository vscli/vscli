'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { Position, Range, Selection, Uri, EventEmitter, TextDocument } = require('./api-types.cjs');

test('document coordinates use UTF-16 and preserve CRLF line boundaries', () => {
  const doc = new TextDocument({ uri: 'untitled:example', text: '猫🙂x\r\n  second\n', version: 1, languageId: 'plaintext', isDirty: true });
  assert.equal(doc.lineCount, 3);
  assert.equal(doc.offsetAt(new Position(0, 3)), 3);
  assert.deepEqual(doc.positionAt(5), new Position(0, 4));
  assert.deepEqual(doc.positionAt(6), new Position(1, 0));
  assert.equal(doc.getText(new Range(0, 1, 0, 3)), '🙂');
  assert.equal(doc.lineAt(1).firstNonWhitespaceCharacterIndex, 2);
  assert.deepEqual(doc.lineAt(0).rangeIncludingLineBreak.end, new Position(1, 0));
  assert.throws(() => doc.lineAt(3), RangeError);
});

test('selection retains direction while ranges normalize endpoints', () => {
  const selection = new Selection(3, 2, 1, 0);
  assert.equal(selection.isReversed, true);
  assert.deepEqual(selection.start, new Position(1, 0));
  assert.equal(selection.contains(new Position(2, 8)), true);
  assert.equal(selection.intersection(new Range(5, 0, 6, 0)), undefined);
  assert.deepEqual(new Position(2, 3).translate({ characterDelta: 2 }), new Position(2, 5));
  assert.throws(() => new Position(-1, 0), RangeError);
});

test('positions beyond the final line clamp to EOF and valid ranges retain identity', () => {
  const document = new TextDocument({ uri: 'untitled:test', text: 'first\nlast', version: 1 });
  assert.deepEqual(document.validatePosition(new Position(100, 0)), new Position(1, 4));
  assert.equal(document.offsetAt(new Position(100, 0)), 10);
  const range = new Range(0, 1, 1, 2);
  assert.equal(document.validateRange(range), range);
  assert.throws(() => document.validatePosition({ line: 1, character: 0 }), /Invalid position/);
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
