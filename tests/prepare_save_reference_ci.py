#!/usr/bin/env python3
"""Start one trusted CI capture without reusing or overwriting cached evidence."""
import os
from pathlib import Path
import re
import subprocess
import uuid

CAPTURE = Path('target/save-actions-stable-configuration')
STAGED = Path('target/save-actions-stable-reference-artifact')


def prepare(workspace, expected, archive_root=None):
    workspace = Path(workspace)
    actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=workspace, text=True, timeout=10).strip()
    if not re.fullmatch('[0-9a-f]{40}', expected) or actual != expected:
        raise ValueError('Checked-out revision differs from trusted workflow revision')
    (workspace / 'target/vscode-reference').mkdir(parents=True, exist_ok=True)
    archives = Path(archive_root) if archive_root else workspace / 'target'
    archives.mkdir(parents=True, exist_ok=True)
    for relative in [CAPTURE, STAGED]:
        current = workspace / relative
        if current.exists() or current.is_symlink():
            archive = archives / (current.name + '.previous-' + str(uuid.uuid4()))
            current.rename(archive)
    capture = workspace / CAPTURE
    capture.mkdir()
    with (capture / 'candidate-revision.txt').open('x', encoding='utf-8') as output:
        output.write(expected + '\n')


if __name__ == '__main__':
    prepare(Path.cwd(), os.environ.get('CANDIDATE_REVISION', ''), os.environ.get('RUNNER_TEMP'))
