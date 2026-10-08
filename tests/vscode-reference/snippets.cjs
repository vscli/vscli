'use strict';
const assert = require('node:assert/strict');
const cases = require('./snippet-cases.json');

// All coordinates are Unicode scalar offsets so the native rope can compare
// them directly. VS Code's document.offsetAt uses UTF-16 code units.
function observe(editor) {
  const text = editor.document.getText();
  const offset = position => [...text.slice(0, editor.document.offsetAt(position))].length;
  return { text, selections: editor.selections.map(s => ({ anchor: offset(s.anchor), cursor: offset(s.active) })) };
}

async function snippetTrace(vscode) {
  const traces = [];
  for (const fixture of cases) {
    const document = await vscode.workspace.openTextDocument({ content: '', language: 'plaintext' });
    const editor = await vscode.window.showTextDocument(document, { preview: false });
    editor.options = { tabSize: 4, insertSpaces: true };
    const inserted = await editor.insertSnippet(new vscode.SnippetString(fixture.body));
    assert.equal(inserted, true, `Snippet insertion failed: ${fixture.name}`);
    const observations = [{ action: 'insert', ...observe(editor) }];
    for (const step of fixture.steps) {
      if (Object.hasOwn(step, 'type')) {
        await vscode.commands.executeCommand('type', { text: step.type });
      } else {
        await vscode.commands.executeCommand(step.command);
      }
      observations.push({ action: step.command || 'type', ...observe(editor) });
    }
    traces.push({ name: fixture.name, body: fixture.body, observations });
    await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
  }
  return traces;
}

module.exports = { snippetTrace };
