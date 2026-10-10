'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const corpus = require('./editor-sticky-tabs-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const scope = 'Independent sticky-prefix/preview membership, original pin/unpin/protected and forced clean Close/batches, per-profile close policy, public groups/views/identities/events and unchanged Unicode CRLF disks; no graphical mouse or dirty dialog';
const sources = ['editor-sticky-tabs-cases.json', 'editor-sticky-tabs-run.cjs',
  'editor-sticky-tabs-suite.cjs', 'editor-sticky-tabs-worker.cjs', 'supervisor.cjs',
  'package-lock.json', 'package.json', 'extension.cjs'];

// Startup output/virtual models are public document events, but are not fixture
// editors. Preserve them separately without weakening file/editor validation.
function supplementalRecorder({ documentId, phase, fixtureEventCount }) {
  const events = [];
  let bytes = 0;
  const capture = (kind, document, changes = []) => {
    if (document.uri.scheme === 'file') return false;
    assert.ok(events.length < 256, 'Supplemental event count budget exceeded');
    const uri = document.uri.toString(true);
    assert.ok(Buffer.byteLength(uri) <= 4096, 'Supplemental URI budget exceeded');
    assert.ok(typeof document.languageId === 'string' && Buffer.byteLength(document.languageId) <= 128);
    assert.ok(changes.length <= 128, 'Supplemental change count budget exceeded');
    let textBytes = 0;
    const copied = changes.map(change => {
      textBytes += Buffer.byteLength(change.text);
      assert.ok(textBytes <= 64 * 1024, 'Supplemental change text budget exceeded');
      return { text: change.text, rangeOffset: change.rangeOffset, rangeLength: change.rangeLength };
    });
    const event = { kind, uri, scheme: document.uri.scheme, phase: { ...phase() },
      fixtureEventCount: fixtureEventCount(), documentObject: documentId(document),
      languageId: document.languageId, dirty: document.isDirty, version: document.version,
      ...(kind === 'document-change' ? { changes: copied } : {}) };
    const size = Buffer.byteLength(JSON.stringify(event));
    assert.ok(bytes + size <= 512 * 1024, 'Supplemental event byte budget exceeded');
    events.push(event); bytes += size;
    return true;
  };
  return { events, capture };
}

