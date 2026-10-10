'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const corpus = require('./navigation-history-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();
async function main() {
  const [output, session, onlyCase] = process.argv.slice(2);
  const cases = onlyCase ? corpus.filter(fixture => fixture.name === onlyCase) : corpus;
  assert.ok(cases.length, 'Unknown requested navigation fixture');
  fs.mkdirSync(output, { recursive: true });
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0',
    cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const files = [];
  for (const fixture of cases) {
    assert.match(fixture.name, /^[a-z0-9-]+$/);
    const profile = path.join(session, fixture.name, 'profile'), extensions = path.join(session, fixture.name, 'extensions');
    const workspace = path.join(session, fixture.name, 'workspace');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true });
    fs.mkdirSync(workspace, { recursive: true });
    for (const file of fixture.files) {
      assert.match(file.name, /^[a-z0-9-]+\.cpp$/);
      fs.writeFileSync(path.join(workspace, file.name), file.text);
      files.push({ case: fixture.name, resource: file.name, sha256: hash(Buffer.from(file.text)) });
    }
    const settings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false,
      'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
      'workbench.editor.enablePreview': false, 'workbench.editor.navigationScope': 'default',
      'editor.parameterHints.enabled': false, 'editor.quickSuggestions': false,
      'editor.detectIndentation': false, 'editor.tabSize': 4, 'editor.insertSpaces': true,
      'editor.wordWrap': 'off', 'editor.autoClosingQuotes': 'never', 'editor.autoClosingBrackets': 'never' });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), settings);
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'navigation-history-suite.cjs'),
      extensionTestsEnv: { VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_NAVIGATION_CASE: fixture.name,
        VSCLI_REFERENCE_NAVIGATION_WORKSPACE: workspace, VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(settings) },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-evidence.json`))));
  assert.equal(new Set(evidence.map(trace => trace.name)).size, cases.length);
  const traces = evidence.map((trace, index) => {
    assert.equal(trace.name, cases[index].name);
    assert.equal(trace.observations.length, cases[index].steps.length + 1);
    return { name: trace.name, observations: trace.observations.map(({ action, resource, text, primary, dirty }) =>
      ({ action, resource, text, primary, dirty })) };
  });
  const write = (name, value) => {
    const bytes = JSON.stringify(value, null, 2) + '\n'; fs.writeFileSync(path.join(output, name), bytes); return hash(bytes);
  };
  const traceSha256 = write('navigation-history.json', traces);
  const evidenceSha256 = write('navigation-history-evidence.json', evidence);
  const runs = cases.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-provenance.json`))));
  write('navigation-history-provenance.json', { version: '1.95.0',
    commit: '912bb683695358a54ae0c670461738984cbb5b95', platform: process.platform, architecture: process.arch,
    caseCount: cases.length, snapshotCount: traces.reduce((total, trace) => total + trace.observations.length, 0),
    traceSha256, evidenceSha256, casesSha256: hash(fs.readFileSync(path.join(__dirname, 'navigation-history-cases.json'))), files, runs });
  console.log(`Observed ${cases.length} actual navigation-history cases in fresh isolated processes`);
}
main().then(() => process.send({ type: 'complete', ok: true }),
  error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
