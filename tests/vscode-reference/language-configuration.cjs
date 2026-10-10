'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { parse } = require('jsonc-parser');
const corpus = require('./language-configuration-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

function snapshot(vscode, editor) {
  const text = editor.document.getText();
  const scalar = position => [...text.slice(0, editor.document.offsetAt(position))].length;
  return { text, selections: editor.selections.map(selection => ({ anchor: scalar(selection.anchor),
    cursor: scalar(selection.active) })), version: editor.document.version,
    eol: editor.document.eol === vscode.EndOfLine.CRLF ? 'CRLF' : 'LF' };
}
async function show(vscode, text, language) {
  const document = await vscode.workspace.openTextDocument({ content: text, language });
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  assert.equal(document.languageId, language);
  editor.options = { tabSize: 4, insertSpaces: true };
  return editor;
}
async function close(vscode) {
  await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
}
async function prepare(vscode, editor) {
  const before = snapshot(vscode, editor);
  await vscode.commands.executeCommand('editor.action.forceRetokenize');
  const after = snapshot(vscode, editor);
  assert.deepEqual(after, before, 'Token preparation changed document bytes/version/selections/EOL');
  return { before, after };
}
async function declarationReadiness(vscode, language) {
  const original = 'VSCLI_CONFIG_READY\r\n';
  const editor = await show(vscode, original, language);
  const deadline = Date.now() + 10000;
  try {
    for (let attempts = 1; ; attempts++) {
      assert.equal(editor.document.getText(), original);
      const position = editor.document.positionAt('VSCLI_CONFIG_READY'.length);
      editor.selection = new vscode.Selection(position, position);
      const preparation = await prepare(vscode, editor);
      await vscode.commands.executeCommand('type', { text: '\n' });
      const observed = snapshot(vscode, editor);
      if (observed.text === 'VSCLI_CONFIG_READY\r\n!\r\n') return { attempts, preparation, observed,
        scope: 'Fixture-only onEnterRules sentinel proves installed JSON declaration loaded; excluded from native comparison; no native regex API qualification' };
      await vscode.commands.executeCommand('undo');
      assert.equal(editor.document.getText(), original);
      assert.ok(Date.now() < deadline, 'Installed declarative configuration did not become ready');
      await delay(20);
    }
  } finally { await close(vscode); }
}
async function lexicalReadiness(vscode, language, opening, closing) {
  const original = `${opening} /* ${closing} */ ${closing}`;
  const editor = await show(vscode, original, language);
  const deadline = Date.now() + 10000;
  try {
    for (let attempts = 1; ; attempts++) {
      const position = editor.document.positionAt(0);
      editor.selection = new vscode.Selection(position, position);
      const preparation = await prepare(vscode, editor);
      await vscode.commands.executeCommand('editor.action.jumpToBracket');
      const observed = snapshot(vscode, editor);
      assert.equal(observed.text, original);
      if (observed.selections[0].cursor === 10) return { attempts, opening, closing,
        preparation, observed,
        scope: 'Prepared public bracket jump skips a block-comment closer; independent of the autoClosingPairs.notIn behavior under observation' };
      assert.ok(Date.now() < deadline, `Independent lexical bracket readiness timed out: ${JSON.stringify(observed)}`);
      await delay(20);
    }
  } finally { await close(vscode); }
}
function installedInventory(vscode, language) {
  const inventory = [];
  for (const extension of vscode.extensions.all) for (const contribution of extension.packageJSON.contributes?.languages || []) {
    if (contribution.id !== language || !contribution.configuration) continue;
    const bytes = fs.readFileSync(path.join(extension.extensionPath, contribution.configuration));
    const errors = [], configuration = parse(bytes.toString(), errors);
    assert.deepEqual(errors, []);
    inventory.push({ extension: extension.id, version: extension.packageJSON.version,
      license: extension.packageJSON.license || null, path: contribution.configuration,
      sha256: hash(bytes), configuration });
  }
  return inventory;
}

async function languageConfigurationTrace(vscode, variantId) {
  const variant = corpus.variants.find(candidate => candidate.id === variantId);
  assert.ok(variant);
  const declaration = await declarationReadiness(vscode, variant.language);
  const inventory = installedInventory(vscode, variant.language);
  assert.ok(inventory.some(entry => entry.extension === `vscli-test.language-configuration-${variantId}`));
  let lexical;
  if (variant.language === 'cpp') {
    lexical = await lexicalReadiness(vscode, variant.language,
      variantId === 'valid-mixed' ? '«' : '{', variantId === 'valid-mixed' ? '»' : '}');
  }
  const traces = [];
  for (const fixture of corpus.cases.filter(candidate => candidate.variant === variantId)) {
    const editor = await show(vscode, fixture.text, variant.language);
    try {
      const effective = vscode.workspace.getConfiguration('editor', editor.document);
      const settings = Object.fromEntries(['autoIndent', 'autoClosingBrackets', 'autoClosingQuotes', 'autoSurround',
        'tabSize', 'insertSpaces', 'detectIndentation'].map(key => [key, effective.get(key)]));
      assert.deepEqual(settings, { autoIndent: 'full', autoClosingBrackets: 'languageDefined',
        autoClosingQuotes: 'languageDefined', autoSurround: 'languageDefined', tabSize: 4,
        insertSpaces: true, detectIndentation: false });
      const position = scalar => editor.document.positionAt([...editor.document.getText()].slice(0, scalar).join('').length);
      editor.selections = fixture.selections.map(selection => new vscode.Selection(position(selection.anchor), position(selection.cursor)));
      const observations = [{ action: 'initial', ...snapshot(vscode, editor) }], preparation = [];
      for (const [index, step] of fixture.steps.entries()) {
        if (Object.hasOwn(step, 'type')) preparation.push({ step: index, ...await prepare(vscode, editor) });
        if (Object.hasOwn(step, 'type')) await vscode.commands.executeCommand('type', { text: step.type });
        else await vscode.commands.executeCommand(step.command);
        observations.push({ action: step.command || 'type', ...snapshot(vscode, editor) });
      }
      traces.push({ name: fixture.name, variant: variantId, language: variant.language,
        settings, preparation, observations });
      console.log(`Observed declaration ${fixture.name}: ${JSON.stringify(observations[1])}`);
    } finally { await close(vscode); }
  }
  return { variant: variantId, language: variant.language, declaration, lexical, inventory, traces };
}
module.exports = { languageConfigurationTrace };
