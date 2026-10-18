#!/usr/bin/env python3
"""Native complex import shortcuts work without JavaScript and with public context updates."""
from pathlib import Path
import json,tempfile
from pty_smoke import Editor,CTRL_S,CTRL_Z,eventually,wait_screen
from profile_migration_pty import finish_after_failure

def run():
    with tempfile.TemporaryDirectory(prefix='vscli-when-pty-') as temporary:
        root=Path(temporary);file=root/'native.txt';original='λ🙂 original\r\n'.encode();file.write_bytes(original)
        bindings=root/'keybindings.json'
        bindings.write_text(json.dumps([{'key':'f6','command':'type','args':{'text':'native '},'when':'editorTextFocus && workspaceFolderCount >= 1 && resourceExtname =~ /^\\.txt$/i && resourceScheme == file'}]))
        app=Editor(root,'--extension-node',root/'no-node','--keybindings',bindings,file,enhanced=True)
        try:
            app.send(b'\x1b[17~');app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'native '+original)
            app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        print('PASS: no-Node cached regex/numeric/resource conditions → actual native shortcut → Unicode/CRLF save/Undo')
        fixture=Path(__file__).resolve().parent/'fixtures/context-keys-extension'
        bindings.write_text(json.dumps([{'key':'f9','command':'fixture.context.edit','when':"editorTextFocus && fixture.enabled && !fixture.zero && resourceLangId in fixture.modes && fixture.count >= 2 && fixture.name =~ /^rust$/i"}]))
        app=Editor(root,'--extension',fixture,'--keybindings',bindings,file,enhanced=True)
        try:
            wait_screen(app,'3 commands',timeout=15)
            app.send(b'\x1b[20~');wait_screen(app,'Context edit applied and shortcut disabled',timeout=10)
            app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'context '+original)
            app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        print('PASS: public setContext arrays/count/string/zero → imported membership/numeric/regex condition → native edit/save/Undo')

if __name__=='__main__':run()
