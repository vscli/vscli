'use strict';
const assert = require('node:assert/strict');
const { prepareLanguages } = require('./language-readiness.cjs');
const fixtures = require('./typing-cases.json');

function observe(editor) {
  const text = editor.document.getText();
  const offset = position => [...text.slice(0, editor.document.offsetAt(position))].length;
  return { text, selections: editor.selections.map(selection => ({
    anchor: offset(selection.anchor), cursor: offset(selection.active),
  })) };
}

async function typingTrace(vscode) {
  // Readiness probes another feature using the reference's own configuration.
  // No gesture is retried until it agrees with the native implementation.
  await prepareLanguages(vscode, fixtures);
  const traces = [];
  for (const fixture of fixtures) {
    const document = await vscode.workspace.openTextDocument({
      content: fixture.text, language: fixture.language,
    });
    const editor = await vscode.window.showTextDocument(document, { preview: false });
    editor.options = { tabSize: 4, insertSpaces: true };
    const position = scalar => document.positionAt([...document.getText()].slice(0, scalar).join('').length);
    editor.selections = (fixture.selections || [{ anchor: 0, cursor: 0 }])
      .map(selection => new vscode.Selection(position(selection.anchor), position(selection.cursor)));
    const observations = [{ action: 'initial', ...observe(editor) }];
    for (const step of fixture.steps) {
      if (Object.hasOwn(step, 'type')) {
        assert.equal(typeof step.type, 'string');
        await vscode.commands.executeCommand('type', { text: step.type });
      } else {
        await vscode.commands.executeCommand(step.command);
      }
      observations.push({ action: step.command || 'type', ...observe(editor) });
    }
    traces.push({ name: fixture.name, observations });
    await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
  }
  return traces;
}

module.exports = { typingTrace };
