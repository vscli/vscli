'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const vscode = require('vscode');
const { navigationHistoryTrace } = require('./navigation-history.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
exports.run = async () => {
  assert.equal(vscode.version, '1.95.0');
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const name = process.env.VSCLI_REFERENCE_NAVIGATION_CASE;
  const evidence = await navigationHistoryTrace(vscode, name, process.env.VSCLI_REFERENCE_NAVIGATION_WORKSPACE);
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  const bytes = JSON.stringify(evidence, null, 2) + '\n';
  fs.writeFileSync(path.join(output, `${name}-evidence.json`), bytes);
  const artifact = name => hash(fs.readFileSync(path.join(__dirname, name)));
  fs.writeFileSync(path.join(output, `${name}-provenance.json`), JSON.stringify({
    version: vscode.version, commit: product.commit, platform: process.platform, architecture: process.arch,
    observedAt: new Date().toISOString(), name, productSha256: hash(productBytes), evidenceSha256: hash(bytes),
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
    observerSha256: artifact('navigation-history.cjs'), casesSha256: artifact('navigation-history-cases.json'),
    suiteSha256: artifact('navigation-history-suite.cjs'), runnerSha256: artifact('navigation-history-run.cjs'),
    workerSha256: artifact('navigation-history-worker.cjs'), supervisorSha256: artifact('supervisor.cjs'),
    testElectronLockSha256: artifact('package-lock.json'),
    harnessManifestSha256: artifact('package.json'), harnessExtensionSha256: artifact('extension.cjs'),
    projection: 'Actual single-group session history via public file opens, commands, quickOpen/accept GoToLine, primary scalar positions and dirty text; setup and composite preview recorded separately. No private stack, persisted/recent history, multi-pane equivalence or asynchronous native navigation qualification.',
  }, null, 2) + '\n');
};
