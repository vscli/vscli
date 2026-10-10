'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { saveParticipantReadiness, readinessText } = require('./save-code-actions.cjs');

function fixture(options = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-readiness-test-'));
  const auxiliary = path.join(root, 'readiness.ini'), target = path.join(root, 'main.txt');
  fs.writeFileSync(auxiliary, readinessText); fs.writeFileSync(target, 'original 猫🙂\r\n');
  const uri = file => ({ fsPath: file, toString: () => pathToFileURL(file).href });
  let time = 0, timer, timerCleared = false, provider, disposed = false, saves = 0, targetCommands = 0;
  const document = { uri: uri(auxiliary), version: 1, isDirty: false, languageId: options.language || 'ini',
    text: readinessText, getText() { return this.text; }, positionAt(offset) { return { offset }; },
    async save() {
      saves += 1;
      if (options.hold) { timer(); return new Promise(() => {}); }
      if (saves >= (options.readyAt || 1)) {
        if (options.foreign) {
          provider.provideDocumentFormattingEdits({ uri: uri(target), version: this.version,
            getText: () => 'original 猫🙂\r\n' }, {}, { isCancellationRequested: false });
        } else {
          const current = this.version;
          if (options.stale) this.version -= 1;
          provider.provideDocumentFormattingEdits(this, {}, {
            isCancellationRequested: !!options.cancelled });
          if (options.duplicate) provider.provideDocumentFormattingEdits(this, {}, {
            isCancellationRequested: false });
          this.version = current;
        }
      }
      fs.writeFileSync(auxiliary, this.text); this.isDirty = false;
      if (options.atDeadline) time = 5000;
      return true;
    } };
  class WorkspaceEdit { constructor() { this.edits = []; } insert(resource, position, text) {
    this.edits.push({ resource, position, text });
  } }
  const vscode = {
    Uri: { file: uri }, RelativePattern: class { constructor(base, pattern) {
      this.base = base; this.pattern = pattern;
    } }, WorkspaceEdit,
    workspace: {
      async openTextDocument(resource) { assert.equal(resource.fsPath, auxiliary); return document; },
      getConfiguration(section, doc) { assert.equal(doc, document); return { get(key) {
        return ({ editor: { formatOnSave: options.disabled ? false : true, formatOnSaveMode: 'file' },
          files: { autoSave: 'off' } })[section][key];
      } }; },
      async applyEdit(edit) {
        assert.equal(edit.edits.length, 1);
        const change = edit.edits[0]; assert.equal(change.resource, document.uri);
        document.text = document.text.slice(0, change.position.offset) + change.text
          + document.text.slice(change.position.offset);
        document.version += 1; document.isDirty = true; return true;
      }
    },
    languages: { registerDocumentFormattingEditProvider(selector, value) {
      assert.equal(selector.scheme, 'file'); assert.equal(selector.language, 'ini');
      assert.equal(selector.pattern.base, root); assert.equal(selector.pattern.pattern, 'readiness.ini');
      provider = value;
      // A direct API-style warm callback occurs outside a Save; it must not
      // prove save-contribution readiness even for the exact auxiliary model.
      if (options.directWarm) provider.provideDocumentFormattingEdits(document, {}, {
        isCancellationRequested: false });
      return { dispose() { disposed = true; } };
    } },
    commands: { async executeCommand() { targetCommands += 1; assert.fail('Setup executed a target command'); } }
  };
  const clock = { now: () => time, pause: async ms => { time += ms; },
    setTimeout(callback, ms) { assert.equal(ms, 5000); timer = callback; return 1; },
    clearTimeout(handle) { assert.equal(handle, 1); timerCleared = true; } };
  return { root, document, vscode, clock, inspect: () => ({ saves, disposed, timerCleared, targetCommands }),
    cleanup() {
      assert.equal(fs.readFileSync(target, 'utf8'), 'original 猫🙂\r\n');
      assert.equal(targetCommands, 0); assert.equal(timerCleared, true);
      fs.rmSync(root, { recursive: true, force: true });
    } };
}
async function using(options, callback) {
  const value = fixture(options);
  try { await callback(value); } finally { value.cleanup(); }
}

test('exact public auxiliary Save callback qualifies with literal null-formatter bytes', async () => {
  await using({}, async f => {
    const proof = await saveParticipantReadiness(f.vscode, f.root, f.clock);
    assert.equal(proof.status, 'observed'); assert.equal(proof.attempts.length, 1);
    assert.equal(proof.callbacks.length, 1); assert.equal(proof.callbacks[0].matched, true);
    assert.equal(proof.callbacks[0].duringSave, true);
    assert.equal(proof.callbacks[0].uri, proof.initial.uri);
    assert.equal(proof.final.text, readinessText + 'x'); assert.equal(proof.final.disk, proof.final.text);
    assert.equal(proof.final.dirty, false); assert.equal(proof.final.version, 2);
    assert.equal(f.inspect().disposed, true);
  });
});
test('direct warm callback cannot hide late actual save-participant installation', async () => {
  await using({ readyAt: 4, directWarm: true }, async f => {
    const proof = await saveParticipantReadiness(f.vscode, f.root, f.clock);
    assert.equal(proof.attempts.length, 4);
    assert.deepEqual(proof.attempts.map(value => value.matched), [false, false, false, true]);
    assert.equal(proof.callbacks[0].duringSave, false); assert.equal(proof.callbacks[0].matched, false);
    assert.equal(proof.callbacks[1].attempt, 4); assert.equal(proof.callbacks[1].version, 5);
    assert.equal(f.inspect().saves, 4); assert.equal(f.inspect().disposed, true);
  });
});
for (const condition of ['foreign', 'stale', 'cancelled']) {
  test(`${condition} callback cannot authorize readiness or execute target commands`, async () => {
    await using({ [condition]: true }, async f => {
      await assert.rejects(saveParticipantReadiness(f.vscode, f.root, f.clock),
        /Save-participant readiness attempt budget exceeded/);
      assert.equal(f.inspect().saves, 16); assert.equal(f.inspect().disposed, true);
    });
  });
}
test('duplicate matching callbacks reject instead of silently authorizing', async () => {
  await using({ duplicate: true }, async f => {
    await assert.rejects(saveParticipantReadiness(f.vscode, f.root, f.clock),
      /Duplicate readiness formatter invocation/);
    assert.equal(f.inspect().saves, 1); assert.equal(f.inspect().disposed, true);
  });
});
test('whole readiness deadline retires an actually held Save without a retry', async () => {
  await using({ hold: true }, async f => {
    await assert.rejects(saveParticipantReadiness(f.vscode, f.root, f.clock),
      /Save-participant readiness deadline exceeded/);
    assert.equal(f.inspect().saves, 1); assert.equal(f.inspect().disposed, true);
  });
});
test('a positive callback at the deadline is too late', async () => {
  await using({ atDeadline: true }, async f => {
    await assert.rejects(saveParticipantReadiness(f.vscode, f.root, f.clock),
      /Save-participant readiness deadline exceeded/);
    assert.equal(f.inspect().saves, 1); assert.equal(f.inspect().disposed, true);
  });
});
for (const [condition, error] of [['language', /Readiness language differs/],
  ['disabled', /Readiness format-on-save is disabled/]]) {
  test(`${condition} precondition fails before auxiliary mutation`, async () => {
    await using({ [condition]: condition === 'language' ? 'plaintext' : true }, async f => {
      await assert.rejects(saveParticipantReadiness(f.vscode, f.root, f.clock), error);
      assert.equal(f.inspect().saves, 0); assert.equal(f.document.version, 1);
      assert.equal(f.document.text, readinessText);
    });
  });
}
