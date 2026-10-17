#!/usr/bin/env python3
"""Public tab inventory/close promises drive actual native prompts and saved receipts."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

ORIGINAL = '猫🙂 tabs\r\nlast\r\n'
DIRTY = 'dirty ' + ORIGINAL
UNTITLED = '猫🙂 new\r\ntail\r\n'
REDO = b'\x1b[122;6u'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode())
    (root / 'b.txt').write_bytes(b'other\r\n')
    (root / 'c.txt').write_bytes(b'third\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-tabs-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual public tab qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    return app


def verify(app, query, receipt):
    command(app, query)
    wait_screen(app, receipt)


def disks(root, a=ORIGINAL):
    assert (root / 'a.cpp').read_bytes() == a.encode()
    assert (root / 'b.txt').read_bytes() == b'other\r\n'
    assert (root / 'c.txt').read_bytes() == b'third\r\n'


def inventory(root):
    app = ready(root)
    verify(app, 'Editor tabs setup inventory', 'API tabs inventory;readonly=true;inputs=7;sharedMemberships=true;events=true')
    command(app, 'View: Pin Editor')
    verify(app, 'Editor tabs verify pinned', 'API tabs pinned;sticky=true;preview=false')
    command(app, 'View: Unpin Editor')
    verify(app, 'Editor tabs verify unpinned', 'API tabs unpinned;committed=true')
    app.send(CTRL_Z)
    verify(app, 'Editor tabs verify clean history', 'API tabs clean history;identities=true')
    app.send(REDO)
    verify(app, 'Editor tabs verify dirty shared history', 'API tabs dirty shared history;identities=true')
    disks(root)
    app.finish(discard=True)
    disks(root)
    print('PASS: public frozen live groups/tabs and seven input classes retain shared-file membership identities, real sticky state and native CRLF Undo/Redo without disk writes')


def clean(root):
    app = ready(root)
    verify(app, 'Editor tabs setup inventory', 'API tabs inventory;readonly=true;inputs=7;sharedMemberships=true;events=true')
    verify(app, 'Editor tabs close clean array group', 'API tabs close clean;array/group=true;arrayPreserveFocus=true;retired/fake=refused;empty=true')
    app.send(CTRL_Z)
    verify(app, 'Editor tabs verify clean history', 'API tabs clean history;identities=true')
    verify(app, 'Editor tabs close welcome', 'API tabs welcome;oneEmptyGroup=true;activeTab=undefined;editors=0')
    disks(root)
    app.finish()
    disks(root)
    print('PASS: clean inactive tab arrays and shared group close publish events before true; tab preserveFocus, retired/forged refusal, empty close and one actual empty welcome group')


def peer_history(app):
    verify(app, 'Editor tabs show protected peer', 'API tabs peer active;identity=true')
    app.send(CTRL_Z)
    verify(app, 'Editor tabs verify peer clean', 'API tabs peer Undo;clean=true')
    app.send(REDO)
    verify(app, 'Editor tabs verify peer dirty', 'API tabs peer Redo;dirty=true')


def dirty(root):
    app = ready(root)
    verify(app, 'Editor tabs prepare protected peer', 'API tabs protected peer;dirty=true;origin=inactive')
    command(app, 'Editor tabs close dirty cancel')
    wait_screen(app, 'Unsaved Changes')
    app.send(b'\x1b')
    wait_screen(app, 'API tabs dirty Cancel=false;source/history=retained;peer=unchanged')
    verify(app, 'Editor tabs restore original', 'API tabs original retained;identity/history=true')
    app.send(CTRL_Z)
    verify(app, 'Editor tabs verify clean history', 'API tabs clean history;identities=true')
    app.send(REDO)
    verify(app, 'Editor tabs verify original dirty', 'API tabs original Redo;dirty=true')
    verify(app, 'Editor tabs show protected peer', 'API tabs peer active;identity=true')
    command(app, 'Editor tabs close dirty discard')
    wait_screen(app, 'Unsaved Changes')
    app.send(b'd')
    wait_screen(app, 'API tabs dirty Discard=true;events=true;peer=unchanged')
    verify(app, 'Editor tabs reopen discarded', 'API tabs discarded reopened;diskBaseline=true;peer=unchanged')
    peer_history(app)
    disks(root)
    app.finish(discard=True)
    disks(root)
    print('PASS: actual dirty tab Cancel retains source/caret/history; Discard leaves exact disk baseline and independent dirty peer identity/Undo/Redo untouched')


def save(root):
    app = ready(root)
    verify(app, 'Editor tabs prepare protected peer', 'API tabs protected peer;dirty=true;origin=inactive')
    command(app, 'Editor tabs close dirty save')
    wait_screen(app, 'Unsaved Changes')
    app.send(b's')
    wait_screen(app, 'API tabs dirty Save=true;receipt/event=true;peer=unchanged')
    disks(root, DIRTY)
    verify(app, 'Editor tabs prepare untitled', 'API tabs untitled ready;dirty=true;identity=true')
    command(app, 'Editor tabs close untitled cancel')
    wait_screen(app, 'Unsaved Changes')
    app.send(b's')
    wait_screen(app, 'Save As (existing files are protected)')
    app.send(b'\x1b')
    wait_screen(app, 'API tabs untitled SaveAs Cancel=false;identity/source=true')
    assert not (root / 'tabs-saved.cpp').exists()
    command(app, 'Editor tabs close untitled save')
    wait_screen(app, 'Unsaved Changes')
    app.send(b's')
    wait_screen(app, 'Save As (existing files are protected)')
    app.send(CTRL_A)
    app.paste(str(root / 'tabs-saved.cpp'))
    app.send(b'\r')
    wait_screen(app, 'API tabs untitled SaveAs=true;close/open/save=true;peer=unchanged')
    assert (root / 'tabs-saved.cpp').read_bytes() == UNTITLED.encode()
    peer_history(app)
    disks(root, DIRTY)
    app.finish(discard=True)
    disks(root, DIRTY)
    assert (root / 'tabs-saved.cpp').read_bytes() == UNTITLED.encode()
    print('PASS: tab Save-close waits actual saved receipt/events; untitled SaveAs Cancel preserves identity and accepted new file lifecycle/bytes while peer history survives')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-tabs-') as directory:
        try:
            for name, run in [('inventory', inventory), ('clean', clean), ('dirty', dirty), ('save', save)]:
                root = Path(directory, name).resolve()
                root.mkdir()
                run(root)
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Tabs interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
