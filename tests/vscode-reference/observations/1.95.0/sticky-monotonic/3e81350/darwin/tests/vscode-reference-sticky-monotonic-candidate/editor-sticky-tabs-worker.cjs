'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { workerLifetime } = require('./supervisor.cjs');
const corpus = require('./editor-sticky-tabs-cases.json');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
workerLifetime();
function validateCorpus() {
  assert.equal(corpus.version, '1.95.0');
  assert.equal(corpus.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  assert.equal(corpus.cases.length, 18); assert.equal(corpus.files.length, 8);
  assert.equal(new Set(corpus.files.map(file => file.name)).size, 8);
  for (const file of corpus.files) {
    assert.match(file.name, /^[a-h]\.txt$/); assert.equal(typeof file.text, 'string');
    assert.ok(Buffer.byteLength(file.text) < 4096 && !file.text.includes('\0'));
  }
  const names = new Set();
  for (const fixture of corpus.cases) {
    assert.match(fixture.name, /^[a-z0-9-]+$/); assert.ok(!names.has(fixture.name)); names.add(fixture.name);
    assert.ok(['keyboardAndMouse', 'keyboard', 'mouse', 'never'].includes(fixture.closePolicy));
    assert.ok(fixture.setup.length > 0 && fixture.setup.length <= 32 && fixture.steps.length > 0 && fixture.steps.length <= 8);
    assert.equal(fixture.setup[0].api, 'showTextDocument');
    for (const [setup, steps] of [[true, fixture.setup], [false, fixture.steps]]) for (const step of steps) {
      if (step.api) {
        assert.ok(setup); assert.equal(step.api, 'showTextDocument');
        assert.ok(corpus.files.some(file => file.name === step.resource));
        assert.ok(Number.isInteger(step.group) && step.group >= 1 && step.group <= 4);
        assert.equal(typeof step.preview, 'boolean');
        assert.ok(Object.keys(step).every(key => ['api', 'resource', 'group', 'preview', 'selection'].includes(key)));
        if (step.selection) assert.ok(Object.keys(step.selection).length === 2 &&
          ['anchor', 'cursor'].every(key => Number.isSafeInteger(step.selection[key]) && step.selection[key] >= 0 && step.selection[key] <= 4096));
      } else if (step.open) {
        assert.ok(!setup); assert.ok(corpus.files.some(file => file.name === step.open));
        assert.equal(typeof step.preview, 'boolean'); assert.ok(Object.keys(step).every(key => ['open', 'preview'].includes(key)));
      } else {
        assert.equal(typeof step.command, 'string'); assert.ok(step.command.length <= 128);
        assert.ok(Object.keys(step).every(key => ['command', 'args'].includes(key)));
        if (Object.hasOwn(step, 'args')) {
          assert.equal(step.command, 'type'); assert.deepEqual(Object.keys(step.args), ['text']);
          assert.equal(typeof step.args.text, 'string'); assert.ok(Buffer.byteLength(step.args.text) <= 64);
        }
      }
    }
  }
}
async function executableHash(executable) {
  const file = fs.realpathSync(executable), before = fs.statSync(file);
  assert.ok(before.isFile() && before.size > 0 && before.size <= 512 * 1024 * 1024, 'Launched executable byte budget invalid');
  const digest = createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) digest.update(chunk);
  const after = fs.statSync(file);
  assert.equal(after.size, before.size); assert.equal(after.mtimeMs, before.mtimeMs);
  return digest.digest('hex');
}
async function main() {
  validateCorpus();
  const [output, session, onlyCase] = process.argv.slice(2);
  const selected = onlyCase ? corpus.cases.filter(fixture => fixture.name === onlyCase) : corpus.cases;
  assert.ok(selected.length && selected.length <= 18, 'Unknown sticky case selection');
  fs.mkdirSync(output, { recursive: true });
  const executable = await downloadAndUnzipVSCode({ version: corpus.version,
    cachePath: path.resolve(__dirname, '../../target/vscode-reference/cache') });
  const launchedExecutableSha256 = await executableHash(executable);
  const fixtures = [];
  for (const fixture of selected) {
    const profile = path.join(session, fixture.name, 'profile'), extensions = path.join(session, fixture.name, 'extensions');
    const workspace = path.join(session, fixture.name, 'workspace');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true }); fs.mkdirSync(extensions, { recursive: true }); fs.mkdirSync(workspace, { recursive: true });
    for (const file of corpus.files) {
      fs.writeFileSync(path.join(workspace, file.name), file.text);
      fixtures.push({ case: fixture.name, resource: file.name, sha256: hash(Buffer.from(file.text)) });
    }
    const settings = JSON.stringify({ 'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false, 'security.workspace.trust.enabled': false,
      'workbench.startupEditor': 'none', ...corpus.policy, 'workbench.editor.preventPinnedEditorClose': fixture.closePolicy,
      'files.autoSave': 'off', 'editor.quickSuggestions': false, 'editor.parameterHints.enabled': false,
      'editor.detectIndentation': false, 'editor.tabSize': 4, 'editor.insertSpaces': true, 'editor.wordWrap': 'off',
      'editor.autoClosingQuotes': 'never', 'editor.autoClosingBrackets': 'never', 'editor.formatOnSave': false, 'editor.codeActionsOnSave': {} });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), settings);
    await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'editor-sticky-tabs-suite.cjs'), extensionTestsEnv: {
        VSCLI_REFERENCE_OUTPUT: output, VSCLI_REFERENCE_EDITOR_STICKY_TABS_CASE: fixture.name,
        VSCLI_REFERENCE_EDITOR_STICKY_TABS_WORKSPACE: workspace,
        VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256: hash(settings), VSCLI_REFERENCE_EXECUTABLE_SHA256: launchedExecutableSha256 },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions, '--locale=en',
        '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust', '--disable-gpu', '--no-sandbox'] });
  }
  const evidence = selected.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-evidence.json`))));
  const projection = evidence.map((trace, index) => {
    assert.equal(trace.name, selected[index].name);
    assert.equal(trace.observations.length, selected[index].steps.length + 1);
    assert.equal(trace.setup.operations.length, selected[index].setup.length);
    return { name: trace.name, observations: trace.observations };
  });
  const write = (name, value) => { const bytes = JSON.stringify(value, null, 2) + '\n'; fs.writeFileSync(path.join(output, name), bytes); return hash(bytes); };
  const traceSha256 = write('editor-sticky-tabs.json', projection), evidenceSha256 = write('editor-sticky-tabs-evidence.json', evidence);
  write('editor-sticky-tabs-provenance.json', { version: corpus.version, commit: corpus.commit,
    platform: process.platform, architecture: process.arch, caseCount: selected.length,
    snapshotCount: projection.reduce((count, trace) => count + trace.observations.length, 0),
    setupObservationCount: evidence.reduce((count, trace) => count + trace.setup.operations.length, 0),
    traceSha256, evidenceSha256, casesSha256: hash(fs.readFileSync(path.join(__dirname, 'editor-sticky-tabs-cases.json'))),
    launchedExecutableSha256, files: fixtures, runs: selected.map(fixture => JSON.parse(fs.readFileSync(path.join(output, `${fixture.name}-provenance.json`)))) });
  console.log(`Observed ${selected.length} original-command sticky cases with complete public observations`);
}
main().then(() => process.send({ type: 'complete', ok: true }),
  error => process.send({ type: 'complete', ok: false, error: error.stack || String(error) }));
