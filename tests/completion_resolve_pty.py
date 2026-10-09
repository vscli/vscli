#!/usr/bin/env python3
"""Original CommonJS completion identity, imports, snippets, and stale resolve leases."""
from pathlib import Path
import os
import signal
import sys
import tempfile
import time
from extension_sessions_pty import Editor, LIVE, wait, save
from pty_smoke import CTRL_Z, eventually


def run(root):
    source = root / 'input.sql'
    original = 'sel 🙂\r\nfrom table;\r\n'
    source.write_bytes(original.encode())
    extension = Path(__file__).resolve().parent / 'fixtures' / 'completion-resolve-extension'
    app = Editor(root, '--extension', extension, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[C' * 3 + b'\x1b[32;5u')
    wait(app, 'Resolved SELECT snippet')
    app.send(b'\t')
    save(app, source, 'SELECT 猫 🙂\r\n-- import 界\r\nfrom table;\r\n')
    app.send('犬')
    save(app, source, 'SELECT 犬 🙂\r\n-- import 界\r\nfrom table;\r\n')
    app.send(CTRL_Z)
    save(app, source, 'SELECT 猫 🙂\r\n-- import 界\r\nfrom table;\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: original extension completion resolve, Unicode snippet fields/imports, CRLF persistence and one completion Undo')

    source.write_bytes(original.encode())
    (root / 'resolve-started').unlink()
    (root / 'hold-resolve').touch()
    app = Editor(root, '--extension', extension, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[32;5u')
    wait(app, 'Suggestions')
    eventually(lambda: app.read() and (root / 'resolve-started').exists())
    app.send(b'\t')  # Accept waits for the held original item, and cannot partially edit.
    app.send('X')
    save(app, source, 'X' + original)
    (root / 'hold-resolve').unlink()
    deadline = time.monotonic() + 0.4
    while time.monotonic() < deadline:
        app.read()
        time.sleep(0.01)
    save(app, source, 'X' + original)
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: held resolve acceptance retires on typing; released result cannot overwrite text, imports, Undo or disk')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-resolve-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if app.process.poll() is None:
                        os.kill(app.process.pid, signal.SIGTERM)
                        eventually(lambda: app.read() and app.process.poll() is not None, timeout=4)
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    if app.process.poll() is None:
                        app.process.kill()
                        app.process.wait(timeout=3)
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
