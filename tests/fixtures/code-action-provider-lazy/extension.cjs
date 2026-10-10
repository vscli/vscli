'use strict';
const vscode = require('vscode');
const fs = require('node:fs');
const path = require('node:path');
exports.activate = context => {
  const originals = new WeakMap(), diagnostics = vscode.languages.createDiagnosticCollection('lazy');
  const refresh = document => {
    if (document.languageId !== 'markdown') return;
    const text = document.lineAt(0).text, offset = text.indexOf('bad');
    if (offset < 0) { diagnostics.delete(document.uri); return; }
    const diagnostic = new vscode.Diagnostic(new vscode.Range(0, offset, 0, offset + 3), 'lazy original diagnostic');
    diagnostic.code = 'lazy-fix';
    diagnostic.fixInfo = { marker: 'lazy opaque' };
    diagnostic.fixInfo.circular = diagnostic.fixInfo;
    diagnostics.set(document.uri, [diagnostic]);
  };
  for (const document of vscode.workspace.textDocuments) refresh(document);
  context.subscriptions.push(diagnostics, vscode.workspace.onDidOpenTextDocument(refresh),
    vscode.workspace.onDidChangeTextDocument(event => refresh(event.document)));
  const provider = {
    marker: 'lazy receiver',
    provideCodeActions(document, range, actionContext) {
      const original = diagnostics.get(document.uri)[0];
      if (!original || !actionContext.diagnostics.includes(original)) throw new Error('Lazy original Diagnostic identity lost');
      const action = new vscode.CodeAction('Lazy original object fix', vscode.CodeActionKind.QuickFix);
      action.data = { document, range, diagnostic: original }; action.data.self = action.data;
      action.diagnostics = [original]; action.customProperty = { marker: 42 };
      originals.set(action, action.data);
      fs.writeFileSync(path.join(vscode.workspace.rootPath, 'lazy-provided.json'), JSON.stringify({ original: true }));
      return [action];
    },
    async resolveCodeAction(action, token) {
      fs.writeFileSync(path.join(vscode.workspace.rootPath, 'lazy-resolving.json'), JSON.stringify({ started: true }));
      const deadline = Date.now() + 4000;
      while (fs.existsSync(path.join(vscode.workspace.rootPath, 'hold-resolve')) && Date.now() < deadline && !token.isCancellationRequested) await new Promise(resolve => setTimeout(resolve, 5));
      if (this.marker !== 'lazy receiver' || originals.get(action) !== action.data || action.data.self !== action.data
          || action.customProperty.marker !== 42 || action.diagnostics[0] !== action.data.diagnostic
          || action.data.diagnostic.fixInfo.circular !== action.data.diagnostic.fixInfo) throw new Error('Lazy original action/diagnostic/custom identity lost');
      action.edit = new vscode.WorkspaceEdit();
      action.edit.replace(action.data.document.uri, action.data.range, 'resolved');
      fs.writeFileSync(path.join(vscode.workspace.rootPath, 'lazy-resolved.json'), JSON.stringify({ original: true, cancelled: token.isCancellationRequested }));
      return action;
    },
  };
  context.subscriptions.push(vscode.languages.registerCodeActionsProvider('markdown', provider,
    { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] }));
};
