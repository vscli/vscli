#!/usr/bin/env python3
"""Preferred extension language, translated module load/command, native editing and restart."""
from pathlib import Path
import hashlib
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def finish_failure(app):
    if app.process.poll() is None:
        app.process.kill()
        app.process.wait()
    app.close_fds()


def run():
    fixture = Path(__file__).resolve().parent / 'fixtures/localization-extension'
    original_files = {path: hashlib.sha256(path.read_bytes()).digest() for path in fixture.rglob('*') if path.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-l10n-pty-') as temporary:
        root = Path(temporary)
        file = root / 'native.txt'
        original = 'λ🙂 original\r\n'.encode()
        file.write_bytes(original)
        app = Editor(root, '--config-dir', root / 'native', '--locale', 'fr', '--extension-node', root / 'no-node', file, enhanced=True)
        try:
            app.send('N')
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == b'N' + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_failure(app)
            raise
        print('PASS: preferred locale does not start optional JavaScript; native Unicode/CRLF edit/save/Undo without Node')
        app = Editor(root, '--config-dir', root / 'native', '--locale', 'fr', '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '1 commands', timeout=15)
            for restart in [False, True]:
                if restart:
                    app.send(b'\x1bOP')
                    app.send('Extensions: Restart Selected Session\r')
                    wait_screen(app, '1 commands', timeout=15)
                app.send(b'\x1b[20~')
                wait_screen(app, 'Prêt/Appliqué true;language=fr')
                app.send(CTRL_S)
                eventually(lambda: app.read() and file.read_bytes() == 'Bonjour 猫🙂 '.encode() + original)
                app.send(CTRL_Z)
                app.send(CTRL_S)
                eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_failure(app)
            raise
        for path, digest in original_files.items():
            assert hashlib.sha256(path.read_bytes()).digest() == digest, path
        print('PASS: CLI preferred language, French bundle/URI and module evaluation, localized native command, Unicode/CRLF save/Undo and restart preserve source files')


if __name__ == '__main__':
    run()
