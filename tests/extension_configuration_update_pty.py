#!/usr/bin/env python3
"""Real extension configuration writes, native effects and dirty-buffer refusal."""
from pathlib import Path
import hashlib
import json
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import command
from profile_migration_pty import finish_after_failure


def run():
    fixture = Path(__file__).resolve().parent / 'fixtures/configuration-update-extension'
    originals = {p: hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-configuration-update-') as temporary:
        root = Path(temporary)
        config = root / 'profile'
        config.mkdir()
        user = config / 'settings.json'
        user.write_bytes('{\r\n // user 猫🙂\r\n "fixture.value": "before",\r\n "keep.root": [1,2]\r\n}\r\n'.encode())
        workspace = root / '.vscode'
        workspace.mkdir()
        settings = workspace / 'settings.json'
        settings.write_bytes('{\r\n // workspace 猫🙂\r\n "[cpp]": {"fixture.value": false, "keep.language": 7}\r\n}\r\n'.encode())
        source = root / 'source-settings.json'
        source.write_bytes(b'source preserved\r\n')
        file = root / 'main.cpp'
        original = 'λ🙂 original\r\nsecond\r\n'.encode()
        file.write_bytes(original)
        app = Editor(root, '--config-dir', config, '--extension', fixture, file, enhanced=True)
        try:
            wait_screen(app, '5 commands', timeout=15)
            app.send('dirty ')
            command(app, 'Write extension configuration')
            wait_screen(app, 'Configuration roundtrip persisted', timeout=15)
            assert b'// user ' in user.read_bytes() and b'"fixture.value": null' in user.read_bytes()
            assert b'"keep.root": [1,2]' in user.read_bytes()
            command(app, 'Write language configuration')
            wait_screen(app, 'Language configuration persisted', timeout=15)
            assert b'// workspace ' in settings.read_bytes()
            assert b'"keep.language": 7' in settings.read_bytes()
            assert b'"editor.tabSize": 3' in settings.read_bytes()
            app.send(CTRL_S)
            dirty = b'dirty ' + original
            eventually(lambda: app.read() and file.read_bytes() == dirty)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.send(b'\x1b[122;6u' + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == dirty)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        app = Editor(root, '--config-dir', config, '--extension-node', root / 'missing-node', file,
                     enhanced=True, extra_env={'PATH': ''})
        try:
            app.send(b'\t' + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == b'   ' + dirty)
            app.send(CTRL_Z + CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == dirty)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert source.read_bytes() == b'source preserved\r\n'
        assert not list(root.rglob('.vscli-write.lock'))
        print('PASS: public configuration values/removal/events precede void; native language effect, Unicode CRLF Undo/Redo, no-Node restart and no lock sidecars')
    with tempfile.TemporaryDirectory(prefix='vscli-configuration-dirty-') as temporary:
        root = Path(temporary)
        config = root / 'profile'
        config.mkdir()
        user = config / 'settings.json'
        original = b'{"fixture.value":"before"}\r\n'
        user.write_bytes(original)
        app = Editor(root, '--config-dir', config, '--extension', fixture, user, enhanced=True)
        try:
            wait_screen(app, '5 commands', timeout=15)
            app.send('unsaved ')
            command(app, 'Refuse dirty configuration')
            wait_screen(app, 'Dirty configuration preserved', timeout=15)
            assert user.read_bytes() == original
            wait_screen(app, 'unsaved ')
            app.send(CTRL_Z)
            wait_screen(app, absent=('unsaved ',))
            assert user.read_bytes() == original
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        print('PASS: dirty settings refuse extension persistence while exact disk bytes and native Undo remain intact')
    with tempfile.TemporaryDirectory(prefix='vscli-configuration-clean-') as temporary:
        root = Path(temporary)
        config = root / 'profile'
        config.mkdir()
        user = config / 'settings.json'
        original = b'{"fixture.value":"before"}\r\n'
        user.write_bytes(original)
        app = Editor(root, '--config-dir', config, '--extension', fixture, user, enhanced=True)
        try:
            wait_screen(app, '5 commands', timeout=15)
            command(app, 'Refresh open configuration')
            wait_screen(app, 'Open configuration refreshed', 'after-clean', timeout=15)
            updated = user.read_bytes()
            assert b'after-clean' in updated and updated.endswith(b'\r\n')
            app.send(CTRL_Z)
            wait_screen(app, 'before', absent=('after-clean',))
            assert user.read_bytes() == updated
            app.send(b'\x1b[122;6u')
            wait_screen(app, 'after-clean')
            assert user.read_bytes() == updated
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        print('PASS: open settings text and change event precede public success; native Undo/Redo retains exact persisted CRLF bytes')
    assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())


if __name__ == '__main__':
    run()
