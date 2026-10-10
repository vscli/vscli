'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const vscode = require('vscode');
const { advancedIndentation } = require('./advanced-indentation.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

exports.run = async () => {
  assert.equal(vscode.version, '1.95.0');
  const productBytes = fs.readFileSync(path.join(vscode.env.appRoot, 'product.json'));
  const product = JSON.parse(productBytes);
  assert.equal(product.commit, '912bb683695358a54ae0c670461738984cbb5b95');
  const evidence = await advancedIndentation(vscode, process.env.VSCLI_REFERENCE_INDENT_MODE);
  const output = process.env.VSCLI_REFERENCE_OUTPUT;
  const bytes = JSON.stringify(evidence, null, 2) + '\n';
  fs.writeFileSync(path.join(output, `${evidence.mode}-evidence.json`), bytes);
  const artifact = name => hash(fs.readFileSync(path.join(__dirname, name)));
  fs.writeFileSync(path.join(output, `${evidence.mode}-provenance.json`), JSON.stringify({
    version: vscode.version, commit: product.commit, platform: process.platform, architecture: process.arch,
    observedAt: new Date().toISOString(), mode: evidence.mode, fixtureCount: evidence.traces.length,
    productSha256: hash(productBytes), evidenceSha256: hash(bytes),
    profileSettingsSha256: process.env.VSCLI_REFERENCE_PROFILE_SETTINGS_SHA256,
    observerSha256: artifact('advanced-indentation.cjs'), casesSha256: artifact('advanced-indentation-cases.json'),
    suiteSha256: artifact('advanced-indentation-suite.cjs'), runnerSha256: artifact('advanced-indentation-run.cjs'),
    workerSha256: artifact('advanced-indentation-worker.cjs'), readinessSha256: artifact('language-readiness.cjs'),
    supervisorSha256: artifact('supervisor.cjs'), testElectronLockSha256: artifact('package-lock.json'),
    projection: 'Actual public type/lineBreakInsert/Undo/Redo bytes and scalar selections. Prepared-token scope: public forceRetokenize before every type/lineBreakInsert, with exact bytes/version/selections/EOL preservation; never between Undo/Redo. Five fresh processes with identical global and CPP/JSON fixed mode at startup. Independent plain Enter mode witness before every target; actual document settings and positive/negative grammar readiness. Target gestures never retried. None versus indentation retention is behavior-witnessed; distinctions among Keep/Brackets/Advanced/Full additionally rely on fixed startup settings and actual document configuration. Natural tokenization startup timing and arbitrary language/profile parity are outside this contract.',
  }, null, 2) + '\n');
};
