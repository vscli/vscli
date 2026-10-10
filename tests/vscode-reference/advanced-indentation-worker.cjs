'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const fixtures = require('./advanced-indentation-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();

async function main() {
  const [output, session] = process.argv.slice(2);
  fs.mkdirSync(output, { recursive: true });
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0',
    cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const modes = ['none', 'keep', 'brackets', 'advanced', 'full'];
  for (const mode of modes) {
    const profile = path.join(session, mode, 'profile'), extensions = path.join(session, mode, 'extensions');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true });
    const editor = { 'editor.autoIndent': mode, 'editor.tabSize': 4, 'editor.insertSpaces': true,
      'editor.detectIndentation': false, 'editor.autoClosingQuotes': 'languageDefined',
      'editor.autoClosingBrackets': 'languageDefined', 'editor.autoClosingOvertype': 'auto',
      'editor.autoClosingDelete': 'auto' };
    const settings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false,
      'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
      'editor.parameterHints.enabled': false, 'editor.quickSuggestions': false,
      ...editor, '[cpp]': editor, '[json]': editor });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), settings);
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'advanced-indentation-suite.cjs'),
      extensionTestsEnv: { VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_INDENT_MODE: mode,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(settings) },
      launchArgs: ['--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = modes.map(mode => JSON.parse(fs.readFileSync(path.join(output, `${mode}-evidence.json`))));
  const byName = new Map(evidence.flatMap(mode => mode.traces).map(trace => [trace.name, trace]));
  assert.equal(byName.size, fixtures.length, 'Missing or duplicate actual reference cases');
  const traces = fixtures.map(fixture => {
    const trace = byName.get(fixture.name);
    assert.ok(trace, `Missing actual reference case: ${fixture.name}`);
    return { name: trace.name, observations: trace.observations.map(({ action, text, selections }) => ({ action, text, selections })) };
  });
  const write = (name, value) => {
    const bytes = JSON.stringify(value, null, 2) + '\n';
    fs.writeFileSync(path.join(output, name), bytes);
    return hash(bytes);
  };
  const traceSha256 = write('advanced-indentation.json', traces);
  const evidenceSha256 = write('advanced-indentation-evidence.json', evidence);
  const runs = modes.map(mode => JSON.parse(fs.readFileSync(path.join(output, `${mode}-provenance.json`))));
  write('advanced-indentation-provenance.json', { version: '1.95.0',
    commit: '912bb683695358a54ae0c670461738984cbb5b95', platform: process.platform,
    architecture: process.arch, fixtureCount: traces.length, traceSha256, evidenceSha256,
    casesSha256: hash(fs.readFileSync(path.join(__dirname, 'advanced-indentation-cases.json'))), runs });
  console.log(`Observed ${traces.length} prepared-token advanced indentation cases across five fresh processes`);
}
main().then(() => process.send({ type: 'complete', ok: true }),
  error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
