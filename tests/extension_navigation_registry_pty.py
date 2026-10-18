#!/usr/bin/env python3
"""Selected public/native queries share one stateful exclusive registry."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

SOURCE = '猫🙂 source\r\nnext\r\n'
TARGET = 'α🙂 target\r\n'
REDO = b'\x1b[122;6u'


def verify(app, title, receipt):
    command(app, 'Navigation registry ' + title)
    wait_screen(app, receipt)


def source(app):
    verify(app, 'show source', 'API registry source;retained=true')


def query(app, candidate, winner):
    verify(app, 'query ' + candidate + ' ' + ('none' if winner == 'null' else winner),
           'API registry query;' + candidate + '=' + winner + ';inactive/noEdit=true')


def native(app, winner):
    source(app)
    command(app, 'Language: Go to Definition')
    # Resource names also exist in Explorer and inactive tab strips. The actual
    # active-document status is the independent native reveal barrier.
    wait_screen(app, winner + '.cpp' + (' *' if winner == 'wildcard' else '') + '  |  cpp',
                absent=('Command Palette', 'Extension Locations'))
    verify(app, 'verify native ' + winner,
           'API registry native;winner=' + winner + ';collapsed/shared=true')


def unchanged(root):
    assert (root / 'source.cpp').read_bytes() == SOURCE.encode()
    assert (root / 'candidate.rs').read_bytes() == '猫🙂 rust\r\n'.encode()
    for name in ['early', 'wildcard', 'third']:
        assert (root / (name + '.cpp')).read_bytes() == TARGET.encode()


def run(root):
    (root / 'source.cpp').write_bytes(SOURCE.encode())
    (root / 'candidate.rs').write_bytes('猫🙂 rust\r\n'.encode())
    for name in ['early', 'wildcard', 'third']:
        (root / (name + '.cpp')).write_bytes(TARGET.encode())
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/navigation-registry-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {}
    }), encoding='utf-8')
    node = shutil.which('node')
    assert node, 'Selected navigation registry fixture requires optional Node'
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'source.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'source.cpp')
    app.paste('dirty ')
    verify(app, 'prepare retained targets', 'API registry prepared;shared/dirty=true')

    query(app, 'cpp', 'early')
    native(app, 'early')
    # Public query A changes the shared registry's ordered vector; native B
    # must select wildcard despite its lower cpp score.
    query(app, 'rust', 'wildcard')
    native(app, 'wildcard')
    query(app, 'cpp', 'wildcard')
    native(app, 'wildcard')
    app.send(CTRL_Z)
    verify(app, 'verify target Undo', 'API registry target Undo;exact=true')
    app.send(REDO)
    verify(app, 'verify target Redo', 'API registry target Redo;exact=true')

    verify(app, 'register third', 'API registry third registered;source=retained')
    query(app, 'cpp', 'wildcard')
    native(app, 'wildcard')
    verify(app, 'dispose wildcard', 'API registry wildcard disposed;source=retained')
    native(app, 'third')
    query(app, 'cpp', 'third')
    query(app, 'rust', 'null')
    native(app, 'third')
    verify(app, 'dispose third', 'API registry third disposed;source=retained')
    native(app, 'early')
    query(app, 'cpp', 'early')

    source(app)
    app.send(CTRL_Z)
    verify(app, 'verify source Undo', 'API registry source Undo;exact=true')
    app.send(REDO)
    verify(app, 'verify source Redo', 'API registry source Redo;exact=true')
    unchanged(root)
    app.send(CTRL_SHIFT_W)
    for _ in range(2):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()
    unchanged(root)
    print('PASS: public rust and native cpp queries share pinned evolving exclusive registry order through cached query, registration and disposal; native collapsed reveals and keyboard shared-target/source UndoRedo preserve Unicode CRLF and all disks with empty PATH')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-navigation-registry-') as directory:
        try:
            run(Path(directory).resolve())
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Navigation registry interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
