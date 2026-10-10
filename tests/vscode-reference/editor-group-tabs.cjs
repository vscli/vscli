'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./editor-group-tabs-cases.json');
const files = {
  'a.txt': '猫🙂 alpha\r\nline-a\r\n', 'b.txt': 'β🙂 beta\r\nline-b\r\n',
  'c.txt': 'γ🙂 gamma\r\nline-c\r\n', 'd.txt': 'δ🙂 delta\r\nline-d\r\n',
};
const policy = {
  'workbench.editor.enablePreview': false,
  'workbench.editor.enablePreviewFromQuickOpen': false,
  'workbench.editor.openPositioning': 'right',
  'workbench.editor.focusRecentEditorAfterClose': true,
  'workbench.editor.closeEmptyGroups': true,
  'workbench.editor.revealIfOpen': false,
};
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

async function editorGroupTabsTrace(vscode, name, workspace) {
  const fixture = cases.find(item => item.name === name);
  assert.ok(fixture, 'Unknown editor-group-tabs case');
  assert.ok(fixture.steps.length <= 16);
  const commands = await vscode.commands.getCommands(true);
  const required = [...new Set(['vscode.open', ...fixture.steps.filter(step => step.command).map(step => step.command)])];
  for (const command of required) assert.ok(commands.includes(command), `Pinned command unavailable: ${command}`);
  const effective = {};
  for (const [key, expected] of Object.entries(policy)) {
    const split = key.lastIndexOf('.');
    effective[key] = vscode.workspace.getConfiguration(key.slice(0, split)).get(key.slice(split + 1));
    assert.deepEqual(effective[key], expected, `Committed-tab fixture configuration changed: ${key}`);
  }
  const canonicalWorkspace = fs.realpathSync(workspace);
  const resource = uri => {
    assert.equal(uri.scheme, 'file', 'Non-file editor escaped the fixture');
    const relative = path.relative(canonicalWorkspace, fs.realpathSync(uri.fsPath)).split(path.sep).join('/');
    assert.ok(Object.hasOwn(files, relative), `Editor escaped fixture resource: ${relative}`);
    return relative;
  };
  const documentIds = new WeakMap();
  let nextDocument = 1;
  const publicDocument = document => {
    if (!documentIds.has(document)) documentIds.set(document, nextDocument++);
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
    const all = vscode.window.tabGroups.all;
    assert.ok(all.length <= 4);
    assert.ok(all.every(group => group.tabs.length <= 16));
    const groups = all.map(group => ({ viewColumn: group.viewColumn, active: group.isActive,
      tabs: group.tabs.map(tabState) }));
    assert.ok(vscode.window.visibleTextEditors.length <= 4);
    assert.ok(vscode.workspace.textDocuments.length <= 128);
    const visible = vscode.window.visibleTextEditors.map(editorState).sort((a, b) => a.viewColumn - b.viewColumn);
    const active = vscode.window.activeTextEditor ? editorState(vscode.window.activeTextEditor) : null;
    const documents = Object.keys(files).map(name => {
      const document = vscode.workspace.textDocuments.find(doc => doc.uri.scheme === 'file'
        && fs.realpathSync(doc.uri.fsPath) === path.join(canonicalWorkspace, name));
      return { resource: name, loaded: Boolean(document), documentObject: document ? publicDocument(document) : null,
        text: document ? document.getText() : fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8'),
        dirty: document?.isDirty ?? false, version: document?.version ?? null,
        disk: fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8') };
    });
    return { groups, active, visible, documents };
  };
  const events = [];
  const record = event => { assert.ok(events.length < 512); events.push(event); };
  const listeners = [
    vscode.window.tabGroups.onDidChangeTabs(event => record({ kind: 'tabs',
      opened: event.opened.map(tabState), closed: event.closed.map(tabState), changed: event.changed.map(tabState) })),
    vscode.window.tabGroups.onDidChangeTabGroups(event => record({ kind: 'groups',
      opened: event.opened.map(group => group.viewColumn), closed: event.closed.map(group => group.viewColumn),
      changed: event.changed.map(group => group.viewColumn) })),
    vscode.window.onDidChangeActiveTextEditor(editor => record({ kind: 'active', editor: editor ? editorState(editor) : null })),
    vscode.window.onDidChangeVisibleTextEditors(editors => record({ kind: 'visible', editors: editors.map(editorState) })),
    vscode.window.onDidChangeTextEditorSelection(event => record({ kind: 'selection',
      selectionKind: event.kind ?? null, editor: editorState(event.textEditor) })),
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
  try {
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(canonicalWorkspace, 'a.txt')));
    await vscode.window.showTextDocument(document, { preview: false, viewColumn: vscode.ViewColumn.One });
    const prepared = await settle();
    const setup = { commandInventory: required, effectiveConfiguration: effective,
      action: 'api.openTextDocument/showTextDocument', resource: 'a.txt', settlement: prepared,
      scope: 'Initial file API setup only; every following tab/group/navigation/edit target uses its original public command once' };
    const observations = [{ action: 'initial', ...prepared.observed }], settlements = [prepared], details = [];
    for (const [index, step] of fixture.steps.entries()) {
      const eventStart = events.length;
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
        args = [vscode.Uri.file(path.join(canonicalWorkspace, step.open)), { preview: false }];
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
      observations.push({ action, ...(step.open ? { opened: step.open } : {}), ...settled.observed });
      settlements.push(settled);
      details.push({ step: index, gesture: step, events: events.slice(eventStart) });
    }
    for (const [name, text] of Object.entries(files)) assert.equal(fs.readFileSync(path.join(canonicalWorkspace, name), 'utf8'), text,
      'Tab/group/navigation/type/Undo capture wrote fixture disk');
    return { name, setup, observations, settlements, details, events,
      scope: 'Committed text-file tabs, explicit default positioning/MRU/close-empty policy, public groups and visible editor selections only; no private workbench/MRU/layout access' };
  } finally { for (const listener of listeners) listener.dispose(); }
}
module.exports = { editorGroupTabsTrace, files, policy };
