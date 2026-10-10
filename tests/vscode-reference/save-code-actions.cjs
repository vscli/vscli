'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./save-code-actions-cases.json');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const TEXT = 'fix=0\r\nimports=0\r\nchild=0\r\nother=0\r\n猫🙂\r\n';
exports.text = TEXT;
exports.saveCodeActionsTrace = async (vscode, name, workspace) => {
  const fixture = cases.find(value => value.name === name);
  assert.ok(fixture, 'Unknown save code actions fixture');
  const file = path.join(workspace, 'main.txt');
  assert.equal(fs.readFileSync(file, 'utf8'), TEXT);
  const commands = new Set(await vscode.commands.getCommands(true));
  const required = ['type', 'workbench.action.files.save', 'undo', 'redo'];
  for (const command of required) assert.ok(commands.has(command), `Original command missing: ${command}`);
  const callbacks = [], saved = [], changes = [];
  let phase = 'setup', document, editor;
  const push = (list, value) => { assert.ok(list.length < 256, 'Observer event budget exceeded'); list.push(value); };
  const definitions = [
    { title: 'Synthetic imports', kind: 'source.organizeImports', line: 1, text: 'imports=1' },
    { title: 'Synthetic child fix', kind: 'source.fixAll.child', line: 2, text: 'child=1' },
    { title: 'Synthetic root fix', kind: 'source.fixAll', line: 0, text: 'fix=1' },
    { title: 'Synthetic unrelated prefix', kind: 'source.fixAllX', line: 3, text: 'other=1' },
  ];
  const selector = { scheme: 'file', language: 'plaintext', pattern: new vscode.RelativePattern(workspace, 'main.txt') };
  const registration = vscode.languages.registerCodeActionsProvider(selector, {
    provideCodeActions(doc, range, context, token) {
      // Deliberately return every kind: the editor must filter on hierarchy,
      // exclusions and the requested family, rather than trusting this provider.
      const actions = definitions.map(definition => {
        const action = new vscode.CodeAction(definition.title, new vscode.CodeActionKind(definition.kind));
        action.edit = new vscode.WorkspaceEdit();
        action.edit.replace(doc.uri, doc.lineAt(definition.line).range, definition.text);
        return action;
      });
      push(callbacks, { phase, only: context.only?.value ?? null, triggerKind: context.triggerKind,
        version: doc.version, text: doc.getText(), cancelled: token.isCancellationRequested,
        range: [range.start.line, range.start.character, range.end.line, range.end.character],
        returned: definitions.map(value => ({ title: value.title, kind: value.kind })) });
      return actions;
    }
  }, { providedCodeActionKinds: definitions.map(value => new vscode.CodeActionKind(value.kind)) });
  document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  editor = await vscode.window.showTextDocument(document, { preview: false });
  const end = document.positionAt(document.getText().length);
  editor.selection = new vscode.Selection(end, end);
  const scalar = position => [...document.getText().slice(0, document.offsetAt(position))].length;
  const proof = () => ({ text: document.getText(), version: document.version, dirty: document.isDirty,
    selections: editor.selections.map(selection => [selection.anchor.line, selection.anchor.character,
      selection.active.line, selection.active.character]), disk: fs.readFileSync(file, 'utf8') });
  const state = action => ({ action, resource: 'main.txt', text: document.getText(), dirty: document.isDirty,
    primary: { anchor: scalar(editor.selection.anchor), cursor: scalar(editor.selection.active) },
    disk: fs.readFileSync(file, 'utf8') });
  const saveListener = vscode.workspace.onDidSaveTextDocument(doc => {
    if (doc === document) push(saved, { phase, version: doc.version, text: doc.getText(), dirty: doc.isDirty,
      disk: fs.readFileSync(file, 'utf8') });
  });
  const changeListener = vscode.workspace.onDidChangeTextDocument(event => {
    if (event.document === document) push(changes, { phase, version: document.version,
      text: document.getText(), dirty: document.isDirty, reason: event.reason ?? null,
      changes: event.contentChanges.map(change => ({ rangeOffset: change.rangeOffset,
        rangeLength: change.rangeLength, text: change.text })) });
  });
  async function stable() {
    const start = Date.now(); let changed = start, previous;
    for (;;) {
      const key = JSON.stringify(proof());
      if (key !== previous) { previous = key; changed = Date.now(); }
      if (Date.now() - start >= 100 && Date.now() - changed >= 50) return {
        elapsedMs: Date.now() - start,
        scope: 'Actual command/didSave acknowledgement followed by minimum100ms/50ms stable public state; no preferred target-output predicate or retry' };
      assert.ok(Date.now() - start < 3000, 'Public state did not settle');
      await pause(10);
    }
  }
  async function didSave(after) {
    const start = Date.now();
    while (saved.length === after) {
      assert.ok(Date.now() - start < 8000, 'Actual didSave acknowledgement absent');
      await pause(10);
    }
    assert.equal(saved.length, after + 1, 'Unexpected duplicate target save');
    assert.equal(saved.at(-1).disk, saved.at(-1).text, 'didSave precedes disk commit');
    return { elapsedMs: Date.now() - start, event: saved.at(-1) };
  }
  try {
    assert.equal(document.eol, vscode.EndOfLine.CRLF);
    const configuration = vscode.workspace.getConfiguration('editor', document);
    const effective = { codeActionsOnSave: configuration.get('codeActionsOnSave'),
      formatOnSave: configuration.get('formatOnSave'),
      autoSave: vscode.workspace.getConfiguration('files', document).get('autoSave'),
      autoSaveDelay: vscode.workspace.getConfiguration('files', document).get('autoSaveDelay') };
    assert.equal(effective.autoSave, fixture.autosave); assert.equal(effective.formatOnSave, false);
    const before = proof();
    const warm = await vscode.commands.executeCommand('vscode.executeCodeActionProvider', document.uri,
      new vscode.Range(new vscode.Position(0, 0), end), 'source', 100);
    assert.ok(Array.isArray(warm) && warm.length === definitions.length, 'Positive source-action API readiness failed');
    assert.equal(callbacks.length, 1, 'Readiness callback count differs');
    assert.deepEqual(proof(), before, 'Readiness changed target text/version/selection/disk');
    const setup = { effective, effectiveKeys: Array.isArray(effective.codeActionsOnSave) ? null : Object.keys(effective.codeActionsOnSave),
      configurationInspect: configuration.inspect('codeActionsOnSave'), before, after: proof(),
      warmKinds: warm.map(action => action.kind?.value ?? null),
      scope: 'One public execute-source-provider call; no edits applied, exact unchanged proof and no readiness retry' };
    phase = 'target';
    const observations = [state('initial')], steps = [];
    const count = saved.length;
    await vscode.commands.executeCommand('type', { text: 'λ🙂' });
    steps.push({ action: 'type', settlement: await stable() }); observations.push(state('type'));
    const saveAction = fixture.autosave === 'afterDelay' ? 'files.autoSave.afterDelay' : 'workbench.action.files.save';
    if (fixture.autosave !== 'afterDelay') await vscode.commands.executeCommand(saveAction);
    steps.push({ action: saveAction, acknowledgement: await didSave(count), settlement: await stable() });
    observations.push(state(saveAction));
    if (fixture.autosave === 'off') {
      // Five steps cover the four possible mutations plus typed input. Fixed
      // counts retain no-ops/unexpected grouping rather than retrying to a goal.
      for (const command of [...Array(5).fill('undo'), ...Array(5).fill('redo')]) {
        await vscode.commands.executeCommand(command);
        steps.push({ action: command, settlement: await stable() }); observations.push(state(command));
      }
    }
    return { name, setup, observations, steps, callbacks, saved, changes,
      commandInventory: required.sort(), executeApiListed: commands.has('vscode.executeCodeActionProvider'),
      scope: 'Pinned synthetic sourceActions, layered/language settings and original Save/Undo/Redo; exact callback order and text/disk/selection. Not native LSP, unchanged extension or full save-lifecycle parity' };
  } finally { saveListener.dispose(); changeListener.dispose(); registration.dispose(); }
};
