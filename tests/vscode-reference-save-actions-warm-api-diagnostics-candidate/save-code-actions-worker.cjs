'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { workerLifetime } = require('./supervisor.cjs');
const { sourceIdentity, loadCases } = require('./save-configuration-ready.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();
function launcherIdentity(executable) {
  const descriptor = fs.openSync(executable, 'r');
  try {
    const stat = fs.fstatSync(descriptor);
    assert.ok(stat.isFile() && stat.size > 0 && stat.size <= 512 * 1024 * 1024, 'Selected launcher exceeds bounds');
    assert.ok(Buffer.byteLength(executable) <= 4096, 'Selected launcher path exceeds bounds');
    const digest = createHash('sha256'), block = Buffer.alloc(256 * 1024);
    let offset = 0;
    while (offset < stat.size) {
      const count = fs.readSync(descriptor, block, 0, Math.min(block.length, stat.size - offset), offset);
      assert.ok(count > 0, 'Selected launcher changed while hashing');
      digest.update(block.subarray(0, count)); offset += count;
    }
    const after = fs.fstatSync(descriptor);
    assert.equal(after.size, stat.size, 'Selected launcher changed while hashing');
    assert.equal(after.mtimeMs, stat.mtimeMs, 'Selected launcher changed while hashing');
    return { path: executable, bytes: stat.size, sha256: digest.digest('hex') };
  } finally { fs.closeSync(descriptor); }
}
async function main() {
  const [output, session] = process.argv.slice(2);
  sourceIdentity(__dirname);
  const cases = loadCases();
  const { text, readinessText } = require('./save-code-actions.cjs');
  const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
  fs.mkdirSync(output, { recursive: true });
  assert.equal(cases.length, 14, 'Named code-action save qualification budget');
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0', cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const launcher = launcherIdentity(executable);
  for (const fixture of cases) {
    assert.match(fixture.name, /^[a-z0-9-]+$/);
    const root = path.join(session, fixture.name), profile = path.join(root, 'p'), workspace = path.join(root, 'w'), extensions = path.join(root, 'e');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(path.join(workspace, '.vscode'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true });
    fs.writeFileSync(path.join(workspace, 'main.txt'), text);
    fs.writeFileSync(path.join(workspace, 'readiness.ini'), readinessText);
    const userSettings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false, 'security.workspace.trust.enabled': false,
      'workbench.startupEditor': 'none', 'workbench.editor.enablePreview': false,
      'editor.parameterHints.enabled': false, 'editor.quickSuggestions': false,
      'editor.formatOnSave': false, 'editor.formatOnPaste': false, 'editor.formatOnType': false,
      '[ini]': { 'editor.formatOnSave': true, 'editor.formatOnSaveMode': 'file' },
      'editor.codeActions.triggerOnFocusChange': false, 'editor.codeActionsOnSave': {},
      'editor.detectIndentation': false, 'editor.tabSize': 2, 'editor.insertSpaces': true,
      'files.autoSave': fixture.autosave, 'files.autoSaveDelay': 500,
      'files.trimTrailingWhitespace': false, 'files.insertFinalNewline': false,
      'files.trimFinalNewlines': false, ...fixture.user });
    const workspaceSettings = JSON.stringify(fixture.workspace);
    fs.writeFileSync(path.join(profile, 'User/settings.json'), userSettings);
    fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), workspaceSettings);
    const launchBytes = JSON.stringify({ name: fixture.name, profileSettings: userSettings,
      workspaceSettings, targetText: text, auxiliaryText: readinessText }, null, 2) + '\n';
    fs.writeFileSync(path.join(output, `${fixture.name}-launch-inputs.json`), launchBytes);
    let casePassed = false;
    try {
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'save-code-actions-suite.cjs'), extensionTestsEnv: {
        VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_SAVE_CODE_ACTIONS_CASE: fixture.name,
        VSCLI_REFERENCE_LAUNCHER_PATH: launcher.path, VSCLI_REFERENCE_LAUNCHER_SHA256: launcher.sha256,
        VSCLI_REFERENCE_LAUNCHER_BYTES: String(launcher.bytes),
        VSCLI_REFERENCE_SAVE_CODE_ACTIONS_WORKSPACE: workspace,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(userSettings),
        VSCLI_REFERENCE_WORKSPACE_SETTINGS_SHA256: hash(workspaceSettings) },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
    casePassed = true;
    } finally {
      const runtimeBytes = JSON.stringify({ name: fixture.name,
        profileSettings: fs.readFileSync(path.join(profile, 'User/settings.json'), 'utf8'),
        workspaceSettings: fs.readFileSync(path.join(workspace, '.vscode/settings.json'), 'utf8') }, null, 2) + '\n';
      fs.writeFileSync(path.join(output, `${fixture.name}-runtime-settings.json`), runtimeBytes);
      fs.writeFileSync(path.join(output, `${fixture.name}-worker-io-provenance.json`), JSON.stringify({
        name: fixture.name, casePassed, launcher, launchInputsSha256: hash(launchBytes), runtimeSettingsSha256: hash(runtimeBytes),
        scope: 'Declared launch bytes and actual post-editor settings bytes; upstream migration may change runtime files; no target-outcome retry'
      }, null, 2) + '\n');
    }
  }
  const evidence = cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-evidence.json`))));
  const projection = evidence.map(raw => ({ name: raw.name, effective: raw.setup.effective,
    effectiveKeys: raw.setup.effectiveKeys, observations: raw.observations,
    callbacks: raw.callbacks.filter(value => value.phase === 'target').map(({ only, triggerKind, text, returned }) => ({ only, triggerKind, text, returned })) }));
  const write = (name, value) => {
    const bytes = JSON.stringify(value, null, 2) + '\n'; fs.writeFileSync(path.join(output, name), bytes); return hash(bytes);
  };
  const traceSha256 = write('save-code-actions.json', projection), evidenceSha256 = write('save-code-actions-evidence.json', evidence);
  write('save-code-actions-provenance.json', { observerProtocol: 'stable-save-source-api-diagnostics-v1', version: '1.95.0', commit: '912bb683695358a54ae0c670461738984cbb5b95',
    platform: process.platform, architecture: process.arch, caseCount: cases.length,
    snapshotCount: projection.reduce((count, row) => count + row.observations.length, 0), traceSha256, evidenceSha256,
    casesSha256: hash(fs.readFileSync(path.join(__dirname, 'save-code-actions-cases.json'))),
    fixtureReceipts: cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-worker-io-provenance.json`)))),
    runs: cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-provenance.json`)))) });
  console.log(`Observed ${cases.length} named synthetic source-action save fixtures`);
  const invalid = evidence.filter(raw => raw.stableConfiguration.valid !== true).map(raw => raw.name);
  assert.equal(invalid.length, 0, `Target-time canonical configuration differed: ${invalid.join(', ')}`);
}
main().then(() => process.send({ type: 'complete', ok: true }), error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
