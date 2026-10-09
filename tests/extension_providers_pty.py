#!/usr/bin/env python3
"""Native terminal workflows driven by an optional real provider process."""
from pathlib import Path
import os
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait, save
from pty_smoke import CTRL_Z


def run(root):
    source = root / 'input.sql'
    target = root / 'target.sql'
    original = 'sel 🙂\r\nfrom table;\r\n'
    source.write_bytes(original.encode())
    target.write_bytes('α🙂 target\r\n'.encode())
    extension = Path(__file__).resolve().parent / 'fixtures' / 'language-provider-extension'
    app = Editor(root, '--extension', extension, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[32;5u')  # original Ctrl+Space
    wait(app, 'Suggestions')
    app.send(b'\x1b[B\r')
    save(app, source, 'SELECT 🙂\r\nfrom table;\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    command(app, 'Language: Hover')
    wait(app, 'Native provider hover')
    app.send(b'\x1b')
    command(app, 'Language: Parameter Hints')
    wait(app, 'query(🙂value)')
    app.send(b'\x1b[27;2u')  # Shift+Escape
    command(app, 'Go to Symbol in Editor')
    wait(app, 'query')
    app.send('query\r')
    command(app, 'Language: Go to Definition')
    wait(app, 'Extension location opened')
    app.send('猫')
    save(app, target, 'α猫 target\r\n')
    app.send(CTRL_Z)
    save(app, target, 'α🙂 target\r\n')
    command(app, 'Language: Find References')
    wait(app, 'Extension location opened')
    app.send('界')
    save(app, target, 'α界 target\r\n')
    app.send(CTRL_Z)
    save(app, target, 'α🙂 target\r\n')
    command(app, 'File: Open File')
    app.send(str(source) + '\r')
    wait(app, 'input.sql  |  sql')
    app.send(b'\x1b[105;6u')  # original Linux Ctrl+Shift+I
    wait(app, 'Extension formatting applied')
    save(app, source, 'SELECT 🙂\r\nFROM table;\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: seven native extension provider workflows, original shortcuts, searchable symbols, Unicode navigation, CRLF save and undo')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-provider-pty-') as directory:
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
