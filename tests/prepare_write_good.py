#!/usr/bin/env python3
"""Prepare an unchanged pinned Write Good Linter with its committed lockfile."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

REPOSITORY = 'https://github.com/TravisTheTechie/vscode-write-good.git'
COMMIT = 'c1bf30a5983fd390d63e3d2392f84ca162a72e7b'
PACKAGE = 'travisthetechie.write-good-linter'
VERSION = '0.1.7'


def command(args, cwd=None, timeout=180):
    return subprocess.run(args, cwd=cwd, check=True, timeout=timeout,
                          text=True, capture_output=True).stdout.strip()


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def verify_source(directory):
    if command(['git', 'rev-parse', 'HEAD'], directory) != COMMIT:
        raise RuntimeError('Write Good checkout is not the pinned commit; preserve it and use another directory')
    command(['git', 'diff', '--exit-code', 'HEAD', '--'], directory)
    manifest = json.loads((directory / 'package.json').read_text())
    if (manifest.get('publisher') + '.' + manifest.get('name'), manifest.get('version'), manifest.get('license')) != (PACKAGE, VERSION, 'MIT'):
        raise RuntimeError('Pinned Write Good package identity/version/license changed')
    source_files = command(['git', 'ls-files'], directory).splitlines()
    return {file: digest(directory / file) for file in source_files}


def prepare(directory, verify_only=False):
    directory = directory.resolve()
    if not directory.exists():
        if verify_only:
            raise RuntimeError('Pinned package directory does not exist')
        directory.parent.mkdir(parents=True, exist_ok=True)
        command(['git', 'clone', '--filter=blob:none', '--no-checkout', REPOSITORY, str(directory)])
        command(['git', 'checkout', '--detach', COMMIT], directory)
    original = verify_source(directory)
    if not verify_only:
        command(['npm', 'ci', '--ignore-scripts', '--no-audit', '--no-fund'], directory)
        command(['npm', 'run', 'compile'], directory)
    if verify_source(directory) != original:
        raise RuntimeError('Preparing Write Good changed upstream source or lockfile')
    entry = directory / 'out/src/extension.js'
    if not entry.is_file():
        raise RuntimeError('Compiled pinned entry point is missing; run preparation without --verify-only')
    compiled = {str(file.relative_to(directory)): digest(file)
                for file in sorted((directory / 'out').rglob('*')) if file.is_file()}
    runtime = {}
    for package in command(['npm', 'ls', '--omit=dev', '--all', '--parseable'], directory).splitlines():
        package = Path(package)
        if package == directory:
            continue
        for file in sorted(package.rglob('*')):
            if file.is_file():
                runtime[str(file.relative_to(directory))] = digest(file)
    report = {
        'schema': 1, 'repository': REPOSITORY, 'commit': COMMIT,
        'package': PACKAGE, 'version': VERSION, 'license': 'MIT',
        'licenseFile': 'LICENSE.txt', 'licenseSha256': digest(directory / 'LICENSE.txt'),
        'sourceUnchanged': True, 'lockedDependencies': True,
        'installation': ['npm', 'ci', '--ignore-scripts', '--no-audit', '--no-fund'],
        'build': ['npm', 'run', 'compile'], 'sourceSha256': original,
        'compiledSha256': compiled,
        'runtimeDependencySha256': runtime,
        'installedLockSha256': digest(directory / 'node_modules/.package-lock.json'),
        'toolchain': {'node': command(['node', '--version']), 'npm': command(['npm', '--version'])},
        'qualification': 'Preparation verifies pinned source/build provenance; editor compatibility requires separate native and PTY tests.',
    }
    provenance = directory / 'out/vscli-preparation-provenance.json'
    if verify_only:
        if not provenance.is_file():
            raise RuntimeError('Preparation provenance is missing; run normal preparation first')
        previous = json.loads(provenance.read_text())
        # The provenance file itself is not a generated compiler artifact.
        compiled.pop('out/vscli-preparation-provenance.json', None)
        if (previous.get('sourceSha256') != original or previous.get('compiledSha256') != compiled
                or previous.get('runtimeDependencySha256') != runtime
                or previous.get('installedLockSha256') != report['installedLockSha256']):
            raise RuntimeError('Prepared source or compiled artifact hashes changed')
        return previous
    compiled.pop('out/vscli-preparation-provenance.json', None)
    provenance.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path, default=Path(__file__).resolve().parents[1] / 'target/write-good-upstream')
    parser.add_argument('--verify-only', action='store_true')
    arguments = parser.parse_args()
    report = prepare(arguments.directory, arguments.verify_only)
    print(json.dumps({key: report[key] for key in ('repository', 'commit', 'package', 'version', 'license', 'sourceUnchanged', 'lockedDependencies')}))
