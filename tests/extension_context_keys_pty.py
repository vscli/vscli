#!/usr/bin/env python3
"""Actual extension setContext controls contributed keys without editing native history."""
from pathlib import Path
import hashlib
import tempfile
from pty_smoke import CTRL_S,CTRL_Z,Editor,eventually,wait_screen
from profile_migration_pty import finish_after_failure


def run():
    fixture=Path(__file__).resolve().parent/'fixtures/context-keys-extension'
    originals={p:hashlib.sha256(p.read_bytes()).digest() for p in fixture.rglob('*') if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-context-pty-') as temporary:
        root=Path(temporary);file=root/'native.txt';original='λ🙂 original\r\n'.encode();file.write_bytes(original)
        marker=root/'context-entered'
        app=Editor(root,'--extension',fixture,file,enhanced=True)
        try:
            wait_screen(app,'3 commands',timeout=15)
            app.send(b'\x1b[19~')
            wait_screen(app,'Context edit applied and shortcut disabled',timeout=10)
            app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'context '+original)
            assert marker.read_text().splitlines()==['edit']
            app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original)
            # The next real host command is an acknowledgement barrier after the
            # inactive key. No output-state retry can hide an extra invocation.
            app.send(b'\x1b[19~');app.send(b'\x1b[18~')
            wait_screen(app,'Context shortcut enabled',timeout=10)
            assert marker.read_text().splitlines()==['edit']
            assert file.read_bytes()==original
            app.send(b'\x1b[19~');wait_screen(app,'Context edit applied and shortcut disabled',timeout=10)
            app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'context '+original)
            assert marker.read_text().splitlines()==['edit','edit']
            app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original)
            app.finish()
        except BaseException:
            finish_after_failure(app);raise
        for path,digest in originals.items():assert hashlib.sha256(path.read_bytes()).digest()==digest
        print('PASS: module/activation setContext → conditional contributed shortcut → native edit/save/Undo → disabled key does not invoke owner → public re-enable → repeat edit/save/Undo; original package unchanged')


if __name__=='__main__':
    run()
