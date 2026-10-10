'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { configurationDiagnostics } = require('./save-code-actions.cjs');
const plain = value => JSON.parse(JSON.stringify(value));

function fixture() {
  const file = path.join(path.parse(process.cwd()).root, 'diagnostic-fixture', 'main.txt');
  const uri = { scheme: 'file', fsPath: file, toString: () => pathToFileURL(file).href };
  const document = { uri, languageId: 'plaintext', version: 1 };
  const values = { editor: { codeActionsOnSave: { 'source.fixAll': 'explicit', 'source.fixAll.child': false },
    formatOnSave: false }, files: { autoSave: 'off', autoSaveDelay: 500 } };
  let listener, disposed = 0, phase = 'setup', time = 0;
  const vscode = { workspace: {
    getConfiguration(section, scope) {
      assert.equal(scope.uri, uri); assert.equal(scope.languageId, 'plaintext');
      return { get(key) { return values[section][key]; }, inspect(key) {
        return { key: `${section}.${key}`, defaultValue: key === 'codeActionsOnSave' ? {} : undefined,
          globalValue: values[section][key] };
      } };
    },
    onDidChangeConfiguration(callback) { listener = callback; return { dispose() { disposed += 1; listener = undefined; } }; }
  }, commands: { executeCommand() { assert.fail('Recorder must not execute a target command'); } } };
  const recorder = configurationDiagnostics(vscode, uri, () => phase, () => time);
  return { recorder, document, values, setPhase(value) { phase = value; }, setTime(value) { time = value; },
    event(affected) {
      listener?.({ affectsConfiguration(key, scope) {
        assert.equal(scope.uri, uri); assert.equal(scope.languageId, 'plaintext');
        return affected.includes(key);
      } });
    }, disposed: () => disposed };
}
const context = { only: 'source.fixAll', triggerKind: 2, cancelled: false };

test('pre-open migration event and callback/save configurations remain separately ordered', () => {
  const f = fixture();
  try {
    f.values.editor.codeActionsOnSave['source.fixAll.child'] = 'never';
    f.setTime(12); f.event(['editor.codeActionsOnSave']);
    f.recorder.bind(f.document); f.setPhase('target');
    f.document.version = 3; f.setTime(20); f.recorder.provider(f.document, context);
    f.document.version = 4; f.setTime(30); f.recorder.saved(f.document);
    const proof = plain(f.recorder.finish());
    assert.equal(proof.initial.effective.codeActionsOnSave['source.fixAll.child'], false);
    assert.equal(proof.initial.open, false);
    assert.deepEqual(proof.records.map(row => [row.sequence, row.origin, row.phase, row.elapsedMs]),
      [[1, 'configuration-change', 'setup', 12], [2, 'source-provider', 'target', 20], [3, 'did-save', 'target', 30]]);
    assert.equal(proof.records[0].snapshot.open, false);
    for (const row of proof.records) {
      assert.equal(row.snapshot.resource, 'main.txt');
      assert.equal(row.snapshot.languageId, 'plaintext');
      assert.equal(row.snapshot.effective.codeActionsOnSave['source.fixAll.child'], 'never');
      assert.equal(row.snapshot.inspect.codeActionsOnSave.globalValue['source.fixAll.child'], 'never');
    }
    assert.deepEqual(proof.records[1].context, context);
    assert.equal(proof.records[1].snapshot.exactDocument, true);
    assert.equal(proof.records[2].snapshot.version, 4);
    assert.equal(proof.records[2].context, null);
  } finally { f.recorder.dispose(); }
});

test('migration after provider and before didSave cannot rewrite retained false evidence', () => {
  const f = fixture();
  try {
    f.recorder.bind(f.document); f.setPhase('target');
    f.recorder.provider(f.document, context);
    f.values.editor.codeActionsOnSave['source.fixAll.child'] = 'never';
    f.event(['editor.codeActionsOnSave']); f.recorder.saved(f.document);
    const proof = plain(f.recorder.finish());
    assert.equal(proof.records[0].snapshot.effective.codeActionsOnSave['source.fixAll.child'], false);
    assert.equal(proof.records[0].snapshot.inspect.codeActionsOnSave.globalValue['source.fixAll.child'], false);
    assert.equal(proof.records[1].snapshot.effective.codeActionsOnSave['source.fixAll.child'], 'never');
    assert.equal(proof.records[2].snapshot.effective.codeActionsOnSave['source.fixAll.child'], 'never');
  } finally { f.recorder.dispose(); }
});

