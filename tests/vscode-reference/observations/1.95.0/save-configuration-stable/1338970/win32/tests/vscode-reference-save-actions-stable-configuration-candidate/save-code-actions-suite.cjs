'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { archiveFailure, sourceIdentity } = require('./save-configuration-ready.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
exports.run = async () => {
  const sources = sourceIdentity(__dirname);
  const vscode = require('vscode');
  const { saveCodeActionsTrace } = require('./save-code-actions.cjs');
  assert.equal(vscode.version, '1.95.0');
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const name = process.env.VSCLI_REFERENCE_SAVE_CODE_ACTIONS_CASE;
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  const launcher = { path: process.env.VSCLI_REFERENCE_LAUNCHER_PATH,
    sha256: process.env.VSCLI_REFERENCE_LAUNCHER_SHA256, bytes: Number(process.env.VSCLI_REFERENCE_LAUNCHER_BYTES) };
  assert.ok(typeof launcher.path === 'string' && Buffer.byteLength(launcher.path) <= 4096);
  assert.match(launcher.sha256, /^[a-f0-9]{64}$/);
  assert.ok(Number.isInteger(launcher.bytes) && launcher.bytes > 0 && launcher.bytes <= 512 * 1024 * 1024);
  const metadata = { version: vscode.version, commit: product.commit, platform: process.platform,
    architecture: process.arch, observedAt: new Date().toISOString(), productSha256: hash(productBytes), launcher, sources,
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
    workspaceSettingsSha256: process.env.VSCLI_REFERENCE_WORKSPACE_SETTINGS_SHA256 };
  let evidence;
  try {
    evidence = await saveCodeActionsTrace(vscode, name, process.env.VSCLI_REFERENCE_SAVE_CODE_ACTIONS_WORKSPACE);
  } catch (error) {
    const failure = error.saveReferenceFailure || { protocol: 'stable-save-setup-failure-v1', name,
      phase: 'before-observer', targetStarted: false,
      error: { name: String(error.name || 'Error').slice(0, 128), message: String(error.message || error).slice(0, 1024) },
      scope: 'Observer initialization failed before any target command; diagnostic recorder may be unavailable' };
    archiveFailure(output, name, failure, metadata);
    throw error;
  }
  const bytes = JSON.stringify(evidence, null, 2) + '\n';
  assert.ok(Buffer.byteLength(bytes) <= 2 * 1024 * 1024, 'Stable save evidence exceeds bounds');
  fs.writeFileSync(path.join(output, `${name}-evidence.json`), bytes);
  fs.writeFileSync(path.join(output, `${name}-provenance.json`), JSON.stringify({ version: vscode.version,
    commit: product.commit, platform: process.platform, architecture: process.arch, observedAt: new Date().toISOString(),
    name, productSha256: hash(productBytes), launcher, evidenceSha256: hash(bytes), sources,
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
    workspaceSettingsSha256: process.env.VSCLI_REFERENCE_WORKSPACE_SETTINGS_SHA256,
    scope: evidence.scope }, null, 2) + '\n');
};