async function observe() {
  const vscode = require('vscode');
  assert.equal(vscode.version, corpus.version);
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, corpus.commit);
  const name = process.env.VSCLI_REFERENCE_EDITOR_STICKY_TABS_CASE;
  const fixture = corpus.cases.find(item => item.name === name);
  assert.ok(fixture, 'Unknown sticky reference case');
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  const workspace = fs.realpathSync(process.env.VSCLI_REFERENCE_EDITOR_STICKY_TABS_WORKSPACE);
  const files = Object.fromEntries(corpus.files.map(file => [file.name, file.text]));
  const effective = {};
  const policy = { ...corpus.policy, 'workbench.editor.preventPinnedEditorClose': fixture.closePolicy };
  const required = [...new Set(['vscode.open', ...fixture.setup.filter(step => step.command).map(step => step.command),
    ...fixture.steps.filter(step => step.command).map(step => step.command)])];
  let eventFailure;
  const documentIds = new WeakMap();
  let nextDocument = 1;
  const documentId = document => {
    if (!documentIds.has(document)) {
      assert.ok(nextDocument <= 128, 'Public document identity budget exceeded');
      documentIds.set(document, nextDocument++);
    }
    return documentIds.get(document);
  };
  const resource = uri => {
    assert.equal(uri.scheme, 'file', 'Non-file editor escaped the clean sticky fixture');
    const relative = path.relative(workspace, fs.realpathSync(uri.fsPath)).split(path.sep).join('/');
    assert.ok(Object.hasOwn(files, relative), `Editor escaped sticky resource inventory: ${relative}`);
    return relative;
  };
  const editorState = editor => {
    const document = editor.document, text = document.getText();
    assert.ok(Buffer.byteLength(text) < 4096 && editor.selections.length <= 128);
    const position = p => ({ line: p.line, character: p.character,
      scalar: [...text.slice(0, document.offsetAt(p))].length });
    return { resource: resource(document.uri), viewColumn: editor.viewColumn ?? null,
      documentObject: documentId(document), text, dirty: document.isDirty, version: document.version,
      eol: document.eol === vscode.EndOfLine.CRLF ? 'CRLF' : 'LF',
      selections: editor.selections.map(selection => ({ anchor: position(selection.anchor), cursor: position(selection.active) })) };
  };
  const tabState = tab => {
    assert.ok(tab.input instanceof vscode.TabInputText, 'Non-text tab escaped sticky fixture');
    return { resource: resource(tab.input.uri), active: tab.isActive, dirty: tab.isDirty,
      pinned: tab.isPinned, preview: tab.isPreview };
  };
  const snapshot = () => {
    if (eventFailure) throw eventFailure;
    const all = vscode.window.tabGroups.all;
    assert.ok(all.length <= 4 && all.reduce((count, group) => count + group.tabs.length, 0) <= 8, 'Sticky group/tab budget exceeded');
    const groups = all.map(group => ({ viewColumn: group.viewColumn, active: group.isActive,
      tabs: group.tabs.map(tabState) }));
    assert.ok(vscode.window.visibleTextEditors.length <= 4 && vscode.workspace.textDocuments.length <= 128);
    const visible = vscode.window.visibleTextEditors.map(editorState).sort((a, b) => a.viewColumn - b.viewColumn);
    const active = vscode.window.activeTextEditor ? editorState(vscode.window.activeTextEditor) : null;
    const documents = corpus.files.map(file => {
      const document = vscode.workspace.textDocuments.find(doc => doc.uri.scheme === 'file'
        && fs.realpathSync(doc.uri.fsPath) === path.join(workspace, file.name));
      return { resource: file.name, loaded: Boolean(document), documentObject: document ? documentId(document) : null,
        text: document ? document.getText() : fs.readFileSync(path.join(workspace, file.name), 'utf8'),
        dirty: document?.isDirty ?? false, version: document?.version ?? null,
        disk: fs.readFileSync(path.join(workspace, file.name), 'utf8') };
    });
    for (const document of documents) assert.equal(document.disk, files[document.resource], 'Sticky observation changed fixture disk bytes');
    return { groups, active, visible, documents };
  };
  const events = [];
  let phase = { kind: 'readiness' };
  const supplemental = supplementalRecorder({ documentId, phase: () => phase,
    fixtureEventCount: () => events.length });
  const record = event => { assert.ok(events.length < 1024, 'Sticky public event budget exceeded'); events.push(event); };
  const listen = handler => event => {
    if (eventFailure) return;
    try { handler(event); } catch (error) { eventFailure = error; }
  };
  const listeners = [
    vscode.window.tabGroups.onDidChangeTabs(listen(event => record({ kind: 'tabs',
      opened: event.opened.map(tabState), closed: event.closed.map(tabState), changed: event.changed.map(tabState) }))),
    vscode.window.tabGroups.onDidChangeTabGroups(listen(event => record({ kind: 'groups',
      opened: event.opened.map(group => group.viewColumn), closed: event.closed.map(group => group.viewColumn), changed: event.changed.map(group => group.viewColumn) }))),
    vscode.window.onDidChangeActiveTextEditor(listen(editor => record({ kind: 'active', editor: editor ? editorState(editor) : null }))),
    vscode.window.onDidChangeVisibleTextEditors(listen(editors => record({ kind: 'visible', editors: editors.map(editorState) }))),
    vscode.window.onDidChangeTextEditorSelection(listen(event => record({ kind: 'selection', selectionKind: event.kind ?? null, editor: editorState(event.textEditor) }))),
    vscode.workspace.onDidChangeTextDocument(listen(event => {
      if (supplemental.capture('document-change', event.document, event.contentChanges)) return;
      record({ kind: 'document-change', resource: resource(event.document.uri), documentObject: documentId(event.document), dirty: event.document.isDirty,
        version: event.document.version, changes: event.contentChanges.map(change => ({ text: change.text, rangeOffset: change.rangeOffset, rangeLength: change.rangeLength })) });
    })),
    vscode.workspace.onDidCloseTextDocument(listen(document => {
      if (supplemental.capture('document-close', document)) return;
      record({ kind: 'document-close', resource: resource(document.uri), documentObject: documentId(document) });
    })),
  ];
  const settle = async () => {
    const start = Date.now(), deadline = start + 3000;
    let previous, unchangedAt = start, reads = 0;
    for (;;) {
      const observed = snapshot(), fingerprint = JSON.stringify(observed); reads++;
      if (fingerprint !== previous) { previous = fingerprint; unchangedAt = Date.now(); }
      if (Date.now() - unchangedAt >= 100) return { observed, reads, elapsedMs: Date.now() - start,
        scope: 'Awaited acknowledged operation then 100ms unchanged public state; no target retry or preferred-output predicate' };
      assert.ok(Date.now() < deadline, 'Independent sticky public settlement exceeded three seconds');
      await delay(20);
    }
  };
  const acknowledge = async (operation, label) => {
    let timeout;
    try {
      return await Promise.race([operation(), new Promise((_resolve, reject) => {
        timeout = setTimeout(() => reject(new Error(`Original operation acknowledgement exceeded five seconds: ${label}`)), 5000);
      })]);
    } finally { clearTimeout(timeout); }
  };
  const cleanClose = command => {
    if (!['workbench.action.closeActiveEditor', 'workbench.action.closeActivePinnedEditor',
      'workbench.action.closeEditorsInGroup', 'workbench.action.closeAllEditors'].includes(command)) return;
    const observed = snapshot();
    assert.ok(observed.groups.flatMap(group => group.tabs).every(tab => !tab.dirty), 'Refuse dirty-tab close dialog');
    assert.ok(observed.documents.every(document => !document.dirty), 'Refuse dirty-model close dialog');
  };
  const execute = async step => {
    if (step.open) {
      assert.ok(Object.hasOwn(files, step.open));
      return acknowledge(() => vscode.commands.executeCommand('vscode.open', vscode.Uri.file(path.join(workspace, step.open)),
        { preview: step.preview }), 'vscode.open');
    }
    cleanClose(step.command);
    return acknowledge(() => vscode.commands.executeCommand(step.command, ...(Object.hasOwn(step, 'args') ? [step.args] : [])), step.command);
  };
  const setup = { commandInventory: required, effectiveConfiguration: effective, closePolicy: fixture.closePolicy,
    scope: 'Fixed separately labelled public API opens/selections and original pin/focus commands establish clean membership/view/MRU inputs; not target output retries', operations: [] };
  const trace = { name, setup, observations: [], settlements: [], details: [], events,
    supplementalEvents: supplemental.events, scope };
  try {
    const commands = await vscode.commands.getCommands(true);
    for (const command of required) assert.ok(commands.includes(command), `Pinned original command absent: ${command}`);
    for (const [key, expected] of Object.entries(policy)) {
      const split = key.lastIndexOf('.');
      effective[key] = vscode.workspace.getConfiguration(key.slice(0, split)).get(key.slice(split + 1));
      assert.deepEqual(effective[key], expected, `Independent sticky policy differs: ${key}`);
    }
    for (const [index, step] of fixture.setup.entries()) {
      const eventStart = events.length;
      const supplementalStart = supplemental.events.length;
      phase = { kind: 'setup', index };
      if (step.api) {
        const editor = await acknowledge(async () => {
          const document = await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(workspace, step.resource)));
          return vscode.window.showTextDocument(document, { preview: step.preview, viewColumn: step.group });
        }, 'api.openTextDocument/showTextDocument');
        if (step.selection) {
          const text = editor.document.getText();
          const scalarPosition = scalar => {
            assert.ok(Number.isSafeInteger(scalar) && scalar >= 0 && scalar <= [...text].length);
            return editor.document.positionAt([...text].slice(0, scalar).join('').length);
          };
          editor.selections = [new vscode.Selection(scalarPosition(step.selection.anchor), scalarPosition(step.selection.cursor))];
        }
      } else { await execute(step); }
      const settlement = await settle();
      setup.operations.push({ step: index, gesture: step, settlement, events: events.slice(eventStart),
        supplementalEvents: supplemental.events.slice(supplementalStart) });
    }
    phase = { kind: 'initial' };
    const initial = await settle();
    trace.observations.push({ action: 'initial', ...initial.observed }); trace.settlements.push(initial);
    for (const [index, step] of fixture.steps.entries()) {
      const eventStart = events.length;
      const supplementalStart = supplemental.events.length;
      phase = { kind: 'target', index };
      await execute(step);
      const settlement = await settle();
      trace.observations.push({ action: step.open ? 'vscode.open' : step.command,
        ...(step.open ? { opened: step.open, requestedPreview: step.preview } : {}), ...settlement.observed });
      trace.settlements.push(settlement);
      trace.details.push({ step: index, gesture: step, events: events.slice(eventStart),
        supplementalEvents: supplemental.events.slice(supplementalStart) });
    }
    phase = { kind: 'verification' };
    for (const file of corpus.files) assert.equal(fs.readFileSync(path.join(workspace, file.name), 'utf8'), file.text, 'Sticky capture wrote fixture disk');
    const bytes = JSON.stringify(trace, null, 2) + '\n';
    fs.writeFileSync(path.join(output, `${name}-evidence.json`), bytes);
    const hashes = Object.fromEntries(sources.map(file => [file, hash(fs.readFileSync(path.join(__dirname, file)))]));
    const executableSha256 = process.env.VSCLI_REFERENCE_EXECUTABLE_SHA256;
    assert.match(executableSha256, /^[a-f0-9]{64}$/);
    fs.writeFileSync(path.join(output, `${name}-provenance.json`), JSON.stringify({
      version: vscode.version, commit: product.commit, platform: process.platform, architecture: process.arch,
      observedAt: new Date().toISOString(), name, productSha256: hash(productBytes), launchedExecutableSha256: executableSha256,
      evidenceSha256: hash(bytes), sources: hashes, profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
      closePolicy: fixture.closePolicy, scope,
    }, null, 2) + '\n');
  } catch (error) {
    error.referenceTrace = trace;
    throw error;
  } finally { for (const listener of listeners) listener.dispose(); }
}
exports.run = async () => {
  const vscode = require('vscode');
  try { await observe(); } catch (error) {
    const name = process.env.VSCLI_REFERENCE_EDITOR_STICKY_TABS_CASE;
    assert.ok(corpus.cases.some(fixture => fixture.name === name), 'Cannot record unknown reference case failure');
    fs.writeFileSync(path.join(process.env.VSCLI_REFERENCE_OUTPUT, `${name}-failure.json`), JSON.stringify({
      name, version: vscode.version, expectedCommit: corpus.commit, error: error.stack || String(error),
      partial: error.referenceTrace ?? null,
      scope: 'Failed setup/operation evidence, not a completed observation or native qualification',
    }, null, 2) + '\n');
    throw error;
  }
};

