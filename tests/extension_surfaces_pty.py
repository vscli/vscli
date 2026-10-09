#!/usr/bin/env python3
"""Native output/status/lazy-tree interactions with real extension commands."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait, save
from pty_smoke import CTRL_Z, eventually

FIXTURE = Path(__file__).resolve().parent / 'fixtures' / 'extension-surfaces'


def run(root):
    file = root / 'unicode.txt'
    file.write_bytes('original猫\r\n'.encode())
    app = Editor(root, '--extension', FIXTURE, file, enhanced=True)
    wait(app, '(8 commands)')
    wait(app, 'Native Ready')
    command(app, 'Surface show')
    wait(app, 'Output: Native Fixture Output')
    wait(app, 'LOG 猫🙂')
    app.paste('MUST NOT EDIT')
    app.send('x\r\x7f')
    app.send(b'\x1b')
    command(app, 'Extensions: Status Items')
    wait(app, 'Extension Status Items')
    wait(app, 'Runs opaque action')
    app.send(b'\r')
    wait(app, 'Surface applied=true')
    save(app, file, 'INSERTED猫original猫\r\n')
    app.send(CTRL_Z)
    save(app, file, 'original猫\r\n')
    command(app, 'Extensions: Tree Views')
    wait(app, 'Extension Tree Views')
    app.send(b'\r')
    wait(app, 'Group')
    app.send(b'\x1b[C')
    wait(app, 'Leaf 猫')
    app.send(b'\x1b[B\r')
    wait(app, 'Surface applied=true')
    save(app, file, 'INSERTED猫original猫\r\n')
    app.send(CTRL_Z)
    save(app, file, 'original猫\r\n')
    command(app, 'Surface refresh')
    wait(app, 'Extension command completed')
    command(app, 'Extensions: Tree Views')
    wait(app, 'Extension Tree Views')
    app.send(b'\r')
    wait(app, 'Group refreshed')
    app.send(b'\x1b')
    command(app, 'Surface crash')
    wait(app, 'Extension host stopped')
    eventually(lambda: app.read() and 'Native Ready' not in app.screen.text())
    save(app, file, 'original猫\r\n')
    app.finish()
    print('PASS: read-only output, status/tree opaque edits, refresh, crash, Unicode CRLF save/undo')

    empty = root / 'empty'
    empty.mkdir()
    app = Editor(empty, '--extension', FIXTURE, enhanced=True)
    wait(app, '(8 commands)')
    wait(app, 'No open editors')
    command(app, 'Extensions: Status Items')
    wait(app, 'Extension Status Items')
    app.send(b'\r')
    wait(app, 'No active editor; no document created')
    wait(app, 'No open editors')
    command(app, 'Surface prompt')
    wait(app, 'Surface input')
    app.send(b'\r')
    wait(app, 'Surface input=answer')
    wait(app, 'No open editors')
    app.send(b'\x1b')
    command(app, 'Surface dispose')
    wait(app, 'Extension command completed')
    eventually(lambda: app.read() and 'Native Ready' not in app.screen.text())
    app.finish()
    print('PASS: empty workbench status/output/input and disposal create no phantom document')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-surfaces-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            original_failure = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if app.process.poll() is None:
                        os.kill(app.process.pid, signal.SIGTERM)
                        try:
                            app.process.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            app.process.kill()
                            app.process.wait(timeout=3)
                except Exception as error:
                    errors.append(error)
                finally:
                    app.close_fds()
            if errors:
                if original_failure:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
