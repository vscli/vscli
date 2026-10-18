#!/usr/bin/env python3
"""Default manifest labels reach actual native theme/command pickers and editing."""
from pathlib import Path
import hashlib,json,subprocess,tempfile,zipfile
from pty_smoke import BINARY,CTRL_S,CTRL_Z,Editor,eventually,wait_screen
from imported_appearance_pty import command,painted
from profile_migration_pty import finish_after_failure

def run():
    fixture=Path(__file__).resolve().parent/'fixtures/default-manifest-extension'
    originals={p:hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='vscli-default-nls-pty-') as temporary:
        root=Path(temporary);store=root/'extensions';config=root/'native';config.mkdir();file=root/'native.txt';original='λ🙂 original\r\n'.encode();file.write_bytes(original)
        archive=root/'fixture.vsix'
        with zipfile.ZipFile(archive,'w') as out:
            for path in originals:out.write(path,'extension/'+path.name)
        subprocess.run([BINARY,'--install-extension',str(archive),'--extensions-dir',str(store)],check=True,capture_output=True,timeout=15)
        options=['--extensions-dir',store,'--config-dir',config,'--extension-node',root/'no-node',file]
        app=Editor(root,*options,enhanced=True)
        try:
            command(app,'Preferences: Color Theme');wait_screen(app,'Default translated theme',timeout=10);assert '%themeLabel%' not in app.screen.text();app.send('Default translated theme\r');eventually(lambda:painted(app,(241,226,211)))
            app.send('native ');app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'native '+original);app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        app=Editor(root,*options,enhanced=True)
        try:eventually(lambda:painted(app,(241,226,211)));app.finish()
        except BaseException:finish_after_failure(app);raise
        print('PASS: no-Node installed default label → native theme picker/colors → Unicode/CRLF edit/save/Undo → persistent theme restart')
        app=Editor(root,'--extension',fixture,file,enhanced=True)
        try:
            wait_screen(app,'1 commands',timeout=15);command(app,'Apply localized edit');wait_screen(app,'Default manifest edit applied',timeout=10);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'localized '+original);app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        assert all(hashlib.sha256(p.read_bytes()).digest()==digest for p,digest in originals.items())
        print('PASS: upstream-normalized public packageJSON and command picker → native edit/save/Undo; original package/default translations unchanged')

if __name__=='__main__':run()
