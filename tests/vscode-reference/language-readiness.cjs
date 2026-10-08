'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { parse } = require('jsonc-parser');

// Showing a document does not await the workbench's asynchronous language
// configuration loader. Probe a different feature before observing snippets;
// never retry a snippet result until it matches our native implementation.
async function waitForComments(vscode, language, comments, timeoutMs = 10000) {
  const token = comments.lineComment || comments.blockComment?.[0];
  if (!token) return;
  const document = await vscode.workspace.openTextDocument({ content: 'vscli_probe', language });
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const deadline = Date.now() + timeoutMs;
  try {
    for (;;) {
      const changed = await editor.edit(edit => edit.replace(
        new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), 'vscli_probe'));
      assert.equal(changed, true, `Could not reset language readiness probe: ${language}`);
      editor.selection = new vscode.Selection(document.positionAt(0), document.positionAt(0));
      await vscode.commands.executeCommand(comments.lineComment
        ? 'editor.action.addCommentLine' : 'editor.action.blockComment');
      if (document.getText().includes(token)) return;
      assert.ok(Date.now() < deadline, `Language comments did not become ready: ${language}`);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
  } finally {
    await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
  }
}

async function prepareLanguages(vscode, fixtures) {
  for (const language of new Set(fixtures.map(fixture => fixture.language).filter(Boolean))) {
    // Read the reference's own declarative data; do not install configurations
    // or use VSCLI's token table to manufacture expected snippet output.
    for (const extension of vscode.extensions.all) {
      for (const contribution of extension.packageJSON.contributes?.languages || []) {
        if (contribution.id !== language || !contribution.configuration) continue;
        const errors = [];
        const configuration = parse(fs.readFileSync(path.join(extension.extensionPath, contribution.configuration), 'utf8'), errors);
        assert.deepEqual(errors, [], `Invalid reference language configuration: ${language}`);
        await waitForComments(vscode, language, configuration.comments || {});
      }
    }
  }
}

module.exports = { prepareLanguages, waitForComments };
