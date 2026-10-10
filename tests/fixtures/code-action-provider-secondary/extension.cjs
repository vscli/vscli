'use strict';
const vscode = require('vscode');
exports.activate = context => context.subscriptions.push(vscode.languages.registerCodeActionsProvider(['markdown', 'cpp'], {
  provideCodeActions(document, range) {
    const action = new vscode.CodeAction('Secondary independent fix', vscode.CodeActionKind.QuickFix);
    action.edit = new vscode.WorkspaceEdit();
    action.edit.replace(document.uri, range, 'secondary');
    return [action];
  },
}, { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] }));
