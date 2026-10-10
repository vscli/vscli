'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const corpus = require('./navigation-history-cases.json');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

function snapshot(vscode, workspace, fixture) {
  const editor = vscode.window.activeTextEditor;
  assert.ok(editor, 'Target lost its active text editor');
  const text = editor.document.getText();
  const scalar = position => [...text.slice(0, editor.document.offsetAt(position))].length;
  const resource = path.relative(workspace, editor.document.uri.fsPath).split(path.sep).join('/');
  assert.ok(fixture.files.some(file => file.name === resource), 'Active editor escaped fixture resources');
  const documents = fixture.files.map(file => {
    const document = vscode.workspace.textDocuments.find(document =>
      document.uri.scheme === 'file' && path.resolve(document.uri.fsPath) === path.resolve(workspace, file.name));
    return { resource: file.name, text: document?.getText() ?? fs.readFileSync(path.join(workspace, file.name), 'utf8'),
      dirty: document?.isDirty ?? false, version: document?.version ?? null,
      disk: fs.readFileSync(path.join(workspace, file.name), 'utf8') };
  });
  return { resource, text, primary: { anchor: scalar(editor.selection.anchor), cursor: scalar(editor.selection.active) },
    dirty: editor.document.isDirty, version: editor.document.version,
    eol: editor.document.eol === vscode.EndOfLine.CRLF ? 'CRLF' : 'LF',
    selections: editor.selections.map(selection => ({ anchor: scalar(selection.anchor), cursor: scalar(selection.active) })),
    viewColumn: editor.viewColumn, documents };
}

// Reads settle independently of any expected target output. Gestures never retry.
async function settle(vscode, workspace, fixture) {
  const started = Date.now(), deadline = started + 2000;
  let previous, unchangedAt = started, reads = 0;
  for (;;) {
    const observed = snapshot(vscode, workspace, fixture);
    const fingerprint = JSON.stringify(observed);
    reads++;
    if (fingerprint !== previous) { previous = fingerprint; unchangedAt = Date.now(); }
    if (Date.now() - unchangedAt >= 100) return { observed, reads, elapsedMs: Date.now() - started,
      scope: 'Public command acknowledgement followed by 100ms unchanged public editor/document reads; no expected-output predicate or gesture retry' };
    assert.ok(Date.now() < deadline, 'Independent public editor settlement exceeded two seconds');
    await delay(20);
  }
}

async function navigationHistoryTrace(vscode, name, workspace) {
  const fixture = corpus.find(fixture => fixture.name === name);
  assert.ok(fixture);
  const commands = await vscode.commands.getCommands(true);
  const required = ['vscode.open', 'workbench.action.quickOpen', 'workbench.action.acceptSelectedQuickOpenItem',
    'workbench.action.navigateBack', 'workbench.action.navigateForward', 'editor.action.forceRetokenize'];
  for (const step of fixture.steps) if (step.command) required.push(step.command);
  for (const command of new Set(required)) assert.ok(commands.includes(command), `Pinned command unavailable: ${command}`);
  const events = [];
  const listener = vscode.window.onDidChangeTextEditorSelection(event => {
    assert.ok(events.length < 4096, 'Selection event evidence exceeds 4096 entries');
    const text = event.textEditor.document.getText();
    const scalar = position => [...text.slice(0, event.textEditor.document.offsetAt(position))].length;
    events.push({ resource: path.basename(event.textEditor.document.uri.fsPath), kind: event.kind ?? null,
      selections: event.selections.map(selection => ({ anchor: scalar(selection.anchor), cursor: scalar(selection.active) })) });
  });
  try {
    const setup = [];
    const uri = vscode.Uri.file(path.join(workspace, fixture.setup.open));
    const document = await vscode.workspace.openTextDocument(uri);
    const editor = await vscode.window.showTextDocument(document, { preview: false, viewColumn: vscode.ViewColumn.One });
    setup.push({ action: 'api.openTextDocument/showTextDocument', resource: fixture.setup.open,
      settlement: await settle(vscode, workspace, fixture) });
    if (fixture.setup.selection) {
      const position = scalar => document.positionAt([...document.getText()].slice(0, scalar).join('').length);
      editor.selection = new vscode.Selection(position(fixture.setup.selection.anchor), position(fixture.setup.selection.cursor));
      setup.push({ action: 'api.primarySelection', selection: fixture.setup.selection,
        settlement: await settle(vscode, workspace, fixture) });
    }
    if (fixture.setup.prepareTokens) {
      const before = snapshot(vscode, workspace, fixture);
      await vscode.commands.executeCommand('editor.action.forceRetokenize');
      const after = snapshot(vscode, workspace, fixture);
      assert.deepEqual(after, before, 'Setup token preparation changed bytes/version/selections/dirty state');
      setup.push({ action: 'public.forceRetokenize', before, after,
        scope: 'Code-only bracket fixture; no scratch editor or extra navigation was introduced' });
    }
    const initial = await settle(vscode, workspace, fixture);
    const observations = [{ action: 'initial', ...initial.observed }], settlements = [initial], details = [];
    for (const [index, step] of fixture.steps.entries()) {
      const eventStart = events.length;
      let action, substeps;
      if (step.open) {
        assert.ok(fixture.files.some(file => file.name === step.open));
        await vscode.commands.executeCommand('vscode.open', vscode.Uri.file(path.join(workspace, step.open)),
          { preview: false, viewColumn: vscode.ViewColumn.One });
        action = 'open';
      } else if (step.gotoLine) {
        assert.ok(Number.isInteger(step.gotoLine) && step.gotoLine > 0);
        await vscode.commands.executeCommand('workbench.action.quickOpen', `:${step.gotoLine}`);
        const preview = await settle(vscode, workspace, fixture);
        await vscode.commands.executeCommand('workbench.action.acceptSelectedQuickOpenItem');
        action = 'gotoLine';
        substeps = [{ command: 'workbench.action.quickOpen', argument: `:${step.gotoLine}`, preview },
          { command: 'workbench.action.acceptSelectedQuickOpenItem' }];
      } else if (Object.hasOwn(step, 'type')) {
        assert.equal([...step.type].length, 1);
        await vscode.commands.executeCommand('type', { text: step.type });
        action = 'type';
      } else {
        assert.ok(step.command);
        await vscode.commands.executeCommand(step.command, ...(Object.hasOwn(step, 'args') ? [step.args] : []));
        action = step.command;
      }
      const settlement = await settle(vscode, workspace, fixture);
      observations.push({ action, ...settlement.observed });
      settlements.push(settlement);
      details.push({ step: index, gesture: step, events: events.slice(eventStart), ...(substeps ? { substeps } : {}) });
    }
    for (const file of fixture.files) assert.equal(fs.readFileSync(path.join(workspace, file.name), 'utf8'), file.text,
      'Navigation/edit/Undo capture wrote its fixture disk');
    return { name, setup, observations, settlements, details, events,
      scope: 'Single editor group, primary history projection; API setup separated, public selection-event kinds retained, no private history stack access' };
  } finally { listener.dispose(); }
}
module.exports = { navigationHistoryTrace };
