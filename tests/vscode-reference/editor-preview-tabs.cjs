'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./editor-preview-tabs-cases.json');
const files = {
  'a.txt': '猫🙂 alpha\r\nline-a\r\n', 'b.txt': 'β🙂 beta\r\nline-b\r\n',
  'c.txt': 'γ🙂 gamma\r\nline-c\r\n', 'd.txt': 'δ🙂 delta\r\nline-d\r\n',
};
const policy = {
  'workbench.editor.enablePreview': true,
  'workbench.editor.enablePreviewFromQuickOpen': false,
  'workbench.editor.enablePreviewFromCodeNavigation': false,
  'workbench.editor.openPositioning': 'right',
  'workbench.editor.focusRecentEditorAfterClose': true,
  'workbench.editor.closeEmptyGroups': true,
  'workbench.editor.revealIfOpen': false,
};
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

// Both arguments are real filesystem paths. Windows path.relative compares
// drive/root casing and separator variants without weakening fixture containment.
function fixtureFileResource(workspace, filename, paths = path) {
  const relative = paths.relative(workspace, filename).split(paths.sep).join('/');
  assert.ok(Object.hasOwn(files, relative), `Editor escaped fixture resource: ${relative}`);
  return relative;
}


// VS Code catches exceptions thrown by listeners. Retain the first one and
// explicitly fail the next snapshot/return so a capture cannot silently pass.
function eventGuard() {
  let failed = false, failure;
  return {
    listen: handler => (...args) => {
      if (failed) return;
      try { handler(...args); } catch (error) { failed = true; failure = error; }
    },
    check: () => { if (failed) throw failure; },
  };
}
function supplementalRecorder() {
  const events = [];
  let bytes = 0;
  return { events, record: event => {
    assert.ok(events.length < 256, 'Supplemental event count budget exceeded');
    const size = Buffer.byteLength(JSON.stringify(event));
    assert.ok(size <= 72 * 1024 && bytes + size <= 512 * 1024,
      'Supplemental event byte budget exceeded');
    events.push(event); bytes += size;
  } };
}

