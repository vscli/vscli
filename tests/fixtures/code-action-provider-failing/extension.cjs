'use strict';
const vscode = require('vscode');
exports.activate = context => context.subscriptions.push(vscode.languages.registerCodeActionsProvider(['markdown', 'cpp'], {
  provideCodeActions() { throw new Error('Isolated qualification provider failure'); },
}, { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] }));
