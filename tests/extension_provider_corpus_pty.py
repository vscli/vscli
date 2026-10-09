#!/usr/bin/env python3
"""Opt-in native workflows for unchanged, externally prepared provider packages.

VSCLI_NPM_INTELLISENSE and VSCLI_SQL_FORMATTER must name the pinned compiled trees.
No package is downloaded or modified by this test.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, wait, save
from pty_smoke import CTRL_Z, CTRL_S, eventually

PACKAGES = [('VSCLI_NPM_INTELLISENSE', '1.4.5', 'bc5521b659b700c8a91cf04897007c611ed9532a'),
            ('VSCLI_SQL_FORMATTER', '4.2.6', 'a9988c9e667efca000d17995dae742c9efcdcec8')]


def package(variable, version, commit):
    path = Path(os.environ[variable]).resolve()
    manifest = json.loads((path / 'package.json').read_text())
    assert manifest['version'] == version
    assert subprocess.check_output(['git', '-C', str(path), 'rev-parse', 'HEAD'], text=True).strip() == commit
    subprocess.run(['git', '-C', str(path), 'diff', '--exit-code', 'HEAD', '--'], check=True, capture_output=True)
    return path


def run(root):
    npm, sql = [package(*entry) for entry in PACKAGES]
    (root / 'package.json').write_text(json.dumps({'dependencies': {'lodash': '4.17.21'}}))
    source = root / 'input.js'
    original = "import value from 'lo';\r\n// 🙂\r\n"
    source.write_bytes(original.encode())
    app = Editor(root, '--extension', npm, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[C' * (original.index('lo') + 2))
    app.send(b'\x1b[32;5u')
    wait(app, 'Extension Completion')
    wait(app, 'lodash')
    app.send(b'\r')
    save(app, source, original.replace("'lo'", "'lodash'"))
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: unchanged NPM Intellisense 1.4.5 native completion, CRLF/Unicode save and undo')
    source = root / 'input.sql'
    original = "select '🙂' as greeting, id from users where id=1;\r\n"
    source.write_bytes(original.encode())
    app = Editor(root, '--extension', sql, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[105;6u')
    wait(app, 'Extension formatting applied')
    app.send(CTRL_S)
    eventually(lambda: app.read() and source.read_bytes() != original.encode())
    formatted = source.read_bytes()
    assert b'\r\n' in formatted
    assert b'\n' not in formatted.replace(b'\r\n', b'')
    assert '🙂'.encode() in formatted and b'users' in formatted
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    for entry in PACKAGES:
        package(*entry)
    print('PASS: unchanged SQL Formatter VSCode 4.2.6 native formatting, preserved CRLF/Unicode, save and undo')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-provider-corpus-') as directory:
        try:
            run(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in list(LIVE):
                try:
                    if app.process.poll() is None:
                        try:
                            os.kill(app.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            app.process.wait(timeout=4)
                        except (AssertionError, subprocess.TimeoutExpired):
                            app.process.kill()
                            app.process.wait(timeout=3)
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