if (require.main === module) {
  const test = require('node:test');
  const virtual = uri => ({ uri: { scheme: 'output', toString: () => uri },
    languageId: 'Log', version: 2, isDirty: false });
  test('supplemental events retain URI, identity, phase and exact Unicode changes', () => {
    let phase = { kind: 'readiness' }, next = 1;
    const ids = new WeakMap(), identity = doc => {
      if (!ids.has(doc)) ids.set(doc, next++);
      return ids.get(doc);
    };
    const recorder = supplementalRecorder({ documentId: identity, phase: () => phase, fixtureEventCount: () => 3 });
    const document = virtual('output:extension-host-log');
    assert.equal(recorder.capture('document-change', document, [{ text: '猫🙂\r\n', rangeOffset: 0, rangeLength: 0 }]), true);
    phase = { kind: 'target', index: 1 };
    assert.equal(recorder.capture('document-close', document), true);
    assert.deepEqual(recorder.events, [
      { kind: 'document-change', uri: 'output:extension-host-log', scheme: 'output', phase: { kind: 'readiness' },
        fixtureEventCount: 3, documentObject: 1, languageId: 'Log', dirty: false, version: 2,
        changes: [{ text: '猫🙂\r\n', rangeOffset: 0, rangeLength: 0 }] },
      { kind: 'document-close', uri: 'output:extension-host-log', scheme: 'output', phase: { kind: 'target', index: 1 },
        fixtureEventCount: 3, documentObject: 1, languageId: 'Log', dirty: false, version: 2 },
    ]);
  });
  test('file events still require strict fixture classification', () => {
    const recorder = supplementalRecorder({ documentId: () => 1, phase: () => ({ kind: 'setup', index: 0 }), fixtureEventCount: () => 0 });
    const document = virtual('file:///outside.txt'); document.uri.scheme = 'file';
    const route = () => {
      if (!recorder.capture('document-change', document)) throw new Error('Unknown fixture file');
    };
    assert.throws(route, /Unknown fixture file/);
    assert.deepEqual(recorder.events, []);
  });
  test('supplemental bounds reject whole new events without truncating prior evidence', () => {
    const recorder = supplementalRecorder({ documentId: () => 1, phase: () => ({ kind: 'initial' }), fixtureEventCount: () => 0 });
    const document = virtual('output:bounded');
    assert.equal(recorder.capture('document-change', document, [{ text: 'original', rangeOffset: 0, rangeLength: 0 }]), true);
    const prior = JSON.stringify(recorder.events);
    assert.throws(() => recorder.capture('document-change', document, [{ text: 'x'.repeat(65537), rangeOffset: 0, rangeLength: 0 }]), /text budget/);
    assert.equal(JSON.stringify(recorder.events), prior);
    for (let i = 1; i < 256; i++) recorder.capture('document-close', document);
    assert.throws(() => recorder.capture('document-close', document), /count budget/);
    assert.equal(recorder.events.length, 256);
  });
}
