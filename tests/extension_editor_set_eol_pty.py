#!/usr/bin/env python3
"""Public EOL/text notifications and separate native history through the real terminal."""
from pathlib import Path
import hashlib
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import command
from profile_migration_pty import finish_after_failure


def run():
    fixture = Path(__file__).resolve().parent / 'fixtures/editor-set-eol-extension'
    originals = {p: hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-editor-set-eol-') as temporary:
        root = Path(temporary)
        file = root / 'eol.txt'
        original = 'λ🙂 original\nsecond\n'.encode()
        converted = 'λ🙂 original\r\nsecond\r\n'.encode()
        final = 'λ🙂\r\nX original\r\nsecond\r\n'.encode()
        file.write_bytes(original)
        app = Editor(root, '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '4 commands', timeout=15)
            command(app, 'Apply editor EOL and text')
            wait_screen(app, 'Editor EOL and text applied', timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == final)
            for key, expected in [(CTRL_Z, converted), (CTRL_Z, original),
                                  (b'\x1b[122;6u', converted), (b'\x1b[122;6u', final)]:
                app.send(key + CTRL_S)
                eventually(lambda: app.read() and file.read_bytes() == expected)
            app.send(CTRL_Z + CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        print('PASS: public EOL event precedes text event and true; two native Undo/Redo steps save exact Unicode LF/CRLF bytes')
    with tempfile.TemporaryDirectory(prefix='vscli-editor-same-eol-') as temporary:
        root = Path(temporary)
        file = root / 'eol.txt'
        file.write_bytes(converted)
        app = Editor(root, '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '4 commands', timeout=15)
            command(app, 'Apply editor EOL only')
            wait_screen(app, 'Editor EOL only applied', timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(CTRL_Z)
            command(app, 'Keep current editor EOL')
            wait_screen(app, 'Editor EOL unchanged', timeout=10)
            app.send(b'\x1b[122;6u' + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == converted)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        print('PASS: EOL-only last request wins; same-EOL success preserves real native Redo and the original CRLF bytes')
    assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())


if __name__ == '__main__':
    run()
