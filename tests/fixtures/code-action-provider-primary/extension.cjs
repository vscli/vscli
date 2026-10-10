'use strict';
const vscode = require('vscode');
const publishEvidence = require('./publish-evidence.cjs');
const path = require('node:path');
exports.activate = context => {
  const diagnostics = vscode.languages.createDiagnosticCollection('primary');
  const refresh = document => {
    if (!['markdown', 'cpp'].includes(document.languageId)) return;
    const text = document.lineAt(0).text, offset = text.indexOf('bad');
    if (offset < 0) { diagnostics.delete(document.uri); return; }
    const diagnostic = new vscode.Diagnostic(new vscode.Range(0, offset, 0, offset + 3), 'primary original diagnostic', vscode.DiagnosticSeverity.Warning);
    diagnostic.code = 'primary-fix';
    diagnostic.fixInfo = { marker: 'opaque original' };
    diagnostic.fixInfo.self = diagnostic.fixInfo;
    diagnostics.set(document.uri, [diagnostic]);
  };
  for (const document of vscode.workspace.textDocuments) refresh(document);
  context.subscriptions.push(diagnostics, vscode.workspace.onDidOpenTextDocument(refresh),
    vscode.workspace.onDidChangeTextDocument(event => refresh(event.document)));
  const provider = {
    marker: 'original receiver',
    provideCodeActions(document, range, actionContext, token) {
      if (this.marker !== 'original receiver' || token.isCancellationRequested) throw new Error('Provider receiver/token lost');
      const owned = diagnostics.get(document.uri).filter(d => d.range.intersection(range));
      for (const diagnostic of owned) {
        if (!actionContext.diagnostics.includes(diagnostic) || diagnostic.fixInfo.self !== diagnostic.fixInfo) throw new Error('Original Diagnostic/custom fixInfo identity lost');
      }
      publishEvidence(path.join(vscode.workspace.rootPath, 'primary-observed.json'), JSON.stringify({
        diagnostics: owned.length, triggerKind: actionContext.triggerKind, only: actionContext.only?.value,
        selection: range instanceof vscode.Selection,
      }));
      const direct = new vscode.CodeAction('Primary Unicode fix', vscode.CodeActionKind.QuickFix);
      direct.edit = new vscode.WorkspaceEdit();
      direct.edit.replace(document.uri, range, 'fixed\nsecond');
      direct.diagnostics = owned;
      direct.isPreferred = true;
      const multiple = new vscode.CodeAction('Multiple mirrored buffers', vscode.CodeActionKind.QuickFix);
      multiple.edit = new vscode.WorkspaceEdit();
      for (const target of vscode.workspace.textDocuments) {
        if (['markdown', 'cpp', 'plaintext'].includes(target.languageId)) multiple.edit.replace(target.uri, new vscode.Range(0, 0, 0, target.lineAt(0).text.length), 'both');
      }
      const invalid = new vscode.CodeAction('Atomic invalid range', vscode.CodeActionKind.QuickFix);
      invalid.edit = new vscode.WorkspaceEdit();
      invalid.edit.replace(document.uri, range, 'MUST NOT APPLY');
      const other = vscode.workspace.textDocuments.find(target => target !== document) || document;
      invalid.edit.replace(other.uri, new vscode.Range(999, 0, 999, 1), 'INVALID');
      const command = new vscode.CodeAction('Command must remain disabled', vscode.CodeActionKind.QuickFix);
      command.command = { command: 'qualification.mustNotExecute', title: 'Unsafe shortcut' };
      const combined = new vscode.CodeAction('Combined edit and command', vscode.CodeActionKind.QuickFix);
      combined.edit = direct.edit;
      combined.command = command.command;
      const refactor = new vscode.CodeAction('Refactor only', vscode.CodeActionKind.Refactor);
      refactor.edit = new vscode.WorkspaceEdit();
      refactor.edit.replace(document.uri, range, 'refactored');
      return [direct, multiple, invalid, command, combined, refactor];
    },
  };
  context.subscriptions.push(vscode.languages.registerCodeActionsProvider(['markdown', 'cpp'], provider,
    { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix, vscode.CodeActionKind.Refactor] }));
  context.subscriptions.push(vscode.commands.registerCommand('qualification.mustNotExecute', () => {
    publishEvidence(path.join(vscode.workspace.rootPath, 'COMMAND_EXECUTED'), 'bad');
  }));
  context.subscriptions.push(vscode.commands.registerCommand('qualification.openHidden', async () => {
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(vscode.workspace.rootPath, 'hidden.md')));
    publishEvidence(path.join(vscode.workspace.rootPath, 'hidden-opened.json'), JSON.stringify({ id: document._snapshot.id }));
  }));
};
