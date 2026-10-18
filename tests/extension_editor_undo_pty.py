#!/usr/bin/env python3
"""Real public edit callbacks join native Undo and Redo without changing CRLF bytes."""
from pathlib import Path
import hashlib
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import command
from profile_migration_pty import finish_after_failure


def run():
    fixture = Path(__file__).resolve().parent / 'fixtures/editor-undo-extension'
    originals = {p: hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-editor-undo-') as temporary:
        root = Path(temporary)
        file = root / 'undo.txt'
        original = 'λ🙂base\r\n'.encode()
        final = 'λ🙂baseAB\r\n'.encode()
        file.write_bytes(original)
        app = Editor(root, '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '2 commands', timeout=15)
            command(app, 'Join two API edits')
            wait_screen(app, 'Joined API edits applied', timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == final)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(b'\x1b[122;6u' + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == final)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())
        print('PASS: two public edits join one native Undo/Redo group with exact Unicode/CRLF saves; fixture unchanged')

    with tempfile.TemporaryDirectory(prefix='vscli-editor-undo-key-') as temporary:
        root = Path(temporary)
        file = root / 'undo.txt'
        file.write_bytes(original)
        app = Editor(root, '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '2 commands', timeout=15)
            app.send(b'\x1b[F' + b'P')
            wait_screen(app, 'baseP')
            app.send(b'\x1b[20~')
            wait_screen(app, 'Editor undo edit applied', timeout=10)
            app.send(b'B' + CTRL_S)
            expected = 'λ🙂basePAB\r\n'.encode()
            eventually(lambda: app.read() and file.read_bytes() == expected)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(b'\x1b[122;6u' + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == expected)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())
        print('PASS: contributed F9 and false undo stops join preceding/following native typing; one Undo restores exact original CRLF bytes')


if __name__ == '__main__':
    run()
