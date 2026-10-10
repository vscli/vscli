'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const cases = require('./save-code-actions-cases.json');
const { text } = require('./save-code-actions.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();
async function main() {
  const [output, session] = process.argv.slice(2);
  fs.mkdirSync(output, { recursive: true });
  assert.equal(cases.length, 14, 'Named code-action save qualification budget');
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0', cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  for (const fixture of cases) {
    assert.match(fixture.name, /^[a-z0-9-]+$/);
    const root = path.join(session, fixture.name), profile = path.join(root, 'p'), workspace = path.join(root, 'w'), extensions = path.join(root, 'e');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(path.join(workspace, '.vscode'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true });
    fs.writeFileSync(path.join(workspace, 'main.txt'), text);
    const userSettings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false, 'security.workspace.trust.enabled': false,
      'workbench.startupEditor': 'none', 'workbench.editor.enablePreview': false,
      'editor.parameterHints.enabled': false, 'editor.quickSuggestions': false,
      'editor.formatOnSave': false, 'editor.formatOnPaste': false, 'editor.formatOnType': false,
      'editor.codeActions.triggerOnFocusChange': false, 'editor.codeActionsOnSave': {},
      'editor.detectIndentation': false, 'editor.tabSize': 2, 'editor.insertSpaces': true,
      'files.autoSave': fixture.autosave, 'files.autoSaveDelay': 500,
      'files.trimTrailingWhitespace': false, 'files.insertFinalNewline': false,
      'files.trimFinalNewlines': false, ...fixture.user });
    const workspaceSettings = JSON.stringify(fixture.workspace);
    fs.writeFileSync(path.join(profile, 'User/settings.json'), userSettings);
    fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), workspaceSettings);
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'save-code-actions-suite.cjs'), extensionTestsEnv: {
        VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_SAVE_CODE_ACTIONS_CASE: fixture.name,
        VSCLI_REFERENCE_SAVE_CODE_ACTIONS_WORKSPACE: workspace,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(userSettings),
        VSCLI_REFERENCE_WORKSPACE_SETTINGS_SHA256: hash(workspaceSettings) },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-evidence.json`))));
  const projection = evidence.map(raw => ({ name: raw.name, effective: raw.setup.effective,
    effectiveKeys: raw.setup.effectiveKeys, observations: raw.observations,
    callbacks: raw.callbacks.filter(value => value.phase === 'target').map(({ only, triggerKind, text, returned }) => ({ only, triggerKind, text, returned })) }));
  const write = (name, value) => {
    const bytes = JSON.stringify(value, null, 2) + '\n'; fs.writeFileSync(path.join(output, name), bytes); return hash(bytes);
  };
  const traceSha256 = write('save-code-actions.json', projection), evidenceSha256 = write('save-code-actions-evidence.json', evidence);
  write('save-code-actions-provenance.json', { version: '1.95.0', commit: '912bb683695358a54ae0c670461738984cbb5b95',
    platform: process.platform, architecture: process.arch, caseCount: cases.length,
    snapshotCount: projection.reduce((count, row) => count + row.observations.length, 0), traceSha256, evidenceSha256,
    casesSha256: hash(fs.readFileSync(path.join(__dirname, 'save-code-actions-cases.json'))),
    runs: cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-provenance.json`)))) });
  console.log(`Observed ${cases.length} named synthetic source-action save fixtures`);
}
main().then(() => process.send({ type: 'complete', ok: true }), error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
