#!/usr/bin/env python3
"""Prepare the unchanged official Code Actions sample using its committed lockfile.

This is sample conformance provenance, not qualification of a production extension.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

REPOSITORY = 'https://github.com/microsoft/vscode-extension-samples.git'
COMMIT = '73e249b3c1ba8422aa5713f1a5db23ed0eab4f7a'
PACKAGE = 'vscode-samples.code-actions-sample'
VERSION = '0.0.2'
LICENSE_SHA256 = '20535828272932407c2f5172aeb714ac7b374a34e5ecb1825af509f2902cde54'
LOCK_SHA256 = '96ea649202b0745b431b6b3143fef76b3808e20453ae86d61ad5990641a6faed'


def command(args, cwd=None):
    return subprocess.run(args, cwd=cwd, check=True, timeout=180,
                          text=True, capture_output=True).stdout.strip()


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def source(checkout, sample):
    if command(['git', 'rev-parse', 'HEAD'], checkout) != COMMIT:
        raise RuntimeError('Sample checkout is not pinned; preserve it and use another directory')
    command(['git', 'diff', '--exit-code', 'HEAD', '--'], checkout)
    manifest = json.loads((sample / 'package.json').read_text())
    if (manifest.get('publisher') + '.' + manifest.get('name'), manifest.get('version')) != (PACKAGE, VERSION):
        raise RuntimeError('Pinned sample package identity/version changed')
    if (digest(checkout / 'LICENSE') != LICENSE_SHA256
            or digest(sample / 'package-lock.json') != LOCK_SHA256
            or json.loads((checkout / 'package.json').read_text()).get('license') != 'MIT'):
        raise RuntimeError('Pinned sample license or committed lockfile changed')
    files = command(['git', 'ls-files', '--', 'code-actions-sample', 'LICENSE', 'package.json'], checkout).splitlines()
    return {file: digest(checkout / file) for file in files}


def prepare(directory, verify_only=False):
    directory = directory.resolve()
    checkout = directory.parent if directory.name == 'code-actions-sample' else directory
    sample = checkout / 'code-actions-sample'
    if not checkout.exists():
        if verify_only:
            raise RuntimeError('Pinned sample checkout does not exist')
        checkout.parent.mkdir(parents=True, exist_ok=True)
        command(['git', 'clone', '--filter=blob:none', '--no-checkout', REPOSITORY, str(checkout)])
        command(['git', 'checkout', '--detach', COMMIT], checkout)
    original = source(checkout, sample)
    if not verify_only:
        command(['npm', 'ci', '--ignore-scripts', '--no-audit', '--no-fund'], sample)
        command(['npm', 'run', 'compile'], sample)
    if source(checkout, sample) != original:
        raise RuntimeError('Preparing the sample changed upstream source or lockfile')
    if not (sample / 'out/extension.js').is_file():
        raise RuntimeError('Compiled pinned entry point is missing; prepare first')
    provenance = sample / 'out/vscli-preparation-provenance.json'
    compiled = {str(file.relative_to(sample)): digest(file)
                for file in sorted((sample / 'out').rglob('*'))
                if file.is_file() and file != provenance}
    # The sample has no production dependencies. Record the installed compiler
    # too, so a changed build dependency cannot masquerade as this preparation.
    dependencies = {str(file.relative_to(sample)): digest(file)
                    for file in sorted((sample / 'node_modules/typescript').rglob('*'))
                    if file.is_file()}
    runtime = command(['npm', 'ls', '--omit=dev', '--all', '--parseable'], sample).splitlines()
    if any(Path(path).resolve() != sample for path in runtime):
        raise RuntimeError('Pinned official sample unexpectedly has runtime dependencies')
    report = {
        'schema': 1, 'repository': REPOSITORY, 'commit': COMMIT,
        'package': PACKAGE, 'version': VERSION, 'license': 'MIT',
        'licenseFile': '../LICENSE', 'licenseSha256': LICENSE_SHA256,
        'lockSha256': LOCK_SHA256, 'sourceUnchanged': True, 'lockedDependencies': True,
        'installation': ['npm', 'ci', '--ignore-scripts', '--no-audit', '--no-fund'],
        'build': ['npm', 'run', 'compile'], 'sourceSha256': original,
        'compiledSha256': compiled, 'compilerDependencySha256': dependencies,
        'runtimeDependencySha256': {},
        'installedLockSha256': digest(sample / 'node_modules/.package-lock.json'),
        'toolchain': {'node': command(['node', '--version']), 'npm': command(['npm', '--version'])},
        'qualification': 'Unchanged official sample preparation; native and PTY sample conformance require separate execution. Not production extension qualification.',
    }
    if verify_only:
        if not provenance.is_file():
            raise RuntimeError('Sample preparation provenance is missing; prepare first')
        previous = json.loads(provenance.read_text())
        for field in ('sourceSha256', 'compiledSha256', 'compilerDependencySha256', 'installedLockSha256'):
            if previous.get(field) != report[field]:
                raise RuntimeError('Prepared sample source, compiled output or dependency hashes changed')
        return previous
    provenance.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path, default=Path(__file__).resolve().parents[1] / 'target/code-actions-sample-upstream')
    parser.add_argument('--verify-only', action='store_true')
    arguments = parser.parse_args()
    report = prepare(arguments.directory, arguments.verify_only)
    print(json.dumps({key: report[key] for key in ('repository', 'commit', 'package', 'version', 'license', 'sourceUnchanged', 'lockedDependencies')}))
