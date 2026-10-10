'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const cases = require('./breadcrumbs-cases.json');
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
exports.breadcrumbsTrace = async (vscode, name, workspace) => {
  const fixture = cases.find(value => value.name === name);
  assert.ok(fixture, 'Unknown Breadcrumbs fixture');
  const file = path.join(workspace, 'main.cpp'), original = fs.readFileSync(file, 'utf8');
  assert.equal(original, fixture.text);
  let document, editor;
  const available = new Set(await vscode.commands.getCommands(true));
  for (const command of fixture.commands)
    assert.ok(available.has(command), `Pinned original command unavailable: ${command}`);
  assert.ok(available.has('workbench.action.closeSidebar'), 'Pinned sidebar setup command unavailable');
  await vscode.commands.executeCommand('workbench.action.closeSidebar');
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
  }, { label: 'VSCLI named synthetic Breadcrumbs API fixture' });
  document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  editor = await vscode.window.showTextDocument(document, { preview: false });
  const configuration = vscode.workspace.getConfiguration('breadcrumbs', document);
  const effectiveSettings = { enabled: configuration.get('enabled'), filePath: configuration.get('filePath'), symbolPath: configuration.get('symbolPath') };
  assert.deepEqual(effectiveSettings, { enabled: true, filePath: 'on', symbolPath: 'on' });
  editor.selection = new vscode.Selection(fixture.selection.line, fixture.selection.character, fixture.selection.line, fixture.selection.character);
  const scalar = position => [...document.getText().slice(0, document.offsetAt(position))].length;
  const snapshot = action => {
    const active = vscode.window.activeTextEditor;
    assert.ok(active && active.document === document, 'Breadcrumbs gesture lost fixture editor');
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
  async function settle(action) {
    const started = Date.now(); let changed = started, previous;
    for (;;) {
      const observed = snapshot(action), key = JSON.stringify(observed);
      if (key !== previous || activeCallbacks) { previous = key; changed = Date.now(); }
      if (Date.now() - started >= 300 && Date.now() - changed >= 100) return { observed,
        elapsedMs: Date.now() - started,
        scope: 'Original command acknowledged then minimum300ms plus100ms unchanged public editor/document state; no expected-output predicate or target retry' };
      assert.ok(Date.now() - started < 3000, 'Independent Breadcrumbs fixture settlement exceeded3seconds');
      await pause(20);
    }
  }
  try {
    const readinessStarted = Date.now();
    {
      // No execute-provider API or target command runs during this barrier.
      // With Outline hidden, this callback proves visible Breadcrumbs
      // requested the newly registered source; no desired tree/selection result
      // is used to wait or retry.
      while (!requests.length) {
        assert.ok(Date.now() - readinessStarted < 8000, 'Visible Breadcrumbs did not request the registered source');
        await pause(20);
      }
    }
    await pause(1500); // Fixed output-independent post-callback publication interval; targets never retry.
    const setup = { warmPublicationDelayMs: 1500, providerRegisteredBeforeOpen: true, action: 'original workbench.action.closeSidebar setup; api.registerDocumentSymbolProvider/openTextDocument/showTextDocument/setSelection',
      settlement: await settle('initial'), shape: fixture.shape, effectiveSettings, initialSelection: fixture.selection, breadcrumbsEnabledBeforeRegistration: true,
      providerReadinessMs: Date.now() - readinessStarted, callbacksBeforeApiOrTargets: requests.length };
    const supplied = await vscode.commands.executeCommand('vscode.executeDocumentSymbolProvider', document.uri);
    assert.ok(Array.isArray(supplied), 'Actual provider command did not return symbols');
    const actualSymbols = supplied.map(encodeSymbol);
    const observations = [snapshot('initial')], steps = [];
    for (const command of fixture.commands) {
      const eventStart = selectionEvents.length, requestStart = requests.length;
      await vscode.commands.executeCommand(command);
      const settlement = await settle(command);
      observations.push(settlement.observed);
      steps.push({ command, settlement, selectionEvents: selectionEvents.slice(eventStart),
        callbacks: requests.slice(requestStart) });
    }
    assert.equal(fs.readFileSync(file, 'utf8'), original, 'Breadcrumbs gestures wrote fixture disk');
    return { name, scope: 'Named synthetic DocumentSymbolProvider public API and original command/editor projection; not unchanged extension qualification or private Breadcrumbs trail/focus/picker differential',
      setup, actualSymbols, observations, steps, requests, executeApiListed: available.has('vscode.executeDocumentSymbolProvider'), commandInventory: [...available].filter(value =>
        fixture.commands.includes(value) || value === 'workbench.action.closeSidebar' || value.startsWith('breadcrumbs.') || ['list.focusFirst', 'list.focusDown', 'list.select'].includes(value)).sort() };
  } finally { listener.dispose(); registration.dispose(); }
};
