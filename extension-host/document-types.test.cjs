'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { Position, Range, Uri } = require('./api-types.cjs');
const { createDocumentTypes } = require('./document-types.cjs');
const { TextDocument } = createDocumentTypes({ Position, Range, Uri });
const snapshot = (text, extra = {}) => ({ id: 7, uri: 'file:///tmp/document.cpp', languageId: 'cpp', version: 1, isDirty: true, text, ...extra });
const word = (document, position, expression) => {
  const range = document.getWordRangeAtPosition(position, expression);
  return range && [range.start.line, range.start.character, range.end.line, range.end.character, document.getText(range)];
};

test('UTF16 offsets retain surrogate interior and both CRLF interior offsets clamp at line end', () => {
  const doc = new TextDocument(snapshot('a🙂猫\r\nb\r\n'));
  assert.equal(doc.eol, 2);
  assert.equal(doc.lineCount, 3);
  for (const [offset, line, character] of [[0,0,0],[2,0,2],[4,0,4],[5,0,4],[6,1,0],[7,1,1],[8,1,1],[9,2,0],[100,2,0],[-1,0,0]]) {
    assert.deepEqual(doc.positionAt(offset).toJSON(), { line, character });
  }
  assert.equal(doc.offsetAt(new Position(1, 0)), 6);
  assert.equal(doc.getText(new Range(0, 1, 1, 1)), '🙂猫\r\nb');
});

test('validated original objects retain identity, out-of-line positions clamp to final end', () => {
  const doc = new TextDocument(snapshot('ab\ncd'));
  const position = new Position(0, 1), range = new Range(0, 0, 1, 1);
  assert.equal(doc.validatePosition(position), position);
  assert.equal(doc.validateRange(range), range);
  assert.deepEqual(doc.validatePosition(new Position(99, 0)).toJSON(), { line: 1, character: 2 });
  assert.deepEqual(doc.validatePosition(new Position(0, 99)).toJSON(), { line: 0, character: 2 });
  assert.throws(() => doc.validatePosition({ line: 0, character: 1 }), /^Error: Invalid argument$/);
  assert.throws(() => doc.validateRange({ start: position, end: position }), /^Error: Invalid argument$/);
});

test('lineAt requires Position or integer and TextLine remains an immutable snapshot', () => {
  const doc = new TextDocument(snapshot(' \t\u00a0猫\n'));
  const line = doc.lineAt(new Position(0, 3));
  assert.equal(line.firstNonWhitespaceCharacterIndex, 3);
  assert.equal(line.isEmptyOrWhitespace, false);
  assert.deepEqual(line.rangeIncludingLineBreak.end.toJSON(), { line: 1, character: 0 });
  assert.equal(doc.lineAt(1).isEmptyOrWhitespace, true);
  assert.deepEqual(doc.lineAt(1).range.end.toJSON(), doc.lineAt(1).rangeIncludingLineBreak.end.toJSON());
  for (const value of [null, undefined, { line: 0 }, -1, 0.5, 2, NaN]) assert.throws(() => doc.lineAt(value), /Illegal value for `line`/);
  doc._update({ version: 2, text: 'changed' });
  assert.equal(line.text, ' \t\u00a0猫');
});

test('explicit model EOL normalizes public text/indexes without mutating native snapshot bytes', () => {
  const raw = 'one\r\ntwo\nthree\rfour';
  const doc = new TextDocument(snapshot(raw, { eol: 1 }));
  assert.equal(doc.getText(), 'one\ntwo\nthree\nfour');
  assert.equal(doc._snapshot.text, raw);
  assert.equal(doc.offsetAt(new Position(2, 0)), 8);
  doc._update({ eol: 2 });
  assert.equal(doc.getText(), 'one\r\ntwo\r\nthree\r\nfour');
  assert.equal(doc.offsetAt(new Position(2, 0)), 10);
});

test('failed metadata update publishes neither bytes nor URI nor previous version', () => {
  const doc = new TextDocument(snapshot('safe'));
  assert.throws(() => doc._update({ text: 'lost', version: 2, eol: 99 }), /Invalid document EOL snapshot/);
  assert.equal(doc.version, 1); assert.equal(doc.getText(), 'safe');
  assert.throws(() => doc._update({ text: 'lost', version: 2, uri: 'bad scheme:/lost' }));
  assert.equal(doc.version, 1); assert.equal(doc.getText(), 'safe');
});

test('default words include unicode and decimal special case, exclude separators', () => {
  const doc = new TextDocument(snapshot('foo.bar -12.75x 猫🙂'));
  assert.deepEqual(word(doc, new Position(0, 1)), [0,0,0,3,'foo']);
  assert.deepEqual(word(doc, new Position(0, 5)), [0,4,0,7,'bar']);
  assert.deepEqual(word(doc, new Position(0, 12)), [0,8,0,15,'-12.75x']);
  assert.deepEqual(word(doc, new Position(0, 17)), [0,16,0,19,'猫🙂']);
});

