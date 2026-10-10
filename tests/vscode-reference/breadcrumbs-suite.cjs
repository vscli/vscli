'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const vscode = require('vscode');
const { breadcrumbsTrace } = require('./breadcrumbs.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
exports.run = async () => {
  assert.equal(vscode.version, '1.95.0');
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const name = process.env.VSCLI_REFERENCE_BREADCRUMBS_CASE;
  const evidence = await breadcrumbsTrace(vscode, name, process.env.VSCLI_REFERENCE_BREADCRUMBS_WORKSPACE);
  const output = process.env.VSCLI_REFERENCE_OUTPUT, bytes = JSON.stringify(evidence, null, 2) + '\n';
  fs.writeFileSync(path.join(output, `${name}-evidence.json`), bytes);
  const sources = Object.fromEntries(['breadcrumbs.cjs','breadcrumbs-cases.json','breadcrumbs-suite.cjs','breadcrumbs-run.cjs',
    'breadcrumbs-worker.cjs','supervisor.cjs','package-lock.json','package.json','extension.cjs']
    .map(name => [name, hash(fs.readFileSync(path.join(__dirname, name)))]));
  fs.writeFileSync(path.join(output, `${name}-provenance.json`), JSON.stringify({ version: vscode.version,
    commit: product.commit, platform: process.platform, architecture: process.arch, observedAt: new Date().toISOString(),
    name, productSha256: hash(productBytes), evidenceSha256: hash(bytes), sources,
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256, scope: evidence.scope }, null, 2) + '\n');
};
