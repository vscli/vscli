'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vscode = require('vscode');
const { parse } = require('jsonc-parser');
const { configurationTrace } = require('./contracts.cjs');
const { snippetTrace } = require('./snippets.cjs');

async function run() {
  assert.equal(vscode.version, '1.95.0', 'Reference version must remain pinned');
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  assert.ok(output, 'Missing VSCLI_REFERENCE_OUTPUT');
  const document = await vscode.workspace.openTextDocument(vscode.Uri.parse('vscode://defaultsettings/keybindings.json'));
  const content = document.getText();
  const errors = [];
  const bindings = parse(content, errors, { allowTrailingComma: true });
  assert.deepEqual(errors, []);
  assert.ok(Array.isArray(bindings) && bindings.length > 500, 'Default binding inventory is unexpectedly small');
  fs.writeFileSync(path.join(output, 'keybindings.jsonc'), content);
  await vscode.commands.executeCommand('workbench.action.inspectKeyMappingsJSON');
  const keyboard = JSON.parse(vscode.window.activeTextEditor.document.getText());
  assert.ok(keyboard.layout && keyboard.rawMapping, 'Missing observed keyboard layout');
  fs.writeFileSync(path.join(output, 'keyboard.json'), JSON.stringify(keyboard, null, 2) + '\n');
  const product = JSON.parse(fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'), 'utf8'));
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95', 'Reference source commit must remain pinned');
  const extensions = vscode.extensions.all.filter(extension => extension.id !== 'vscli-test.vscli-reference-harness')
    .map(extension => ({ id: extension.id, version: extension.packageJSON.version, license: extension.packageJSON.license || null }))
    .sort((a, b) => a.id.localeCompare(b.id));
  const report = { schema: 1, version: vscode.version, commit: product.commit || null,
    platform: process.platform, arch: process.arch, language: vscode.env.language,
    keyboardLayout: keyboard.layout, extensions, bindings };
  fs.writeFileSync(path.join(output, 'inventory.json'), JSON.stringify(report, null, 2) + '\n');
  const resource = vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri, 'fixture.js');
  const trace = await configurationTrace(vscode, resource, async (layer, key, value) => {
    // Install before writing: the change event can precede update() resolution.
    let subscription, timer;
    const event = new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`Timed out waiting for ${key}`)), 30000);
      subscription = vscode.workspace.onDidChangeConfiguration(change => {
        if (change.affectsConfiguration(key)) resolve();
      });
    });
    try {
      await Promise.all([event, vscode.workspace.getConfiguration().update(key, value,
        layer === 0 ? vscode.ConfigurationTarget.Global : vscode.ConfigurationTarget.Workspace)]);
    } finally { clearTimeout(timer); subscription.dispose(); }
  });
  fs.writeFileSync(path.join(output, 'configuration.json'), JSON.stringify(trace, null, 2) + '\n');
  const snippets = await snippetTrace(vscode);
  fs.writeFileSync(path.join(output, 'snippets.json'), JSON.stringify(snippets, null, 2) + '\n');
  console.log(`Exported ${bindings.length} default rules and ${trace.length} configuration observations`);
}

exports.run = async () => {
  let timer;
  try {
    await Promise.race([run(), new Promise((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Reference suite exceeded 120 seconds')), 120000);
    })]);
  } finally { clearTimeout(timer); }
};
