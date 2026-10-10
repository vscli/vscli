'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const cases = require('./editor-group-tabs-cases.json');
const { files, policy } = require('./editor-group-tabs.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();
async function main() {
  const [output, session, onlyCase] = process.argv.slice(2);
  const selected = onlyCase ? cases.filter(fixture => fixture.name === onlyCase) : cases;
  assert.ok(selected.length && selected.length <= 10, 'Unknown or oversized group case cohort');
  fs.mkdirSync(output, { recursive: true });
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0', cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const fixtures = [];
  for (const fixture of selected) {
    assert.match(fixture.name, /^[a-z0-9-]+$/);
    const profile = path.join(session, fixture.name, 'profile'), extensions = path.join(session, fixture.name, 'extensions');
    const workspace = path.join(session, fixture.name, 'workspace');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true }); fs.mkdirSync(workspace, { recursive: true });
    for (const [name, text] of Object.entries(files)) {
      assert.match(name, /^[a-d]\.txt$/); fs.writeFileSync(path.join(workspace, name), text);
      fixtures.push({ case: fixture.name, resource: name, sha256: hash(Buffer.from(text)) });
    }
    const settings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false, 'security.workspace.trust.enabled': false,
      'workbench.startupEditor': 'none', ...policy, 'files.autoSave': 'off',
      'editor.quickSuggestions': false, 'editor.parameterHints.enabled': false,
      'editor.detectIndentation': false, 'editor.tabSize': 4, 'editor.insertSpaces': true,
      'editor.wordWrap': 'off', 'editor.autoClosingQuotes': 'never', 'editor.autoClosingBrackets': 'never',
      'editor.formatOnSave': false, 'editor.codeActionsOnSave': {} });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), settings);
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'editor-group-tabs-suite.cjs'), extensionTestsEnv: {
        VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_EDITOR_GROUP_TABS_CASE: fixture.name,
        VSCLI_REFERENCE_EDITOR_GROUP_TABS_WORKSPACE: workspace,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(settings) },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = selected.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-evidence.json`))));
  const projection = evidence.map((trace, index) => {
    assert.equal(trace.name, selected[index].name);
    assert.equal(trace.observations.length, selected[index].steps.length + 1);
    // Exact public observations are retained. A consumer may project fields,
    // but must not normalize versions/object-ordinal identities into parity.
    return { name: trace.name, observations: trace.observations };
  });
  const write = (name, value) => { const bytes = JSON.stringify(value, null, 2) + '\n'; fs.writeFileSync(path.join(output, name), bytes); return hash(bytes); };
  const traceSha256 = write('editor-group-tabs.json', projection), evidenceSha256 = write('editor-group-tabs-evidence.json', evidence);
  write('editor-group-tabs-provenance.json', { version: '1.95.0', commit: '912bb683695358a54ae0c670461738984cbb5b95',
    platform: process.platform, architecture: process.arch, caseCount: selected.length,
    snapshotCount: projection.reduce((n, trace) => n + trace.observations.length, 0), traceSha256, evidenceSha256,
    casesSha256: hash(fs.readFileSync(path.join(__dirname, 'editor-group-tabs-cases.json'))), files: fixtures,
    runs: selected.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-provenance.json`)))) });
  console.log(`Observed ${selected.length} named committed editor-group tab cases`);
}
main().then(() => process.send({ type: 'complete', ok: true }),
  error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
