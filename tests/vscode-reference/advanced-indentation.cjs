'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { parse } = require('jsonc-parser');
const { prepareLanguages } = require('./language-readiness.cjs');
const fixtures = require('./advanced-indentation-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const keys = ['autoIndent', 'tabSize', 'insertSpaces', 'detectIndentation',
  'autoClosingBrackets', 'autoClosingQuotes', 'autoClosingOvertype', 'autoClosingDelete'];

function snapshot(vscode, editor) {
  const text = editor.document.getText();
  const scalar = position => [...text.slice(0, editor.document.offsetAt(position))].length;
  return { text, selections: editor.selections.map(selection => ({
    anchor: scalar(selection.anchor), cursor: scalar(selection.active),
  })), version: editor.document.version,
  eol: editor.document.eol === vscode.EndOfLine.CRLF ? 'CRLF' : 'LF' };
}
function effective(vscode, document) {
  const configuration = vscode.workspace.getConfiguration('editor', document);
  return Object.fromEntries(keys.map(key => [key, configuration.get(key)]));
}
async function prepareTokens(vscode, editor) {
  const before = snapshot(vscode, editor);
  await vscode.commands.executeCommand('editor.action.forceRetokenize');
  const after = snapshot(vscode, editor);
  assert.deepEqual(after, before, 'Token preparation changed bytes/version/selections/EOL');
  return { command: 'editor.action.forceRetokenize', before, after };
}
async function close(vscode) {
  await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
}
async function show(vscode, text, language, mode) {
  const document = await vscode.workspace.openTextDocument({ content: text, language });
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  assert.equal(effective(vscode, document).autoIndent, mode);
  return editor;
}
async function enterWitness(vscode, mode, label) {
  const editor = await show(vscode, '    value\r\n', 'plaintext', mode);
  try {
    const position = editor.document.positionAt(9);
    editor.selection = new vscode.Selection(position, position);
    const before = snapshot(vscode, editor);
    await vscode.commands.executeCommand('type', { text: '\n' });
    const after = snapshot(vscode, editor);
    // This independent plain-text gesture distinguishes None from indentation
    // retention. Exact mode additionally comes from immutable startup settings.
    assert.equal(after.text, '    value\r\n' + (mode === 'none' ? '' : '    ') + '\r\n',
      `Independent Enter mode witness failed: ${mode}/${label}`);
    return { label, effective: effective(vscode, editor.document), before, after };
  } finally { await close(vscode); }
}
function inventory(vscode, language) {
  const configurations = [];
  for (const extension of vscode.extensions.all) {
    for (const contribution of extension.packageJSON.contributes?.languages || []) {
      if (contribution.id !== language || !contribution.configuration) continue;
      const bytes = fs.readFileSync(path.join(extension.extensionPath, contribution.configuration));
      const errors = [], configuration = parse(bytes.toString(), errors);
      assert.deepEqual(errors, []);
      configurations.push({ extension: extension.id, version: extension.packageJSON.version,
        path: contribution.configuration, sha256: hash(bytes), configuration });
    }
  }
  assert.ok(configurations.length, `No installed reference configuration: ${language}`);
  return configurations;
}
async function grammarWitness(vscode, language, configurations, mode) {
  const opening = language === 'cpp' ? "'" : '"';
  assert.ok(configurations.some(({ configuration }) => configuration.comments?.lineComment === '//'
    && configuration.autoClosingPairs.some(pair => pair.open === opening && pair.close === opening
      && pair.notIn?.includes('comment'))));
  const original = 'vscli_probe \r\n    \r\n// ';
  const editor = await show(vscode, original, language, mode);
  const deadline = Date.now() + 10000;
  try {
    for (let attempts = 1; ; attempts++) {
      assert.equal(editor.document.getText(), original);
      let position = editor.document.positionAt(original.indexOf('\r\n//'));
      editor.selection = new vscode.Selection(position, position);
      const positivePreparation = await prepareTokens(vscode, editor);
      await vscode.commands.executeCommand('type', { text: opening });
      const positive = snapshot(vscode, editor);
      if (positive.text === original.replace('    \r\n', `    ${opening}${opening}\r\n`)) {
        position = editor.document.positionAt(editor.document.getText().length);
        editor.selection = new vscode.Selection(position, position);
        // A positive edit invalidates later token states. Prepare again so a
        // skipped not-cheap auto-closing operation cannot masquerade as lexical
        // suppression in the comment readiness witness.
        const negativePreparation = await prepareTokens(vscode, editor);
        await vscode.commands.executeCommand('type', { text: opening });
        const negative = snapshot(vscode, editor);
        if (negative.text === positive.text + opening) return { language, attempts, positive, negative,
          positivePreparation, negativePreparation,
          witness: 'Positive code quote pair followed by notIn(comment) suppression; independent of indentation' };
        await vscode.commands.executeCommand('undo');
        assert.equal(editor.document.getText(), positive.text);
      }
      await vscode.commands.executeCommand('undo');
      assert.equal(editor.document.getText(), original);
      assert.ok(Date.now() < deadline, `Independent lexical readiness timed out: ${language}`);
      await delay(20);
    }
  } finally { await close(vscode); }
}

