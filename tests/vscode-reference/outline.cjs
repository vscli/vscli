'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./outline-cases.json');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const range = value => ({ start: { line: value.start.line, character: value.start.character },
  end: { line: value.end.line, character: value.end.character } });
function encodeSymbol(value) {
  const result = { name: value.name, kind: value.kind + 1 };
  if (value.range) Object.assign(result, { detail: value.detail, range: range(value.range),
    selectionRange: range(value.selectionRange), children: value.children.map(encodeSymbol) });
  else Object.assign(result, { location: { range: range(value.location.range) }, containerName: value.containerName });
  return result;
}
// Command acknowledgment and unchanged editor state do not prove that the
// separate Outline tree/focus/reveal work has completed. Pace every observation
// independently of its value, retaining a bounded deadline and quiet window.
async function settleOutline(action, { snapshot, activeCallbacks, now = Date.now, wait = pause }) {
  const started = now(); let changed = started, previous, wasActive = false;
  for (;;) {
    const observed = snapshot(action), key = JSON.stringify(observed), current = now();
    assert.ok(current - started < 3000, 'Independent Outline fixture settlement exceeded3seconds');
    const active = !!activeCallbacks();
    if (key !== previous || active || wasActive) { previous = key; changed = current; }
    wasActive = active;
    if (current - started >= 1000 && current - changed >= 100) return { observed,
      elapsedMs: current - started,
      scope: 'Original command acknowledged then minimum1000ms plus100ms unchanged public editor/document state; no expected-output predicate or target retry; tree focus is not independently observed' };
    await wait(20);
  }
}
async function observeOutlineCommand(command, { execute, ...settlement }) {
  await execute(command);
  return settleOutline(command, settlement);
}
exports.settleOutline = settleOutline;
exports.observeOutlineCommand = observeOutlineCommand;
exports.outlineTrace = async (vscode, name, workspace) => {
  const fixture = cases.find(value => value.name === name);
  assert.ok(fixture, 'Unknown Outline fixture');
  const file = path.join(workspace, 'main.cpp'), original = fs.readFileSync(file, 'utf8');
  assert.equal(original, fixture.text);
  const document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const available = new Set(await vscode.commands.getCommands(true));
  for (const command of fixture.commands)
    assert.ok(available.has(command), `Pinned original command unavailable: ${command}`);
  if (fixture.setupOutlineFocus) await vscode.commands.executeCommand('outline.focus');
  const requests = [], selectionEvents = [];
  let activeCallbacks = 0;
  const makeRange = value => new vscode.Range(value.start.line, value.start.character, value.end.line, value.end.character);
  const create = value => {
    if (fixture.shape === 'flat') return new vscode.SymbolInformation(value.name, value.kind - 1,
      value.containerName, new vscode.Location(document.uri, makeRange(value.location.range)));
    const node = new vscode.DocumentSymbol(value.name, value.detail, value.kind - 1,
      makeRange(value.range), makeRange(value.selectionRange));
    node.children = value.children.map(create); return node;
  };
  const registration = vscode.languages.registerDocumentSymbolProvider({ language: 'cpp', scheme: 'file',
    pattern: new vscode.RelativePattern(workspace, 'main.cpp') }, {
    provideDocumentSymbols(value, token) {
      activeCallbacks++;
      try {
        requests.push({ uri: value.uri.toString(), sameDocument: value === document,
          version: value.version, cancelledAtInvocation: token.isCancellationRequested });
        assert.ok(requests.length <= 128, 'Unbounded synthetic fixture callback trace');
        return fixture.symbols.map(create);
      } finally { activeCallbacks--; }
    },
  }, { label: 'VSCLI named synthetic Outline API fixture' });
  const scalar = position => [...document.getText().slice(0, document.offsetAt(position))].length;
  const snapshot = action => {
    const active = vscode.window.activeTextEditor;
    assert.ok(active && active.document === document, 'Outline gesture lost fixture editor');
    return { action, resource: 'main.cpp', text: document.getText(), dirty: document.isDirty,
      version: document.version, primary: { anchor: scalar(active.selection.anchor), cursor: scalar(active.selection.active) },
      selections: active.selections.map(selection => ({ anchor: scalar(selection.anchor), cursor: scalar(selection.active) })) };
  };
  const listener = vscode.window.onDidChangeTextEditorSelection(event => {
    if (event.textEditor.document !== document) return;
    selectionEvents.push({ kind: event.kind ?? null, primary: { anchor: scalar(event.selections[0].anchor),
      cursor: scalar(event.selections[0].active) } });
    assert.ok(selectionEvents.length <= 512, 'Unbounded fixture selection events');
  });
  const settlementOptions = { snapshot, activeCallbacks: () => activeCallbacks };
  try {
    const readinessStarted = Date.now();
    if (fixture.setupOutlineFocus) {
      // No execute-provider API or target command runs during this barrier.
      // With breadcrumbs disabled, this callback proves the visible Outline
      // requested the newly registered source; no desired tree/selection result
      // is used to wait or retry.
      while (!requests.length) {
        assert.ok(Date.now() - readinessStarted < 8000, 'Visible Outline did not request the registered source');
        await pause(20);
      }
    }
    const setup = { action: 'api.openTextDocument/showTextDocument/registerDocumentSymbolProvider',
      settlement: await settleOutline('initial', settlementOptions), shape: fixture.shape, outlineFocusBeforeRegistration: !!fixture.setupOutlineFocus,
      providerReadinessMs: Date.now() - readinessStarted, callbacksBeforeApiOrTargets: requests.length };
    const supplied = await vscode.commands.executeCommand('vscode.executeDocumentSymbolProvider', document.uri);
    assert.ok(Array.isArray(supplied), 'Actual provider command did not return symbols');
    const actualSymbols = supplied.map(encodeSymbol);
    const observations = [snapshot('initial')], steps = [];
    for (const command of fixture.commands) {
      const eventStart = selectionEvents.length, requestStart = requests.length;
      const settlement = await observeOutlineCommand(command, {
        ...settlementOptions, execute: command => vscode.commands.executeCommand(command),
      });
      observations.push(settlement.observed);
      steps.push({ command, settlement, selectionEvents: selectionEvents.slice(eventStart),
        callbacks: requests.slice(requestStart) });
    }
    assert.equal(fs.readFileSync(file, 'utf8'), original, 'Outline gestures wrote fixture disk');
    return { name, scope: 'Named synthetic DocumentSymbolProvider public API and original command/editor projection; not unchanged extension qualification or private sidebar tree differential',
      setup, actualSymbols, observations, steps, requests, executeApiListed: available.has('vscode.executeDocumentSymbolProvider'), commandInventory: [...available].filter(value =>
        fixture.commands.includes(value) || value.startsWith('outline.') || ['list.focusFirst', 'list.focusDown', 'list.expand', 'list.collapse', 'list.select'].includes(value)).sort() };
  } finally { listener.dispose(); registration.dispose(); }
};