test('word lookup includes end boundary and gives preceding touching match precedence', () => {
  const doc = new TextDocument(snapshot('foo bar'));
  assert.deepEqual(word(doc, new Position(0, 3)), [0,0,0,3,'foo']);
  assert.deepEqual(word(doc, new Position(0, 4)), [0,4,0,7,'bar']);
  assert.deepEqual(word(new TextDocument(snapshot('ab')), new Position(0, 1), /./g), [0,0,0,1,'a']);
});

test('custom regex flags follow upstream global normalization and reset successful global state', () => {
  const doc = new TextDocument(snapshot('AbCd'));
  const regex = /[a-z]+/gi; regex.lastIndex = 99;
  assert.deepEqual(word(doc, new Position(0, 2), regex), [0,0,0,4,'AbCd']);
  assert.equal(regex.lastIndex, 0);
  const nonglobal = /[a-z]+/i;
  assert.deepEqual(word(doc, new Position(0, 2), nonglobal), [0,0,0,4,'AbCd']);
});

test('word provider uses current language after metadata-only update, explicit regex takes precedence', () => {
  const seen = [];
  const doc = new TextDocument(snapshot('foo-bar'), { wordDefinition(language) { seen.push(language); return language === 'cpp' ? /[\w-]+/g : /\w+/g; } });
  assert.deepEqual(word(doc, new Position(0, 5)), [0,0,0,7,'foo-bar']);
  doc._update({ languageId: 'other' });
  assert.deepEqual(word(doc, new Position(0, 5)), [0,4,0,7,'bar']);
  assert.deepEqual(word(doc, new Position(0, 5), /[\w-]+/g), [0,0,0,7,'foo-bar']);
  assert.deepEqual(seen, ['cpp', 'other']);
});

test('empty regex is rejected but upstream anchors are accepted; contextual no-progress is explicit adaptation', () => {
  const doc = new TextDocument(snapshot('x y'));
  assert.throws(() => doc.getWordRangeAtPosition(new Position(0, 1), /a*/), /matches the empty string/);
  assert.deepEqual(word(doc, new Position(0, 0), /^/), [0,0,0,0,'']);
  assert.throws(() => doc.getWordRangeAtPosition(new Position(0, 2), /(?=x)/g), /made no progress/);
});

test('long word window preserves actual UTF16 source coordinates', () => {
  const doc = new TextDocument(snapshot(' '.repeat(1600) + 'hello' + ' '.repeat(1600)));
  assert.deepEqual(word(doc, new Position(0, 1602)), [0,1600,0,1605,'hello']);
  const all = new TextDocument(snapshot('a'.repeat(2000)));
  assert.deepEqual(word(all, new Position(0, 1500)), [0,1001,0,2000,'a'.repeat(999)]);
});

test('closed mirrors remain readable and clean, and reject save before consulting gateway', async () => {
  let calls = 0;
  const doc = new TextDocument(snapshot('retained'), { save() { calls++; return true; } });
  doc._update({ isClosed: true });
  assert.equal(doc.getText(), 'retained'); assert.equal(doc.isDirty, false);
  await assert.rejects(doc.save(), /Document has been closed/); assert.equal(calls, 0);
});

test('save waits gateway terminal receipt; successful old snapshot does not erase newer dirty mirror', async () => {
  let release, called;
  const doc = new TextDocument(snapshot('before'), { save(document) { called = document; return new Promise(resolve => { release = resolve; }); } });
  let settled = false; const promise = doc.save().then(value => { settled = true; return value; });
  await Promise.resolve(); await Promise.resolve();
  assert.equal(called, doc); assert.equal(settled, false); assert.equal(doc.isDirty, true);
  doc._update({ version: 2, text: 'newer', isDirty: true, savedGeneration: 1 });
  release(true); assert.equal(await promise, true);
  assert.equal(doc.getText(), 'newer'); assert.equal(doc.isDirty, true); assert.equal(doc.version, 2);
});

test('save cancellation false/error and absent gateway never mutate mirror state', async () => {
  const missing = new TextDocument(snapshot('safe'));
  await assert.rejects(missing.save(), /gateway is not configured/);
  const cancelled = new TextDocument(snapshot('safe'), { save: () => false });
  assert.equal(await cancelled.save(), false); assert.equal(cancelled.isDirty, true);
  const bad = new TextDocument(snapshot('safe'), { save: () => ({ ok: true }) });
  await assert.rejects(bad.save(), /Invalid native document save acknowledgement/);
  const failed = new TextDocument(snapshot('safe'), { save() { throw new Error('denied'); } });
  await assert.rejects(failed.save(), /denied/); assert.equal(failed.getText(), 'safe');
});

test('successful clean mirror publication keeps the shared object and current encoding', async () => {
  const doc = new TextDocument(snapshot('same', { encoding: 'utf8bom' }), { save(document) { document._update({ isDirty: false, savedGeneration: 1 }); return true; } });
  const identity = doc;
  assert.equal(doc.encoding, 'utf8bom'); assert.equal(await doc.save(), true);
  assert.equal(doc, identity); assert.equal(doc.isDirty, false);
});
