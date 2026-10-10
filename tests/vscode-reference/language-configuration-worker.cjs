'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const corpus = require('./language-configuration-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();

async function main() {
  const [output, session, onlyVariant] = process.argv.slice(2);
  const variants = onlyVariant ? corpus.variants.filter(variant => variant.id === onlyVariant) : corpus.variants;
  assert.ok(variants.length, 'Unknown requested configuration fixture');
  fs.mkdirSync(output, { recursive: true });
  const executable = await downloadAndUnzipVSCode({ version: '1.95.0',
    cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const fixtures = [];
  for (const variant of variants) {
    const profile = path.join(session, variant.id, 'profile'), extensions = path.join(session, variant.id, 'extensions');
    const fixture = path.join(session, variant.id, 'fixture');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(extensions, { recursive: true });
    fs.mkdirSync(fixture, { recursive: true });
    const settings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false,
      'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
      'editor.parameterHints.enabled': false, 'editor.quickSuggestions': false,
      'editor.autoIndent': 'full', 'editor.tabSize': 4, 'editor.insertSpaces': true,
      'editor.detectIndentation': false, 'editor.autoClosingQuotes': 'languageDefined',
      'editor.autoClosingBrackets': 'languageDefined', 'editor.autoSurround': 'languageDefined' });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), settings);
    const manifest = { name: `language-configuration-${variant.id}`, publisher: 'vscli-test',
      version: '0.0.0', private: true, license: 'MIT', engines: { vscode: '^1.95.0' },
      contributes: { languages: [{ id: variant.language, configuration: './language-configuration.json' }] } };
    const configuration = { ...variant.configuration, onEnterRules: [{
      beforeText: '^VSCLI_CONFIG_READY$', action: { indent: 'none', appendText: '!' },
    }] };
    const manifestBytes = JSON.stringify(manifest, null, 2) + '\n';
    const configurationBytes = JSON.stringify(configuration, null, 2) + '\n';
    fs.writeFileSync(path.join(fixture, 'package.json'), manifestBytes);
    fs.writeFileSync(path.join(fixture, 'language-configuration.json'), configurationBytes);
    const saved = path.join(output, 'fixtures', variant.id);
    fs.mkdirSync(saved, { recursive: true });
    fs.writeFileSync(path.join(saved, 'package.json'), manifestBytes);
    fs.writeFileSync(path.join(saved, 'language-configuration.json'), configurationBytes);
    fixtures.push({ variant: variant.id, manifest, configuration,
      manifestSha256: hash(manifestBytes), configurationSha256: hash(configurationBytes) });
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: fixture,
      extensionTestsPath: path.join(__dirname, 'language-configuration-suite.cjs'),
      extensionTestsEnv: { VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_CONFIG_VARIANT: variant.id,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(settings) },
      launchArgs: ['--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = variants.map(variant => JSON.parse(fs.readFileSync(path.join(output, `${variant.id}-evidence.json`))));
  const cases = corpus.cases.filter(fixture => variants.some(variant => variant.id === fixture.variant));
  const byName = new Map(evidence.flatMap(variant => variant.traces).map(trace => [trace.name, trace]));
  assert.equal(byName.size, cases.length, 'Missing or duplicate actual reference cases');
  const traces = cases.map(fixture => {
    const trace = byName.get(fixture.name);
    assert.ok(trace, `Missing actual reference case: ${fixture.name}`);
    return { name: trace.name, observations: trace.observations.map(({ action, text, selections }) => ({ action, text, selections })) };
  });
  const write = (name, value) => {
    const bytes = JSON.stringify(value, null, 2) + '\n';
    fs.writeFileSync(path.join(output, name), bytes);
    return hash(bytes);
  };
  const traceSha256 = write('language-configuration.json', traces);
  const evidenceSha256 = write('language-configuration-evidence.json', evidence);
  const runs = variants.map(variant => JSON.parse(fs.readFileSync(path.join(output, `${variant.id}-provenance.json`))));
  write('language-configuration-provenance.json', { version: '1.95.0',
    commit: '912bb683695358a54ae0c670461738984cbb5b95', platform: process.platform,
    architecture: process.arch, fixtureCount: traces.length, traceSha256, evidenceSha256,
    casesSha256: hash(fs.readFileSync(path.join(__dirname, 'language-configuration-cases.json'))), fixtures, runs });
  console.log(`Observed ${traces.length} installed declarative configuration cases across ${variants.length} fresh processes`);
}
main().then(() => process.send({ type: 'complete', ok: true }),
  error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
