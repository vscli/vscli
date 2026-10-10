'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { createHash } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const cases = require('./save-code-actions-cases.json');
const { configurationReadiness, canonicalContract, canonical, reader, matches, setupBudget, archiveFailure, sourceIdentity, SOURCE_INPUTS } = require('./save-configuration-ready.cjs');
const { stableSaveSetup, readinessText, text: originalTargetText, saveCodeActionsTrace } = require('./save-code-actions.cjs');
const plain = value => JSON.parse(JSON.stringify(value));
function fixture(options = {}) {
  const declaration = cases.find(value => value.name === (options.name || 'ancestor-false-child'));
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-stable-save-test-'));
  const targetFile = path.join(root, 'main.txt'), auxiliaryFile = path.join(root, 'readiness.ini');
  const targetText = options.targetText || 'original 猫🙂\r\n', uri = file => ({ scheme: 'file', fsPath: file, toString: () => pathToFileURL(file).href });
  const targetUri = uri(targetFile);
  fs.writeFileSync(targetFile, targetText); fs.writeFileSync(auxiliaryFile, readinessText);
  let time = 0, nextTimer = 0, listeners = new Set(), provider, auxiliarySaves = 0, targetCommands = 0;
  let configObjects = 0, disposed = 0, migrated = !!options.initialReady, callbackDisposals = 0;
  const timers = new Map(), contract = plain(canonicalContract(declaration));
  let observedProof;
  function values() {
    const value = plain(contract);
    if (!migrated && declaration.name === 'ancestor-false-child') {
      value.effective.codeActionsOnSave['source.fixAll.child'] = false;
      value.inspect.codeActionsOnSave.globalValue['source.fixAll.child'] = false;
    }
    if (options.maskedLayer) {
      value.effective = plain(contract.effective);
      value.inspect.codeActionsOnSave.globalValue['source.fixAll.child'] = false;
    }
    if (options.wrongLanguageIds) value.inspect.formatOnSave.languageIds = ['cpp'];
    if (options.extraLayer) value.inspect.codeActionsOnSave.workspaceValue = {};
    if (options.large) value.effective.codeActionsOnSave.extra = 'x'.repeat(32768);
    if (options.circular) value.effective.codeActionsOnSave.extra = value.effective.codeActionsOnSave;
    return value;
  }
  function event(keys) {
    for (const listener of [...listeners]) listener({ affectsConfiguration(key, scope) {
      assert.equal(scope.uri.toString(), targetUri.toString()); assert.equal(scope.languageId, 'plaintext');
      return keys.includes(key);
    } });
  }
  const document = { uri: uri(auxiliaryFile), version: 1, languageId: 'ini', isDirty: false,
    text: readinessText, getText() { return this.text; }, positionAt(offset) { return { offset }; },
    async save() {
      auxiliarySaves += 1;
      if (auxiliarySaves >= (options.participantAt || 1)) {
        const doc = options.foreignAuxiliary ? { ...this, uri: targetUri } : this;
        provider.provideDocumentFormattingEdits(doc, {}, { isCancellationRequested: false });
      }
      fs.writeFileSync(auxiliaryFile, this.text); this.isDirty = false; return true;
    } };
  class WorkspaceEdit { constructor() { this.edits = []; }
    insert(resource, position, text) { this.edits.push({ resource, position, text }); } }
  const vscode = { Uri: { file: uri }, WorkspaceEdit,
    RelativePattern: class { constructor(base, pattern) { this.base = base; this.pattern = pattern; } },
    workspace: {
      async openTextDocument(resource) { assert.equal(resource.fsPath, auxiliaryFile); return document; },
      async applyEdit(edit) {
        assert.equal(edit.edits.length, 1); const change = edit.edits[0];
        assert.equal(change.resource, document.uri);
        document.text = document.text.slice(0, change.position.offset) + change.text + document.text.slice(change.position.offset);
        document.version += 1; document.isDirty = true; return true;
      },
      getConfiguration(section, scope) {
        if (scope === document) return { get: key => ({ editor: { formatOnSave: true, formatOnSaveMode: 'file' },
          files: { autoSave: declaration.autosave } })[section][key] };
        assert.equal(scope.uri.toString(), targetUri.toString()); assert.equal(scope.languageId, 'plaintext');
        configObjects += 1;
        // Old API objects genuinely keep old values after the source changes.
        const captured = values();
        return { get: key => captured.effective[key], inspect: key => captured.inspect[key] };
      },
      onDidChangeConfiguration(listener) { listeners.add(listener); return { dispose() { listeners.delete(listener); disposed += 1; } }; }
    },
    languages: { registerDocumentFormattingEditProvider(selector, value) {
      assert.equal(selector.pattern.base, root); assert.equal(selector.pattern.pattern, 'readiness.ini');
      provider = value;
      if (options.directWarm) value.provideDocumentFormattingEdits(document, {}, { isCancellationRequested: false });
      return { dispose() { callbackDisposals += 1; } };
    } },
    commands: { async executeCommand(command) {
      targetCommands += 1; assert.equal(command, 'target-once');
    } } };
  const clock = { now: () => time,
    setTimeout(callback, ms) { const id = ++nextTimer; timers.set(id, { at: time + ms, callback }); return id; },
    clearTimeout(id) { timers.delete(id); },
    report(proof) { observedProof = proof; },
    async pause(ms) {
      const previous = time; time += ms;
      if (!migrated && options.migrationAt !== null && time >= (options.migrationAt ?? 400)) {
        migrated = true; event(['editor.codeActionsOnSave']);
      }
      for (const at of options.noiseAt || []) if (previous < at && time >= at) event(['editor.codeActionsOnSave']);
      for (const [id, timer] of [...timers].sort((a, b) => a[1].at - b[1].at)) {
        if (timer.at <= time && timers.has(id)) { timers.delete(id); timer.callback(); }
      }
    } };
  return { vscode, targetUri, declaration, root, clock, event,
    inspect: () => ({ time, configObjects, disposed, callbackDisposals, auxiliarySaves, targetCommands,
      timers: timers.size, observedProof }),
    cleanup() { assert.equal(fs.readFileSync(targetFile, 'utf8'), targetText);
      assert.equal(listeners.size, 0); assert.equal(timers.size, 0); fs.rmSync(root, { recursive: true, force: true }); } };
}
async function using(options, callback) {
  const value = fixture(options); try { await callback(value); } finally { value.cleanup(); }
}
async function targetsAfterActualBarriers(f) {
  const budget = setupBudget(f.clock), progress = {};
  try {
    const setup = await stableSaveSetup(f.vscode, f.root, f.declaration, budget, progress, f.clock);
    const authorization = budget.authorizeTargets();
    await f.vscode.commands.executeCommand('target-once');
    return { setup, authorization, progress };
  } finally { budget.retire(); }
}