async function advancedIndentation(vscode, mode) {
  assert.ok(['none', 'keep', 'brackets', 'advanced', 'full'].includes(mode));
  const selected = fixtures.filter(fixture => fixture.settings.autoIndent === mode);
  const configurationInventory = Object.fromEntries(['cpp', 'json'].map(language => [language, inventory(vscode, language)]));
  const initialConfiguration = {};
  for (const language of ['cpp', 'json']) {
    const configuration = vscode.workspace.getConfiguration('editor', { languageId: language });
    initialConfiguration[language] = { global: configuration.inspect('autoIndent').globalValue,
      language: configuration.inspect('autoIndent').globalLanguageValue, effective: configuration.get('autoIndent') };
    assert.deepEqual(initialConfiguration[language], { global: mode, language: mode, effective: mode });
  }
  assert.ok((await vscode.commands.getCommands(true)).includes('editor.action.forceRetokenize'));
  await prepareLanguages(vscode, selected);
  const grammarReadiness = [];
  for (const [language, configurations] of Object.entries(configurationInventory)) {
    grammarReadiness.push(await grammarWitness(vscode, language, configurations, mode));
  }
  const traces = [];
  for (const fixture of selected) {
    const modeWitness = await enterWitness(vscode, mode, fixture.name);
    const globalConfiguration = vscode.workspace.getConfiguration();
    const overrideKey = `[${fixture.language}]`;
    const previousOverride = globalConfiguration.inspect(overrideKey)?.globalValue;
    const settings = { ...fixture.settings, detectIndentation: false, autoClosingBrackets: 'languageDefined',
      autoClosingQuotes: 'languageDefined', autoClosingOvertype: 'auto', autoClosingDelete: 'auto' };
    const override = { ...previousOverride, ...Object.fromEntries(Object.entries(settings).map(([key, value]) => [`editor.${key}`, value])) };
    let editor;
    try {
      if (JSON.stringify(previousOverride) !== JSON.stringify(override)) {
        await globalConfiguration.update(overrideKey, override, vscode.ConfigurationTarget.Global);
      }
      editor = await show(vscode, fixture.text, fixture.language, mode);
      editor.options = { tabSize: settings.tabSize, insertSpaces: settings.insertSpaces };
      const effectiveConfiguration = effective(vscode, editor.document);
      assert.deepEqual(effectiveConfiguration, settings, `Actual document settings: ${fixture.name}`);
      const position = scalar => editor.document.positionAt([...editor.document.getText()].slice(0, scalar).join('').length);
      editor.selections = fixture.selections.map(selection => new vscode.Selection(position(selection.anchor), position(selection.cursor)));
      const observations = [{ action: 'initial', ...snapshot(vscode, editor) }], preparation = [];
      for (const [index, step] of fixture.steps.entries()) {
        if (Object.hasOwn(step, 'type') || step.command === 'lineBreakInsert') {
          preparation.push({ step: index, ...await prepareTokens(vscode, editor) });
        }
        if (Object.hasOwn(step, 'type')) await vscode.commands.executeCommand('type', { text: step.type });
        else await vscode.commands.executeCommand(step.command);
        observations.push({ action: step.command || 'type', input: step.type, ...snapshot(vscode, editor) });
      }
      traces.push({ name: fixture.name, language: fixture.language, settings, effectiveConfiguration,
        editorOptions: { tabSize: editor.options.tabSize, insertSpaces: editor.options.insertSpaces },
        modeWitness, preparation, observations });
      console.log(`Observed prepared ${mode} ${traces.length}/${selected.length} ${fixture.name}`);
    } finally {
      if (editor) await close(vscode);
      if (JSON.stringify(globalConfiguration.inspect(overrideKey)?.globalValue) !== JSON.stringify(previousOverride)) {
        await globalConfiguration.update(overrideKey, previousOverride, vscode.ConfigurationTarget.Global);
      }
      assert.deepEqual(globalConfiguration.inspect(overrideKey)?.globalValue, previousOverride);
    }
  }
  return { mode, initialConfiguration, configurationInventory, grammarReadiness, traces };
}
module.exports = { advancedIndentation };
