'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const vscode = require('vscode');
const { languageConfigurationTrace } = require('./language-configuration.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

exports.run = async () => {
  assert.equal(vscode.version, '1.95.0');
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const evidence = await languageConfigurationTrace(vscode, process.env.VSCLI_REFERENCE_CONFIG_VARIANT);
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  const bytes = JSON.stringify(evidence, null, 2) + '\n';
  fs.writeFileSync(path.join(output, `${evidence.variant}-evidence.json`), bytes);
  const artifact = name => hash(fs.readFileSync(path.join(__dirname, name)));
  fs.writeFileSync(path.join(output, `${evidence.variant}-provenance.json`), JSON.stringify({
    version: vscode.version, commit: product.commit, platform: process.platform, architecture: process.arch,
    observedAt: new Date().toISOString(), variant: evidence.variant, fixtureCount: evidence.traces.length,
    productSha256: hash(productBytes), evidenceSha256: hash(bytes),
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
    observerSha256: artifact('language-configuration.cjs'), casesSha256: artifact('language-configuration-cases.json'),
    suiteSha256: artifact('language-configuration-suite.cjs'), runnerSha256: artifact('language-configuration-run.cjs'),
    workerSha256: artifact('language-configuration-worker.cjs'), supervisorSha256: artifact('supervisor.cjs'),
    testElectronLockSha256: artifact('package-lock.json'),
    projection: 'Actual installed contributes.languages JSON configuration over built-in CPP or a declared fixture-only language. No setLanguageConfiguration API. Public type/surround/comment commands, Undo/Redo bytes and scalar selections. Independent fixture-only onEnterRules loader sentinel excluded from native comparison; arbitrary native regex is not qualified. Typed target tokens prepared with unchanged bytes/version/selections/EOL; no preparation between Undo/Redo and no target retry. Unknown language and malformed declarations remain raw reference observations until native scope is separately qualified. Cross-extension general priority and lifecycle are not qualified by one development fixture per process.',
  }, null, 2) + '\n');
};