test('original fourteen case bytes and declaration order are unchanged', () => {
  const bytes = fs.readFileSync(path.join(__dirname, 'save-code-actions-cases.json'));
  assert.equal(createHash('sha256').update(bytes).digest('hex'),
    'a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c');
  assert.equal(cases.length, 14);
  for (const declaration of cases) assert.ok(canonicalContract(declaration));
});
test('entry migration is isolated, ordered, and leaves arrays and original inputs intact', () => {
  const input = { 'source.fixAll': true, 'source.fixAll.child': false, 'source.organizeImports': 'always' };
  const before = plain(input);
  assert.deepEqual(plain(canonical(input)), { 'source.fixAll': 'explicit', 'source.fixAll.child': 'never', 'source.organizeImports': 'always' });
  assert.deepEqual(input, before);
  const array = ['source.organizeImports', 'source.fixAll'];
  const result = canonical(array); assert.deepEqual(result, array); assert.notEqual(result, array);
});
test('fresh API snapshots observe migration without retrying target commands', async () => {
  await using({}, async f => {
    const stale = f.vscode.workspace.getConfiguration('editor', { uri: f.targetUri, languageId: 'plaintext' });
    const proof = await configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock);
    assert.equal(stale.get('codeActionsOnSave')['source.fixAll.child'], false);
    assert.equal(proof.status, 'ready'); assert.equal(proof.elapsedMs, 700);
    assert.deepEqual(proof.samples.map(row => row.matched), [false, false, false, false, true, true, true, true]);
    assert.equal(proof.samples[0].snapshot.inspect.codeActionsOnSave.globalValue['source.fixAll.child'], false);
    assert.equal(proof.samples.at(-1).snapshot.inspect.codeActionsOnSave.globalValue['source.fixAll.child'], 'never');
    assert.equal(proof.finalSampleSequence, 8); assert.equal(proof.events.length, 1);
    assert.ok(f.inspect().configObjects >= 18); assert.equal(f.inspect().targetCommands, 0);
  });
});
test('canonical at startup still requires the independent quiet interval', async () => {
  await using({ initialReady: true, noiseAt: [200] }, async f => {
    const proof = await configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock);
    assert.equal(proof.quietSinceMs, 200); assert.equal(proof.elapsedMs, 500);
    assert.equal(proof.events.length, 1); assert.equal(f.inspect().targetCommands, 0);
  });
});
for (const condition of ['maskedLayer', 'wrongLanguageIds', 'extraLayer']) {
  test(`${condition} cannot qualify from a preferred merged effective value`, async () => {
    await using({ initialReady: true, [condition]: true }, async f => {
      await assert.rejects(configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock), error => {
        assert.match(error.message, /Configuration readiness deadline exceeded/);
        assert.equal(error.configurationReadiness.status, 'failed');
        assert.ok(error.configurationReadiness.samples.length > 0);
        assert.equal(error.configurationReadiness.samples.at(-1).matched, false); return true;
      });
      assert.equal(f.inspect().targetCommands, 0);
    });
  });
}
test('a warm provider callback alone cannot authorize either barrier or a target', async () => {
  await using({ participantAt: 99, directWarm: true, initialReady: true }, async f => {
    await assert.rejects(targetsAfterActualBarriers(f), error => {
      assert.match(error.message, /Save-participant readiness attempt budget exceeded/);
      assert.equal(error.saveParticipantReadiness.status, 'failed');
      assert.equal(error.saveParticipantReadiness.callbacks[0].duringSave, false);
      assert.equal(error.saveParticipantReadiness.callbacks[0].matched, false); return true;
    });
    assert.equal(f.inspect().auxiliarySaves, 16); assert.equal(f.inspect().targetCommands, 0);
  });
});
test('late genuine auxiliary Save and canonical configuration authorize exactly one target', async () => {
  await using({ participantAt: 4, directWarm: true }, async f => {
    const { setup, authorization } = await targetsAfterActualBarriers(f);
    assert.equal(setup.saveParticipantReadiness.attempts.length, 4);
    assert.equal(setup.saveParticipantReadiness.callbacks[0].matched, false);
    assert.equal(setup.saveParticipantReadiness.callbacks.at(-1).matched, true);
    assert.equal(setup.configurationProof.status, 'ready');
    assert.equal(authorization.deadlineMs, 15000); assert.equal(f.inspect().targetCommands, 1);
  });
});
for (const name of ['after-delay-always-skipped', 'after-delay-array-skipped']) {
  test(`${name} records an explicit auxiliary Save without changing the target reason`, async () => {
    await using({ name, initialReady: true }, async f => {
      const { setup } = await targetsAfterActualBarriers(f);
      assert.equal(setup.saveParticipantReadiness.saveReason, 'explicit-auxiliary');
      assert.equal(setup.saveParticipantReadiness.targetAutoSave, 'afterDelay');
      assert.equal(setup.saveParticipantReadiness.effective.autoSave, 'afterDelay');
      assert.equal(setup.saveParticipantReadiness.attempts.length, 1);
      assert.equal(setup.saveParticipantReadiness.callbacks[0].duringSave, true);
      assert.equal(setup.configurationProof.expected.effective.autoSave, 'afterDelay');
      assert.equal(f.inspect().targetCommands, 1);
    });
  });
}
test('configuration timeout preserves every sampled value and starts zero targets', async () => {
  await using({ migrationAt: null }, async f => {
    await assert.rejects(targetsAfterActualBarriers(f), error => {
      assert.match(error.message, /Configuration readiness deadline exceeded/);
      const proof = error.configurationReadiness;
      assert.equal(proof.status, 'failed'); assert.equal(proof.samples.length, 100);
      assert.ok(proof.samples.every(row => row.snapshot.effective.codeActionsOnSave['source.fixAll.child'] === false));
      assert.ok(proof.samples.every(row => !row.matched)); return true;
    });
    assert.equal(f.inspect().targetCommands, 0); assert.equal(f.inspect().callbackDisposals, 1);
  });
});
test('unrelated configuration events are not target-scoped witnesses', async () => {
  await using({ initialReady: true }, async f => {
    const task = configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock);
    f.event(['window.zoomLevel']);
    const proof = await task;
    assert.equal(proof.events.length, 0); assert.equal(proof.elapsedMs, 300);
    assert.equal(f.inspect().targetCommands, 0);
  });
});
test('combined deadline fences a held setup operation before factory reentry or target authorization', async () => {
  await using({ initialReady: true }, async f => {
    const budget = setupBudget(f.clock);
    const held = budget.bound(() => new Promise(() => {}));
    await f.clock.pause(15000);
    await assert.rejects(held, /Combined save setup deadline exceeded/);
    assert.equal(budget.signal.aborted, true);
    let called = 0;
    await assert.rejects(budget.bound(() => { called += 1; }), /Combined save setup deadline exceeded/);
    assert.equal(called, 0);
    assert.throws(() => budget.authorizeTargets(), /Combined save setup deadline exceeded/);
    assert.equal(f.inspect().targetCommands, 0); budget.retire();
  });
});
test('a canceled setup retires the configuration listener and preserves the bounded prefix', async () => {
  await using({ migrationAt: null }, async f => {
    const controller = new AbortController();
    const task = configurationReadiness(f.vscode, f.targetUri, f.declaration, { ...f.clock, signal: controller.signal });
    controller.abort();
    await assert.rejects(task, error => {
      assert.match(error.message, /Configuration readiness canceled by setup deadline/);
      assert.equal(error.configurationReadiness.status, 'failed');
      assert.ok(error.configurationReadiness.samples.length > 0); return true;
    });
    assert.equal(f.inspect().disposed, 1); assert.equal(f.inspect().targetCommands, 0);
  });
});
test('failure evidence is written unchanged before rethrow and unsafe names write nothing', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-stable-failure-test-'));
  try {
    const failure = { protocol: 'stable-save-setup-failure-v1', name: 'ancestor-false-child', phase: 'setup',
      targetStarted: false, progress: { configurationReadiness: { status: 'failed', samples: [
        { sequence: 1, snapshot: { observed: false } }], error: 'Configuration readiness deadline exceeded' } },
      scope: 'Synthetic controlled-clock failure writer test; not an actual upstream capture' };
    archiveFailure(root, failure.name, failure, { synthetic: true });
    const bytes = fs.readFileSync(path.join(root, `${failure.name}-failure.json`));
    assert.deepEqual(JSON.parse(bytes), failure);
    const proof = JSON.parse(fs.readFileSync(path.join(root, `${failure.name}-failure-provenance.json`)));
    assert.equal(proof.failureSha256, createHash('sha256').update(bytes).digest('hex'));
    const count = fs.readdirSync(root).length;
    assert.throws(() => archiveFailure(root, '../outside', failure, {}), /Unknown failure fixture/);
    assert.equal(fs.readdirSync(root).length, count);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
test('reader captures known resource/language and retained snapshots do not change retroactively', async () => {
  await using({}, async f => {
    const read = reader(f.vscode, f.targetUri), before = read();
    await f.clock.pause(400);
    const after = read();
    assert.equal(before.languageId, 'plaintext'); assert.equal(before.uri, f.targetUri.toString());
    assert.equal(before.effective.codeActionsOnSave['source.fixAll.child'], false);
    assert.equal(after.effective.codeActionsOnSave['source.fixAll.child'], 'never');
    assert.equal(matches(before, canonicalContract(f.declaration)), false);
    assert.equal(matches(after, canonicalContract(f.declaration)), true);
  });
});

for (const condition of ['large', 'circular']) {
  test(`${condition} public values fail with a retained failure prefix and no targets`, async () => {
    await using({ initialReady: true, [condition]: true }, async f => {
      await assert.rejects(configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock), error => {
        assert.match(error.message, /Configuration readiness value exceeds bounds|Invalid readiness value/);
        assert.equal(error.configurationReadiness.status, 'failed');
        assert.equal(error.configurationReadiness.samples.length, 0); return true;
      });
      assert.equal(f.inspect().targetCommands, 0);
    });
  });
}
test('swallowed event overflow remains an explicit failure instead of truncated readiness', async () => {
  await using({ migrationAt: null }, async f => {
    const task = configurationReadiness(f.vscode, f.targetUri, f.declaration, f.clock);
    for (let index = 0; index < 129; index += 1) f.event(['editor.codeActionsOnSave']);
    await assert.rejects(task, error => {
      assert.match(error.message, /Configuration readiness record budget exceeded/);
      assert.equal(error.configurationReadiness.status, 'failed');
      assert.equal(error.configurationReadiness.events.length, 128); return true;
    });
    assert.equal(f.inspect().targetCommands, 0);
  });
});

// These validate actual current source bytes first. Mutations are labelled test
// manifests, never generated captured evidence or an alternative golden trace.
test('source preflight admits exactly the ten current executable inputs', () => {
  const actual = sourceIdentity(__dirname);
  assert.deepEqual(Object.keys(actual).sort(), [...SOURCE_INPUTS].sort());
  assert.equal(actual['save-code-actions-cases.json'], 'a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c');
});
function copiedSources(callback) {
  assert.ok(sourceIdentity(__dirname));
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-stable-source-test-'));
  try {
    for (const file of [...SOURCE_INPUTS, 'source-sha256.json']) fs.copyFileSync(path.join(__dirname, file), path.join(root, file));
    assert.ok(sourceIdentity(root));
    return callback(root);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
}
for (const mutation of ['traversal', 'missing', 'malformed']) {
  test(`${mutation} real source manifest fails before named input reads`, () => copiedSources(root => {
    const manifest = JSON.parse(fs.readFileSync(path.join(root, 'source-sha256.json')));
    if (mutation === 'traversal') manifest['../outside'] = '0'.repeat(64);
    else if (mutation === 'missing') delete manifest['extension.cjs'];
    else manifest['extension.cjs'] = 'BAD';
    fs.writeFileSync(path.join(root, 'source-sha256.json'), JSON.stringify(manifest));
    const opened = [];
    assert.throws(() => sourceIdentity(root, { ...fs, openSync(file, flags) {
      opened.push(path.basename(file)); return fs.openSync(file, flags);
    } }), /Source inventory differs|Invalid source digest/);
    assert.deepEqual(opened, ['source-sha256.json']);
  }));
}
test('changed real source bytes fail instead of accepting a matching name only', () => copiedSources(root => {
  fs.appendFileSync(path.join(root, 'save-code-actions.cjs'), 'changed');
  const opened = [];
  assert.throws(() => sourceIdentity(root, { ...fs, openSync(file, flags) {
    opened.push(path.basename(file)); return fs.openSync(file, flags);
  } }), /Source bytes differ: save-code-actions.cjs/);
  assert.deepEqual(opened, ['source-sha256.json', 'save-code-actions.cjs']);
}));
for (const name of ['source-sha256.json', 'save-code-actions.cjs']) {
  test(`oversized real ${name} is rejected before its first read`, () => copiedSources(root => {
    const maximum = name === 'source-sha256.json' ? 16384 : 1048576;
    fs.writeFileSync(path.join(root, name), Buffer.alloc(maximum + 1, 32));
    const descriptors = new Map(), reads = [];
    assert.throws(() => sourceIdentity(root, { ...fs,
      openSync(file, flags) { const descriptor = fs.openSync(file, flags); descriptors.set(descriptor, path.basename(file)); return descriptor; },
      closeSync(descriptor) { descriptors.delete(descriptor); return fs.closeSync(descriptor); },
      readSync(descriptor, ...args) { reads.push(descriptors.get(descriptor)); return fs.readSync(descriptor, ...args); }
    }), /Source file exceeds bounds or is not regular/);
    assert.equal(reads.includes(name), false);
  }));
}
test('short positive fd reads still validate the complete real source bytes', () => copiedSources(root => {
  let calls = 0;
  const actual = sourceIdentity(root, { ...fs, readSync(descriptor, bytes, offset, length, position) {
    calls += 1; return fs.readSync(descriptor, bytes, offset, Math.min(length, 17), position);
  } });
  assert.deepEqual(actual, sourceIdentity(root)); assert.ok(calls > 100);
}));
test('premature EOF cannot validate a regular source whose declared bytes remain', () => copiedSources(root => {
  const descriptors = new Map();
  assert.throws(() => sourceIdentity(root, { ...fs,
    openSync(file, flags) { const descriptor = fs.openSync(file, flags); descriptors.set(descriptor, path.basename(file)); return descriptor; },
    closeSync(descriptor) { descriptors.delete(descriptor); return fs.closeSync(descriptor); },
    readSync(descriptor, ...args) { return descriptors.get(descriptor) === 'save-code-actions.cjs' ? 0 : fs.readSync(descriptor, ...args); }
  }), /Source file changed or ended before its declared size/);
}));
test('a real file growing after fstat is stopped by the maximum-plus-one read', () => copiedSources(root => {
  const descriptors = new Map(); let grew = false;
  assert.throws(() => sourceIdentity(root, { ...fs,
    openSync(file, flags) { const descriptor = fs.openSync(file, flags); descriptors.set(descriptor, file); return descriptor; },
    closeSync(descriptor) { descriptors.delete(descriptor); return fs.closeSync(descriptor); },
    fstatSync(descriptor) {
      const stat = fs.fstatSync(descriptor), file = descriptors.get(descriptor);
      if (!grew && path.basename(file) === 'save-code-actions.cjs') {
        grew = true; fs.writeFileSync(file, Buffer.alloc(1048577, 32));
      }
      return stat;
    }
  }), /Source file exceeds bounds while reading/);
  assert.equal(grew, true);
}));
for (const mode of ['suite', 'worker']) {
  test(`${mode} actual bootstrap rejects hostile identity before observer, cases or external editor modules load`, () => copiedSources(root => {
    const manifest = JSON.parse(fs.readFileSync(path.join(root, 'source-sha256.json')));
    manifest['../outside'] = '0'.repeat(64);
    fs.writeFileSync(path.join(root, 'source-sha256.json'), JSON.stringify(manifest));
    const script = `
      const assert = require('node:assert/strict'), path = require('node:path'), Module = require('node:module');
      const loaded = [], original = Module._load, parse = JSON.parse; let caseParses = 0;
      Module._load = function(name, ...args) {
        if (name.endsWith('save-code-actions.cjs') || name.endsWith('save-code-actions-cases.json')
            || name === 'vscode' || name === '@vscode/test-electron') loaded.push(name);
        return original.call(this, name, ...args);
      };
      JSON.parse = (...args) => { const value = parse(...args); if (Array.isArray(value) && value.length === 14) caseParses += 1; return value; };
      const helper = require('./save-configuration-ready.cjs');
      assert.deepEqual(loaded, []); assert.equal(caseParses, 0);
      const failed = message => { assert.match(message, /Source inventory differs/);
        assert.deepEqual(loaded, []); assert.equal(caseParses, 0); process.stdout.write('preflight-before-imports'); };
      if (${JSON.stringify(mode)} === 'suite') require('./save-code-actions-suite.cjs').run().then(
        () => { throw new Error('Unexpected suite success'); }, error => failed(error.message));
      else { process.send = message => { assert.equal(message.ok, false); failed(message.error); };
        process.argv = [process.execPath, path.join(process.cwd(), 'save-code-actions-worker.cjs'), 'unused-output', 'unused-session'];
        require('./save-code-actions-worker.cjs'); }
    `;
    const child = spawnSync(process.execPath, ['-e', script], { cwd: root, encoding: 'utf8', timeout: 5000 });
    assert.equal(child.status, 0, child.stderr); assert.equal(child.signal, null);
    assert.equal(child.stdout, 'preflight-before-imports');
    assert.equal(fs.existsSync(path.join(root, 'unused-output')), false);
    assert.equal(fs.existsSync(path.join(root, 'unused-session')), false);
  }));
}
test('declared case and resource scope cannot be changed into a readiness target', async () => {
  for (const declaration of cases) assert.ok(canonicalContract(declaration));
  const changed = plain(cases[0]); changed.autosave = 'afterDelay';
  assert.throws(() => canonicalContract(changed), /Unknown or changed declared fixture/);
  await using({ initialReady: true }, async f => {
    assert.ok(matches(reader(f.vscode, f.targetUri)(), canonicalContract(f.declaration)));
    const resource = { ...f.targetUri, scheme: 'untitled' };
    assert.throws(() => reader(f.vscode, resource), /Configuration readiness resource differs/);
    assert.throws(() => reader(f.vscode, { ...f.targetUri, fsPath: path.join(f.root, 'foreign.txt') }),
      /Configuration readiness resource differs/);
    assert.equal(f.inspect().targetCommands, 0);
  });
});

async function fullTargetFailure(mode) {
  const f = fixture({ name: 'ancestor-never-child', initialReady: true, targetText: originalTargetText });
  const targetFile = path.join(f.root, 'main.txt');
  const saves = new Set(), changes = new Set(), calls = [];
  let actionProvider, actionDisposed = 0, snapshotFails = false;
  class Position { constructor(line, character) { this.line = line; this.character = character; } }
  class Range { constructor(start, end) { this.start = start; this.end = end; } }
  class Selection { constructor(anchor, active) { this.anchor = anchor; this.active = active; } }
  const document = { uri: f.targetUri, languageId: 'plaintext', version: 1, text: originalTargetText,
    isDirty: false, eol: 2,
    getText() { if (snapshotFails) throw new Error('fixture current snapshot failed'); return this.text; },
    positionAt(offset) {
      const preceding = this.text.slice(0, offset), lines = preceding.split('\n');
      return new Position(lines.length - 1, lines.at(-1).length);
    },
    offsetAt(position) {
      const lines = this.text.split('\n'); let offset = 0;
      for (let index = 0; index < position.line; index += 1) offset += lines[index].length + 1;
      return offset + position.character;
    },
    lineAt(line) { return { range: new Range(new Position(line, 0),
      new Position(line, this.text.split('\n')[line].replace(/\r$/, '').length)) }; }
  };
  const editor = { document, selections: [new Selection(new Position(0, 0), new Position(0, 0))],
    get selection() { return this.selections[0]; }, set selection(value) { this.selections = [value]; } };
  const open = f.vscode.workspace.openTextDocument;
  f.vscode.workspace.openTextDocument = async uri => uri.fsPath === targetFile ? document : open(uri);
  f.vscode.workspace.onDidSaveTextDocument = listener => { saves.add(listener); return { dispose() { saves.delete(listener); } }; };
  f.vscode.workspace.onDidChangeTextDocument = listener => { changes.add(listener); return { dispose() { changes.delete(listener); } }; };
  f.vscode.window = { async showTextDocument(doc) { assert.equal(doc, document); return editor; } };
  Object.assign(f.vscode, { Position, Range, Selection, EndOfLine: { CRLF: 2 },
    CodeAction: class { constructor(title, kind) { this.title = title; this.kind = kind; } },
    CodeActionKind: class { constructor(value) { this.value = value; } } });
  f.vscode.WorkspaceEdit.prototype.replace = function(uri, range, text) { this.edits.push({ uri, range, text }); };
  f.vscode.languages.registerCodeActionsProvider = (selector, value) => {
    assert.equal(selector.pattern.pattern, 'main.txt'); actionProvider = value;
    return { dispose() { actionDisposed += 1; } };
  };
  const range = () => new Range(new Position(0, 0), document.positionAt(document.text.length));
  const source = () => actionProvider.provideCodeActions(document, range(),
    { only: { value: 'source.fixAll' }, triggerKind: 1 }, { isCancellationRequested: false });
  f.vscode.commands.getCommands = async () => ['type', 'workbench.action.files.save', 'undo', 'redo', 'vscode.executeCodeActionProvider'];
  f.vscode.commands.executeCommand = async (command, ...args) => {
    calls.push(command);
    if (command === 'vscode.executeCodeActionProvider') return source();
    if (command === 'type') {
      const before = document.text.length; document.text += args[0].text; document.version += 1; document.isDirty = true;
      editor.selection = new Selection(document.positionAt(document.text.length), document.positionAt(document.text.length));
      for (const listener of changes) listener({ document, contentChanges: [{ rangeOffset: before, rangeLength: 0, text: args[0].text }] });
      return;
    }
    if (command === 'workbench.action.files.save') {
      source();
      if (mode.startsWith('save')) {
        if (mode === 'save-disk') fs.unlinkSync(targetFile);
        throw new Error('fixture Save acknowledgement missing');
      }
      fs.writeFileSync(targetFile, document.text); document.isDirty = false;
      for (const listener of saves) listener(document);
      return;
    }
    assert.equal(command, 'undo'); snapshotFails = mode === 'undo-snapshot';
    throw new Error('fixture Undo failed');
  };
  try {
    let originalError;
    await assert.rejects(saveCodeActionsTrace(f.vscode, f.declaration.name, f.root), error => {
      originalError = error;
      assert.match(error.message, mode.startsWith('save') ? /fixture Save acknowledgement missing/ : /fixture Undo failed/);
      const raw = error.saveReferenceFailure;
      assert.equal(raw.phase, 'target'); assert.equal(raw.targetStarted, true);
      assert.equal(raw.targetCommandsIssued, mode.startsWith('save') ? 2 : 3);
      const prefix = raw.targetProgress;
      assert.equal(prefix.protocol, 'stable-save-target-prefix-v1');
      assert.equal(prefix.pending.action, mode.startsWith('save') ? 'workbench.action.files.save' : 'undo');
      assert.equal(prefix.pending.completed, false);
      assert.deepEqual(prefix.issued.map(value => [value.action, value.completed]), mode.startsWith('save')
        ? [['type', true], ['workbench.action.files.save', false]]
        : [['type', true], ['workbench.action.files.save', true], ['undo', false]]);
      assert.deepEqual(prefix.observations.map(value => value.action), mode.startsWith('save')
        ? ['initial', 'type'] : ['initial', 'type', 'workbench.action.files.save']);
      assert.deepEqual(prefix.steps.map(value => value.action), mode.startsWith('save') ? ['type'] : ['type', 'workbench.action.files.save']);
      assert.deepEqual(prefix.callbacks.map(value => value.phase), ['setup', 'target']);
      assert.equal(prefix.changes.length, 1); assert.equal(prefix.changes[0].changes[0].text, 'λ🙂');
      assert.equal(prefix.saved.length, mode.startsWith('save') ? 0 : 1);
      assert.equal(prefix.observations[1].text, originalTargetText + 'λ🙂');
      if (mode === 'undo-snapshot') {
        assert.equal(prefix.current.status, 'unavailable'); assert.match(prefix.current.error, /fixture current snapshot failed/);
      } else {
        assert.equal(prefix.current.status, 'observed'); assert.equal(prefix.current.text, originalTargetText + 'λ🙂');
        assert.equal(prefix.current.version, 2); assert.equal(prefix.current.dirty, mode.startsWith('save'));
        assert.equal(prefix.current.languageId, 'plaintext'); assert.equal(prefix.current.uri, f.targetUri.toString());
      }
      assert.equal(raw.originalTarget.disk, mode === 'save-disk' ? null : mode === 'save' ? originalTargetText : originalTargetText + 'λ🙂');
      if (mode === 'save-disk') { assert.match(raw.originalTarget.diskError, /ENOENT/);
        assert.match(prefix.current.diskError, /ENOENT/); }
      archiveFailure(f.root, f.declaration.name, raw, { synthetic: true,
        scope: 'Controlled public API target-failure test, not upstream captured evidence' });
      const retained = JSON.parse(fs.readFileSync(path.join(f.root, `${f.declaration.name}-failure.json`)));
      assert.deepEqual(retained.targetProgress, prefix); assert.equal(retained.error.message, error.message);
      return true;
    });
    assert.ok(originalError);
    assert.deepEqual(calls, mode.startsWith('save') ? ['vscode.executeCodeActionProvider', 'type', 'workbench.action.files.save']
      : ['vscode.executeCodeActionProvider', 'type', 'workbench.action.files.save', 'undo']);
    assert.equal(actionDisposed, 1); assert.equal(saves.size, 0); assert.equal(changes.size, 0);
  } finally {
    // Own synthetic fixture cleanup; no actual reference observation is rewritten.
    fs.writeFileSync(targetFile, originalTargetText); f.cleanup();
  }
}
for (const mode of ['save', 'save-disk', 'undo', 'undo-snapshot']) {
  test(`public ${mode} failure preserves issued gesture, complete target prefix and original error`,
    () => fullTargetFailure(mode));
}
