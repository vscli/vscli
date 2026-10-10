'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const vscode = require('vscode');
const { parse } = require('jsonc-parser');
const { configurationTrace } = require('./contracts.cjs');
const { snippetTrace } = require('./snippets.cjs');
const { diagnosticTrace } = require('./diagnostics.cjs');
const { typingTrace } = require('./typing.cjs');

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
  const insertion = await snippetTrace(vscode, require('./snippet-insertion-cases.json'));
  fs.writeFileSync(path.join(output, 'snippet-insertion.json'), JSON.stringify(insertion, null, 2) + '\n');
  const variables = await snippetTrace(vscode, require('./snippet-variable-cases.json'));
  fs.writeFileSync(path.join(output, 'snippet-variables.json'), JSON.stringify(variables, null, 2) + '\n');
  const diagnostics = await diagnosticTrace(vscode);
  fs.writeFileSync(path.join(output, 'diagnostics.json'), JSON.stringify(diagnostics, null, 2) + '\n');
  fs.writeFileSync(path.join(output, 'diagnostics-provenance.json'), JSON.stringify({
    schema: 1, observedAt: new Date().toISOString(), reference: { version: vscode.version,
      commit: product.commit, platform: process.platform, arch: process.arch, node: process.versions.node },
    observerSha256: createHash('sha256').update(fs.readFileSync(require.resolve('./diagnostics.cjs'))).digest('hex'),
    traceSha256: createHash('sha256').update(fs.readFileSync(path.join(output, 'diagnostics.json'))).digest('hex'),
    observations: diagnostics.snapshots.length, events: diagnostics.events.length,
    limitations: ['Collection operations and document edit/close retention are observed in the actual pinned executable.',
      'The optional-host comparison separately excludes document event adapters and main-thread marker mirror events.'],
  }, null, 2) + '\n');
  const typing = await typingTrace(vscode);
  fs.writeFileSync(path.join(output, 'typing.json'), JSON.stringify(typing, null, 2) + '\n');
  fs.writeFileSync(path.join(output, 'typing-provenance.json'), JSON.stringify({
    schema: 1, observedAt: new Date().toISOString(),
    reference: { version: vscode.version, commit: product.commit,
      platform: process.platform, arch: process.arch, node: process.versions.node },
    observerSha256: createHash('sha256').update(fs.readFileSync(require.resolve('./typing.cjs'))).digest('hex'),
    casesSha256: createHash('sha256').update(fs.readFileSync(require.resolve('./typing-cases.json'))).digest('hex'),
    traceSha256: createHash('sha256').update(fs.readFileSync(path.join(output, 'typing.json'))).digest('hex'),
    observations: typing.length,
    limitations: ['Native comparisons cover the named C++/JSON gestures and scalar selections only.',
      'These commands do not qualify physical keyboard delivery or all language indentation rules.'],
  }, null, 2) + '\n');
  console.log(`Exported ${bindings.length} default rules, ${trace.length} configuration observations and ${snippets.length} snippet session traces and ${insertion.length} insertion traces and ${variables.length} variable/cancellation traces`);
}

exports.run = async () => {
  let timer;
  try {
    await Promise.race([run(), new Promise((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Reference suite exceeded 120 seconds')), 120000);
    })]);
  } finally { clearTimeout(timer); }
};
