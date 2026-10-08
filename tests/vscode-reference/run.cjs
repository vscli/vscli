'use strict';
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { downloadAndUnzipVSCode, runTests } = require('@vscode/test-electron');
const { layers } = require('./contracts.cjs');

async function main() {
  const root = path.resolve(__dirname, '../..');
  const output = path.resolve(process.argv[2] || path.join(root, 'target/vscode-reference', process.platform));
  const session = fs.mkdtempSync(path.join(os.tmpdir(), 'vscli-reference-'));
  try {
    const profile = path.join(session, 'profile');
    const workspace = path.join(session, 'workspace');
    const extensions = path.join(session, 'extensions');
    fs.mkdirSync(path.join(profile, 'User'), { recursive: true });
    fs.mkdirSync(workspace); fs.mkdirSync(extensions);
    fs.mkdirSync(path.join(workspace, '.vscode'));
    fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), JSON.stringify(layers[1]));
    fs.mkdirSync(output, { recursive: true });
    fs.writeFileSync(path.join(profile, 'User/settings.json'), JSON.stringify({
      'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
      'extensions.autoUpdate': false, 'extensions.autoCheckUpdates': false,
      'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
      ...layers[0],
    }));
    const executable = await downloadAndUnzipVSCode({
      version: '1.95.0', cachePath: path.join(root, 'target/vscode-reference/cache'),
    });
    await runTests({
      vscodeExecutablePath: executable,
      extensionDevelopmentPath: __dirname,
      extensionTestsPath: path.join(__dirname, 'suite.cjs'),
      extensionTestsEnv: { VSCLI_REFERENCE_OUTPUT: output },
      launchArgs: [workspace, '--user-data-dir', profile, '--extensions-dir', extensions,
        '--locale=en', '--skip-welcome', '--skip-release-notes', '--disable-workspace-trust',
        '--disable-gpu', '--no-sandbox'],
    });
  } finally {
    fs.rmSync(session, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
