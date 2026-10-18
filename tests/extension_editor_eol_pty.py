#!/usr/bin/env python3
"""Actual public editor edits normalize delimiters before native save/Undo."""
from pathlib import Path
import hashlib,tempfile
from pty_smoke import CTRL_S,CTRL_Z,Editor,eventually,wait_screen
from imported_appearance_pty import command
from profile_migration_pty import finish_after_failure

def run():
    fixture=Path(__file__).resolve().parent/'fixtures/editor-eol-extension';originals={p:hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-editor-eol-pty-') as temporary:
        root=Path(temporary);file=root/'native.txt';original='λ🙂 original\r\n'.encode();file.write_bytes(original)
        app=Editor(root,'--extension',fixture,file,enhanced=True)
        try:
            wait_screen(app,'1 commands',timeout=15);command(app,'Normalize multiline edit');wait_screen(app,'Multiline editor edit normalized',timeout=10);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()=='α\r\nβ\r\nγ\r\n'.encode()+original)
            app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        assert all(hashlib.sha256(p.read_bytes()).digest()==digest for p,digest in originals.items())
        print('PASS: public multiline LF/CR/CRLF insert → native original-EOL bytes → actual save → grouped Undo/save restores Unicode/CRLF original; fixture unchanged')

if __name__=='__main__':run()
