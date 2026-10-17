#!/usr/bin/env python3
"""Real provider virtual URI open/refresh/read-only + ordinary native history."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_S, CTRL_Z, wait_screen

ORIGINAL='猫🙂 A\r\nlast\r\n'


def unchanged(root):
    assert (root/'a.txt').read_bytes()==ORIGINAL.encode('utf-8')
    assert not (root/'sample.cpp').exists(), 'Virtual URI was persisted as a file'


def run(root):
    (root/'a.txt').write_bytes(ORIGINAL.encode('utf-8'))
    extension=root/'extension'
    shutil.copytree(Path(__file__).parent/'fixtures/virtual-document-extension',extension)
    config=root/'config';config.mkdir()
    (config/'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled':False,'breadcrumbs.enabled':False,'files.autoSave':'off',
        'editor.formatOnSave':False,'editor.codeActionsOnSave':{},
    }),encoding='utf-8')
    node=shutil.which('node')
    if not node:
        raise AssertionError('Real virtual content provider qualification requires Node')
    app=Editor(root,'--extension',extension,'--extension-node',str(Path(node).resolve()),
               '--config-dir',config,'--no-session',root/'a.txt',enhanced=True,extra_env={'PATH':''})
    wait_screen(app,'Extension ready:','a.txt');app.paste('dirty ')
    command(app,'Virtual document open');wait_screen(app,'API virtual opened;identity=true;LF=true','sample.cpp')
    app.send('x');app.paste('forbidden 猫🙂\r\n');app.send(CTRL_S)
    # The verification command asserts source/version remained exact and both public edits/save refused.
    command(app,'Virtual document verify readonly')
    wait_screen(app,'API virtual readonly;edit=false;save=false',absent=('Save As (existing files are protected)',))
    unchanged(root)
    command(app,'Virtual document refresh');wait_screen(app,'API virtual refreshed;identity=true;event=true')
    command(app,'Virtual document hold refresh');wait_screen(app,'API virtual obsolete callback held')
    command(app,'Virtual document latest refresh');wait_screen(app,'API virtual latest wins;identity=true')
    command(app,'Virtual document dispose provider');wait_screen(app,'API virtual disposed;cached=true;readonly=true')
    unchanged(root)
    command(app,'Virtual document show original');wait_screen(app,'API original restored;dirty=true;identity=true','a.txt *')
    app.send(CTRL_Z);command(app,'Virtual document verify original clean');wait_screen(app,'API original clean history retained',absent=('a.txt *',))
    unchanged(root)
    app.send(b'\x1b[122;6u');command(app,'Virtual document verify original dirty');wait_screen(app,'API original dirty history retained','a.txt *')
    unchanged(root);app.finish(discard=True);unchanged(root)
    print('PASS: real provider virtual URI mirrors/open/refresh/latest/disposal remain read-only, '
          'preserving ordinary shared dirty identity and original Unicode CRLF Undo/Redo/disks')


if __name__=='__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-virtual-api-') as directory:
        try:
            run(Path(directory).resolve())
        finally:
            failed=sys.exc_info()[0] is not None
            errors=[]
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored,'Virtual document interruption leaked terminal mode'
                        app.close_fds();LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
