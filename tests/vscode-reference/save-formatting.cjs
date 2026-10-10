'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./save-formatting-cases.json');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
exports.saveFormattingTrace = async (vscode, name, workspace) => {
  const fixture = cases.find(value => value.name === name);
  assert.ok(fixture, 'Unknown save formatting fixture');
  const file = path.join(workspace, 'main.txt');
  assert.equal(fs.readFileSync(file, 'utf8'), fixture.text);
  const commands = new Set(await vscode.commands.getCommands(true));
  for (const command of fixture.commands) assert.ok(commands.has(command), `Original command missing: ${command}`);
  const callbacks = [], saved = [], changes = [], registrations = [];
  let phase = 'setup', document, editor;
  const boundedPush = (list, value) => { assert.ok(list.length < 128, 'Synthetic observer event budget exceeded'); list.push(value); };
  const selector = { scheme: 'file', language: 'plaintext', pattern: new vscode.RelativePattern(workspace, 'main.txt') };
  if (fixture.sourceAction) {
    registrations.push(vscode.languages.registerCodeActionsProvider(selector, {
      provideCodeActions(doc, _range, context) {
        const only = context.only?.value ?? null;
        if (!context.only || !context.only.contains(vscode.CodeActionKind.SourceOrganizeImports)) return [];
        boundedPush(callbacks, { phase, kind: 'sourceAction', only, version: doc.version, text: doc.getText() });
        const action = new vscode.CodeAction('Synthetic organize imports before format', vscode.CodeActionKind.SourceOrganizeImports);
        action.edit = new vscode.WorkspaceEdit();
        action.edit.replace(doc.uri, doc.lineAt(0).range, 'imported 猫🙂');
        return [action];
      }
    }, { providedCodeActionKinds: [vscode.CodeActionKind.SourceOrganizeImports] }));
  }
  registrations.push(vscode.languages.registerDocumentFormattingEditProvider(selector, {
    provideDocumentFormattingEdits(doc, options, token) {
      boundedPush(callbacks, { phase, kind: 'format', sameDocument: doc === document, version: doc.version,
        text: doc.getText(), options: { tabSize: options.tabSize, insertSpaces: options.insertSpaces },
        cancelledAtInvocation: token.isCancellationRequested, result: fixture.format });
      if (fixture.format === 'null') return null;
      const line = doc.lineAt(1);
      return line.text === 'let value=1;' ? [vscode.TextEdit.replace(line.range, 'let value = 1;')] : [];
    }
  }));
  document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  editor = await vscode.window.showTextDocument(document, { preview: false });
  editor.selection = new vscode.Selection(document.positionAt(document.getText().length), document.positionAt(document.getText().length));
  const scalar = position => [...document.getText().slice(0, document.offsetAt(position))].length;
  const state = action => ({ action, resource: 'main.txt', text: document.getText(), dirty: document.isDirty,
    primary: { anchor: scalar(editor.selection.anchor), cursor: scalar(editor.selection.active) },
    disk: fs.readFileSync(file, 'utf8') });
  const editorProof = () => ({ version: document.version, text: document.getText(), dirty: document.isDirty,
    selections: editor.selections.map(s => [s.anchor.line, s.anchor.character, s.active.line, s.active.character]),
    disk: fs.readFileSync(file, 'utf8') });
  const saveListener = vscode.workspace.onDidSaveTextDocument(doc => {
    if (doc !== document) return;
    boundedPush(saved, { phase, version: doc.version, text: doc.getText(), dirty: doc.isDirty,
      disk: fs.readFileSync(file, 'utf8') });
  });
  const changeListener = vscode.workspace.onDidChangeTextDocument(event => {
    if (event.document !== document) return;
    boundedPush(changes, { phase, version: document.version, text: document.getText(), dirty: document.isDirty,
      reason: event.reason ?? null, changes: event.contentChanges.map(change => ({ rangeOffset: change.rangeOffset,
        rangeLength: change.rangeLength, text: change.text })) });
  });
  async function stable() {
    const start = Date.now(); let changed = start, previous;
    for (;;) {
      const key = JSON.stringify(editorProof());
      if (key !== previous) { previous = key; changed = Date.now(); }
      if (Date.now() - start >= 100 && Date.now() - changed >= 50) return {
        elapsedMs: Date.now() - start, scope: 'Command or actual didSave acknowledged, then minimum100ms/50ms unchanged public state; no preferred output predicate or target retry' };
      assert.ok(Date.now() - start < 3000, 'Public state failed to settle within three seconds');
      await pause(10);
    }
  }
  async function didSave(after) {
    const started = Date.now();
    while (saved.length === after) {
      assert.ok(Date.now() - started < 8000, 'Actual didSave acknowledgement absent');
      await pause(10);
    }
    assert.equal(saved.length, after + 1, 'Unexpected duplicate fixture save');
    assert.equal(saved.at(-1).disk, saved.at(-1).text, 'didSave precedes committed disk bytes');
    return { elapsedMs: Date.now() - started, event: saved.at(-1) };
  }
  try {
    assert.equal(document.eol, vscode.EndOfLine.CRLF);
    const effective = { formatOnSave: vscode.workspace.getConfiguration('editor', document).get('formatOnSave'),
      formatOnSaveMode: vscode.workspace.getConfiguration('editor', document).get('formatOnSaveMode'),
      autoSave: vscode.workspace.getConfiguration('files', document).get('autoSave'),
      autoSaveDelay: vscode.workspace.getConfiguration('files', document).get('autoSaveDelay'),
      codeActionsOnSave: vscode.workspace.getConfiguration('editor', document).get('codeActionsOnSave') };
    assert.equal(effective.formatOnSave, true); assert.equal(effective.formatOnSaveMode, 'file');
    assert.equal(effective.autoSave, fixture.autosave);
    const before = editorProof();
    const warm = await vscode.commands.executeCommand('vscode.executeFormatDocumentProvider', document.uri,
      { tabSize: 2, insertSpaces: true });
    assert.ok(Array.isArray(warm) || (fixture.format === 'null' && warm === undefined),
      'Unexpected execute-format-provider readiness result');
    assert.equal(callbacks.filter(value => value.kind === 'format' && value.phase === 'setup').length, 1,
      'No positive formatting registration readiness callback');
    assert.deepEqual(editorProof(), before, 'Readiness API changed target text/version/selections/disk');
    const setup = { effective, warmEditCount: Array.isArray(warm) ? warm.length : 0, warmReturnType: warm === undefined ? 'undefined' : 'array', before, after: editorProof(),
      scope: 'One public execute-format-provider readiness call; edits not applied; exact unchanged target proof; no readiness retry' };
    phase = 'target';
    const observations = [state('initial')], steps = [];
    const count = saved.length;
    await vscode.commands.executeCommand('type', { text: fixture.typed });
    steps.push({ action: 'type', settlement: await stable() }); observations.push(state('type'));
    if (fixture.autosave === 'afterDelay') {
      const acknowledgement = await didSave(count);
      steps.push({ action: 'files.autoSave.afterDelay', acknowledgement, settlement: await stable() });
      observations.push(state('files.autoSave.afterDelay'));
    } else {
      await vscode.commands.executeCommand('workbench.action.files.save');
      const acknowledgement = await didSave(count);
      steps.push({ action: 'workbench.action.files.save', acknowledgement, settlement: await stable() });
      observations.push(state('workbench.action.files.save'));
      for (const command of ['undo', 'redo']) {
        await vscode.commands.executeCommand(command);
        steps.push({ action: command, settlement: await stable() }); observations.push(state(command));
      }
    }
    return { name, scope: 'Named synthetic formatter/source-action APIs, original commands, exact text/disk/selection/dirty projection; file mode only. Not unchanged formatter extension, native save-participant parity, timeout behavior or entire undo ecosystem qualification',
      setup, observations, steps, callbacks, saved, changes, executeApiListed: commands.has('vscode.executeFormatDocumentProvider'), commandInventory: [...commands].filter(command =>
        fixture.commands.includes(command) || command === 'vscode.executeFormatDocumentProvider').sort() };
  } finally { saveListener.dispose(); changeListener.dispose(); for (const registration of registrations) registration.dispose(); }
};