test('all relevant target-scope keys are retained together while unrelated events add no row', () => {
  const f = fixture();
  try {
    f.event(['window.zoomLevel']);
    f.event(['files.autoSave', 'files.autoSaveDelay', 'editor.formatOnSave']);
    const proof = plain(f.recorder.finish());
    assert.equal(proof.records.length, 1);
    assert.deepEqual(proof.records[0].context.affected,
      ['editor.formatOnSave', 'files.autoSave', 'files.autoSaveDelay']);
  } finally { f.recorder.dispose(); }
});

for (const variant of ['resource', 'language', 'object', 'version']) {
  test(`foreign ${variant} provider identity rejects the entire proof`, () => {
    const f = fixture();
    try {
      f.recorder.bind(f.document);
      const foreign = variant === 'version' ? f.document : { ...f.document };
      const expected = ({ resource: /document resource differs/, language: /document language differs/,
        object: /document object differs/, version: /document version exceeds bounds/ })[variant];
      if (variant === 'resource') foreign.uri = { toString: () => 'file:///other/main.txt' };
      if (variant === 'language') foreign.languageId = 'cpp';
      if (variant === 'version') foreign.version = 2147483648;
      assert.throws(() => f.recorder.provider(foreign, context), expected);
      assert.throws(() => f.recorder.finish(), expected);
    } finally { f.recorder.dispose(); }
  });
}

test('swallowed event overflow is latched and cannot return truncated evidence', () => {
  const f = fixture();
  try {
    for (let index = 0; index < 129; index += 1) f.event(['editor.codeActionsOnSave']);
    assert.throws(() => f.recorder.finish(), /Diagnostic record budget exceeded/);
    assert.throws(() => f.recorder.provider(f.document, context), /Diagnostic record budget exceeded/);
  } finally { f.recorder.dispose(); }
});

test('oversized, circular and deep configuration values fail without a filtered snapshot', () => {
  for (const variant of ['bytes', 'circle', 'depth', 'nodes']) {
    const f = fixture();
    try {
      const value = f.values.editor.codeActionsOnSave;
      if (variant === 'bytes') value.extra = 'x'.repeat(32768);
      if (variant === 'circle') value.extra = value;
      if (variant === 'depth') { let current = value; for (let index = 0; index < 10; index += 1) current = current.extra = {}; }
      if (variant === 'nodes') value.extra = Array(1025).fill(false);
      assert.throws(() => f.recorder.provider(f.document, context), /Diagnostic configuration value exceeds bounds|Invalid diagnostic value/);
      assert.throws(() => f.recorder.finish(), /Diagnostic configuration value exceeds bounds|Invalid diagnostic value/);
    } finally { f.recorder.dispose(); }
  }
});

test('aggregate metadata bytes are bounded independently of record count', () => {
  const f = fixture();
  try {
    f.values.editor.codeActionsOnSave.extra = 'x'.repeat(16000);
    let error;
    for (let index = 0; index < 64; index += 1) {
      try { f.recorder.provider(f.document, context); } catch (caught) { error = caught; break; }
    }
    assert.match(error?.message || '', /Diagnostic total byte budget exceeded/);
    assert.throws(() => f.recorder.finish(), /Diagnostic total byte budget exceeded/);
  } finally { f.recorder.dispose(); }
});

test('invalid phase and clock do not generate believable ordered evidence', () => {
  for (const variant of ['phase', 'clock', 'backwards']) {
    const f = fixture();
    try {
      if (variant === 'phase') f.setPhase('unknown');
      else if (variant === 'clock') f.setTime(-1);
      else { f.setTime(20); f.recorder.provider(f.document, context); f.setTime(10); }
      const expected = variant === 'phase' ? /Diagnostic phase differs/ : /Diagnostic clock exceeds bounded worker lifetime/;
      assert.throws(() => f.recorder.provider(f.document, context), expected);
      assert.throws(() => f.recorder.finish(), expected);
    } finally { f.recorder.dispose(); }
  }
});

test('dispose is idempotent and retires the event subscription without settings writes', () => {
  const f = fixture();
  f.recorder.dispose(); f.recorder.dispose(); f.event(['editor.codeActionsOnSave']);
  assert.equal(f.disposed(), 1);
  assert.throws(() => f.recorder.provider(f.document, context), /Diagnostic record budget exceeded/);
});

test('cancelled provider callback is recorded without becoming a readiness or output predicate', () => {
  const f = fixture();
  try {
    f.recorder.bind(f.document);
    f.recorder.provider(f.document, { ...context, cancelled: true });
    f.recorder.saved(f.document);
    const proof = plain(f.recorder.finish());
    assert.deepEqual(proof.records.map(row => row.origin), ['source-provider', 'did-save']);
    assert.equal(proof.records[0].context.cancelled, true);
    assert.equal(proof.records[1].context, null);
  } finally { f.recorder.dispose(); }
});