async function editorPreviewTabsTrace(vscode, name, workspace, onFailure = () => {}) {
  const fixture = cases.find(item => item.name === name);
  assert.ok(fixture, 'Unknown editor-preview-tabs case');
  assert.ok(fixture.steps.length <= 8);
  const commands = await vscode.commands.getCommands(true);
  const required = [...new Set(['vscode.open', ...fixture.steps.filter(step => step.command).map(step => step.command)])];
  for (const command of required) assert.ok(commands.includes(command), `Pinned command unavailable: ${command}`);
  const effective = {};
  for (const [key, expected] of Object.entries(policy)) {
    const split = key.lastIndexOf('.');
    effective[key] = vscode.workspace.getConfiguration(key.slice(0, split)).get(key.slice(split + 1));
    assert.deepEqual(effective[key], expected, `Preview-tab fixture configuration changed: ${key}`);
  }
  const canonicalWorkspace = fs.realpathSync(workspace);
  const untitledResources = new Map();
  const resource = uri => {
    if (uri.scheme === 'untitled') {
      const key = uri.toString();
      assert.ok(untitledResources.size < 8 || untitledResources.has(key));
      if (!untitledResources.has(key)) untitledResources.set(key, `untitled-${untitledResources.size + 1}`);
      return untitledResources.get(key);
    }
    assert.equal(uri.scheme, 'file', 'Unsupported editor escaped the fixture');
    return fixtureFileResource(canonicalWorkspace, fs.realpathSync(uri.fsPath));
  };
  const documentIds = new WeakMap();
  let nextDocument = 1;
  const guard = eventGuard(), supplemental = supplementalRecorder();
  let phase = { kind: 'readiness' };
  const publicDocument = document => {
    if (!documentIds.has(document)) {
      assert.ok(nextDocument <= 128, 'Public document identity budget exceeded');
      documentIds.set(document, nextDocument++);
    }
    return documentIds.get(document);
  };
  const editorState = editor => {
    const document = editor.document, text = document.getText();
    assert.ok(text.length < 4096);
    assert.ok(editor.selections.length <= 128);
    const position = p => ({ line: p.line, character: p.character,
      scalar: [...text.slice(0, document.offsetAt(p))].length });
    return { resource: resource(document.uri), viewColumn: editor.viewColumn ?? null,
      documentObject: publicDocument(document), text, dirty: document.isDirty, version: document.version,
      eol: document.eol === vscode.EndOfLine.CRLF ? 'CRLF' : 'LF',
      selections: editor.selections.map(selection => ({ anchor: position(selection.anchor), cursor: position(selection.active) })) };
  };
  const tabState = tab => {
    assert.ok(tab.input instanceof vscode.TabInputText, 'Unexpected non-text fixture tab');
    return { resource: resource(tab.input.uri), active: tab.isActive, dirty: tab.isDirty,
      pinned: tab.isPinned, preview: tab.isPreview };
  };
  const snapshot = () => {
    guard.check();
    const all = vscode.window.tabGroups.all;
    assert.ok(all.length <= 4);
    assert.ok(all.every(group => group.tabs.length <= 8));
    const groups = all.map(group => ({ viewColumn: group.viewColumn, active: group.isActive,
      tabs: group.tabs.map(tabState) }));
    assert.ok(vscode.window.visibleTextEditors.length <= 4);
    assert.ok(vscode.workspace.textDocuments.length <= 128);
    const visible = vscode.window.visibleTextEditors.map(editorState).sort((a, b) => a.viewColumn - b.viewColumn);
    const active = vscode.window.activeTextEditor ? editorState(vscode.window.activeTextEditor) : null;
    const documents = Object.keys(files).map(name => {
      const document = vscode.workspace.textDocuments.find(doc => doc.uri.scheme === 'file'
        && resource(doc.uri) === name);
      return { resource: name, loaded: Boolean(document), documentObject: document ? publicDocument(document) : null,
        text: document ? document.getText() : fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8'),
        dirty: document?.isDirty ?? false, version: document?.version ?? null,
        disk: fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8') };
    });
    for (const document of vscode.workspace.textDocuments.filter(doc => doc.uri.scheme === 'untitled')) {
      documents.push({ resource: resource(document.uri), loaded: true,
        documentObject: publicDocument(document), text: document.getText(), dirty: document.isDirty,
        version: document.version, disk: null });
    }
    return { groups, active, visible, documents };
  };
  const events = [];
  const record = event => { assert.ok(events.length < 512); events.push(event); };
  const nonFixtureDocument = document => !['file', 'untitled'].includes(document.uri.scheme);
  const supplementalDocument = document => {
    const uri = document.uri.toString(true), languageId = document.languageId;
    assert.ok(Buffer.byteLength(uri) <= 4096, 'Supplemental URI budget exceeded');
    assert.ok(typeof languageId === 'string' && Buffer.byteLength(languageId) <= 128);
    assert.ok(Buffer.byteLength(document.uri.scheme) <= 64);
    return { uri, scheme: document.uri.scheme, documentObject: publicDocument(document),
      ...(!nonFixtureDocument(document) ? { resource: resource(document.uri) } : {}),
      languageId, dirty: document.isDirty, version: document.version };
  };
  const supplement = (kind, fields) => supplemental.record({ kind, phase: { ...phase },
    fixtureEventCount: events.length, ...fields });
  const listeners = [
    vscode.window.tabGroups.onDidChangeTabs(guard.listen(event => record({ kind: 'tabs',
      opened: event.opened.map(tabState), closed: event.closed.map(tabState), changed: event.changed.map(tabState) }))),
    vscode.window.tabGroups.onDidChangeTabGroups(guard.listen(event => record({ kind: 'groups',
      opened: event.opened.map(group => group.viewColumn), closed: event.closed.map(group => group.viewColumn),
      changed: event.changed.map(group => group.viewColumn) }))),
    vscode.window.onDidChangeActiveTextEditor(guard.listen(editor => {
      if (editor && nonFixtureDocument(editor.document) && phase.kind === 'readiness') {
        supplement('active', { document: supplementalDocument(editor.document), viewColumn: editor.viewColumn ?? null });
      } else record({ kind: 'active', editor: editor ? editorState(editor) : null });
    })),
    vscode.window.onDidChangeVisibleTextEditors(guard.listen(editors => {
      assert.ok(editors.length <= 4);
      if (phase.kind === 'readiness' && editors.some(editor => nonFixtureDocument(editor.document))) {
        supplement('visible', { editors: editors.map(editor => ({ document: supplementalDocument(editor.document),
          viewColumn: editor.viewColumn ?? null })) });
      } else record({ kind: 'visible', editors: editors.map(editorState) });
    })),
    vscode.window.onDidChangeTextEditorSelection(guard.listen(event => {
      if (phase.kind === 'readiness' && nonFixtureDocument(event.textEditor.document)) {
        supplement('selection', { document: supplementalDocument(event.textEditor.document),
          viewColumn: event.textEditor.viewColumn ?? null, selectionKind: event.kind ?? null });
      } else record({ kind: 'selection', selectionKind: event.kind ?? null, editor: editorState(event.textEditor) });
    })),
    vscode.workspace.onDidChangeTextDocument(guard.listen(event => {
      if (nonFixtureDocument(event.document)) {
        assert.ok(event.contentChanges.length <= 128, 'Supplemental change count budget exceeded');
        let textBytes = 0;
        const changes = event.contentChanges.map(change => {
          textBytes += Buffer.byteLength(change.text);
          assert.ok(textBytes <= 64 * 1024, 'Supplemental change text budget exceeded');
          return { text: change.text, rangeOffset: change.rangeOffset, rangeLength: change.rangeLength };
        });
        supplement('document-change', { document: supplementalDocument(event.document), changes });
      } else record({ kind: 'document-change', resource: resource(event.document.uri),
        documentObject: publicDocument(event.document), dirty: event.document.isDirty, version: event.document.version,
        changes: event.contentChanges.map(change => ({ text: change.text,
          rangeOffset: change.rangeOffset, rangeLength: change.rangeLength })) });
    })),
    vscode.workspace.onDidCloseTextDocument(guard.listen(document => {
      if (nonFixtureDocument(document)) supplement('document-close', { document: supplementalDocument(document) });
      else record({ kind: 'document-close', resource: resource(document.uri), documentObject: publicDocument(document) });
    })),
  ];
  const settle = async () => {
    const start = Date.now(), deadline = start + 3000;
    let previous, unchangedAt = start, reads = 0;
    for (;;) {
      const observed = snapshot(), fingerprint = JSON.stringify(observed); reads++;
      if (fingerprint !== previous) { previous = fingerprint; unchangedAt = Date.now(); }
      if (Date.now() - unchangedAt >= 100) return { observed, reads, elapsedMs: Date.now() - start,
        scope: 'Awaited command completion then 100ms unchanged public state; no target retry or expected-output predicate' };
      assert.ok(Date.now() < deadline, 'Independent public group settlement exceeded three seconds');
      await delay(20);
    }
  };
  let setup;
  const observations = [], settlements = [], details = [];
  try {
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(canonicalWorkspace, 'a.txt')));
    await vscode.window.showTextDocument(document, { preview: true, viewColumn: vscode.ViewColumn.One });
    const prepared = await settle();
    setup = { commandInventory: required, effectiveConfiguration: effective,
      action: 'api.openTextDocument/showTextDocument', resource: 'a.txt', settlement: prepared,
      scope: 'Initial file API setup only; every following tab/group/navigation/edit target uses its original public command once' };
    observations.push({ action: 'initial', ...prepared.observed });
    settlements.push(prepared);
    for (const [index, step] of fixture.steps.entries()) {
      phase = { kind: 'target', index };
      const eventStart = events.length, supplementalStart = supplemental.events.length;
      if (step.requireSharedDirty) {
        const before = snapshot();
        assert.ok(before.active?.dirty, 'Shared-dirty-close precondition missing');
        assert.ok(before.groups.flatMap(group => group.tabs).filter(tab => tab.resource === before.active.resource).length >= 2,
          'Refuse to introduce a last-membership dirty dialog into this no-discard fixture');
      }
      let action, args;
      if (step.open) {
        assert.ok(Object.hasOwn(files, step.open));
        action = 'vscode.open';
        args = [vscode.Uri.file(path.join(canonicalWorkspace, step.open)), { preview: step.preview ?? true }];
      } else { action = step.command; args = Object.hasOwn(step, 'args') ? [step.args] : []; }
      let timeout;
      try {
        await Promise.race([
          vscode.commands.executeCommand(action, ...args),
          new Promise((_resolve, reject) => { timeout = setTimeout(() => reject(
            new Error(`Original command acknowledgement exceeded five seconds: ${action}`)), 5000); }),
        ]);
      } finally { clearTimeout(timeout); }
      const settled = await settle();
      observations.push({ action, ...(step.open ? { opened: step.open, requestedPreview: step.preview ?? true } : {}), ...settled.observed });
      settlements.push(settled);
      details.push({ step: index, gesture: step, events: events.slice(eventStart),
        supplementalEvents: supplemental.events.slice(supplementalStart) });
    }
    guard.check();
    for (const [name, text] of Object.entries(files)) assert.equal(fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8'), text,
      'Tab/group/navigation/type/Undo capture wrote fixture disk');
    return { name, setup, observations, settlements, details, events, supplementalEvents: supplemental.events,
      supplementalScope: 'Bounded non-file document change/close evidence and readiness-only non-file views; target editors and all file resources remain strict',
      scope: 'Preview and sticky text-file/Untitled tabs, explicit default positioning/revealIfOpen/MRU policy, public groups and visible selections; no graphical double-click or private stack access' };
  } catch (error) {
    onFailure({ name, phase: { ...phase }, setup: setup ?? null, observations, settlements, details,
      events, supplementalEvents: supplemental.events });
    throw error;
  } finally { for (const listener of listeners) listener.dispose(); }
}
module.exports = { editorPreviewTabsTrace, files, policy, fixtureFileResource, eventGuard, supplementalRecorder };
