#!/usr/bin/env python3
"""Real global workspace callbacks, exact lazy resolve and native Ctrl+T."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

SOURCE = '猫🙂 source\r\nnext\r\n'
TARGET = 'α🙂 target\r\n'
HIDDEN = 'β🙂 hidden\r\n'
REDO = b'\x1b[122;6u'


def ready(root, opened=True):
    for name, value in [('source.cpp', SOURCE), ('target.cpp', TARGET), ('hidden.cpp', HIDDEN)]:
        (root / name).write_bytes(value.encode())
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/workspace-symbols-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {}
    }), encoding='utf-8')
    node = shutil.which('node')
    assert node, 'Selected workspace-symbol fixture requires optional Node'
    args = ['--extension', extension, '--extension-node', str(Path(node).resolve()),
            '--config-dir', config, '--no-session']
    if opened:
        args.append(root / 'source.cpp')
    app = Editor(root, *args, enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:')
    if opened:
        wait_screen(app, 'source.cpp  |  cpp')
        app.paste('dirty ')
        verify(app, 'prepare shared targets', 'API workspace prepared;sharedDirty/hidden=true')
    return app


def verify(app, title, receipt):
    command(app, 'Workspace symbols ' + title)
    wait_screen(app, receipt)


def source(app):
    verify(app, 'show source', 'API workspace source;identity/dirty=true')


def start(app, query):
    app.send(b'\x14')  # Original Ctrl+T, not a fixture-only command.
    wait_screen(app, 'Go to Symbol in Workspace', absent=('Command Palette',))
    app.paste(query)


def settled(app, query, label):
    wait_screen(app, 'Go to Symbol in Workspace', label,
                "1 workspace symbols for '" + query + "' · Enter navigates",
                absent=('Command Palette',))


def accepted(app, filename, dirty=False):
    app.send(b'\r')
    wait_screen(app, filename + (' *' if dirty else '') + '  |  cpp',
                absent=('Go to Symbol in Workspace', 'Command Palette'))


def unchanged(root):
    for name, value in [('source.cpp', SOURCE), ('target.cpp', TARGET), ('hidden.cpp', HIDDEN)]:
        assert (root / name).read_bytes() == value.encode()


def history(app):
    source(app)
    app.send(CTRL_Z)
    verify(app, 'verify source Undo', 'API workspace source Undo;exact=true')
    app.send(REDO)
    verify(app, 'verify source Redo', 'API workspace source Redo;exact=true')


def finish(app, dirty=0):
    app.send(CTRL_SHIFT_W)
    for _ in range(dirty):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()


def public(root):
    app = ready(root, False)
    verify(app, 'query zero editors',
           'API workspace public;zeroEditors/fullFields/sort/dedup/isolation=true;resolve=0')
    unchanged(root)
    finish(app)
    print('PASS: actual global public workspace query needs zero editors, preserves fresh exact SymbolInformation and missing range, sorts/deduplicates/isolate errors without eager resolve or document/disk changes')


def native(root):
    app = ready(root)
    start(app, 'lazy')
    settled(app, 'lazy', 'AlphaLazy')
    accepted(app, 'target.cpp', True)
    verify(app, 'verify resolved target', 'API workspace target;originalResolve/collapsed/shared=true')
    app.send(CTRL_Z)
    verify(app, 'verify target Undo', 'API workspace target Undo;exact=true')
    app.send(REDO)
    verify(app, 'verify target Redo', 'API workspace target Redo;exact=true')
    source(app)
    start(app, 'hidden')
    settled(app, 'hidden', 'Hidden')
    accepted(app, 'hidden.cpp')
    verify(app, 'verify hidden target', 'API workspace hidden;identity/collapsed=true')
    history(app)
    unchanged(root)
    finish(app, 2)
    unchanged(root)
    print('PASS: original Ctrl+T resolves exact opaque cyclic list object before collapsed reveal into retained shared dirty and hidden targets; native keyboard histories and Unicode CRLF disks survive')


def held(root):
    app = ready(root)
    verify(app, 'arm held list', 'API workspace held list armed')
    start(app, 'held-list')
    wait_screen(app, 'API workspace held list;entered=true;actualPending=true')
    app.send(CTRL_A)
    app.paste('hidden')
    settled(app, 'hidden', 'Hidden')
    accepted(app, 'hidden.cpp')
    verify(app, 'verify hidden target', 'API workspace hidden;identity/collapsed=true')
    verify(app, 'release actual callback', 'API workspace held list;cancelled/actualSettled=true')
    verify(app, 'verify hidden target', 'API workspace hidden;identity/collapsed=true')
    source(app)
    verify(app, 'arm held resolve', 'API workspace held resolve armed')
    start(app, 'lazy-resolve')
    settled(app, 'lazy-resolve', 'lazy-resolve')
    app.send(b'\r')
    wait_screen(app, 'API workspace held resolve;entered=true;actualPending=true')
    app.send(b'\x1b')
    verify(app, 'release actual callback', 'API workspace held resolve;cancelled/actualSettled=true')
    history(app)
    unchanged(root)
    finish(app, 2)
    unchanged(root)
    print('PASS: latest workspace query opens its actual hidden target while canceled old callback remains held; late list and Esc-canceled resolver settle without reopening UI or losing original histories/disks')


def capacity(root):
    app = ready(root, False)
    verify(app, 'hold capacity', 'API workspace capacity;actualCallbacks=8;held=true')
    verify(app, 'verify capacity overflow', 'API workspace capacity;ninth=refused;actualStill8=true')
    verify(app, 'release capacity', 'API workspace capacity;originals/fresh=true;actualReleased=true')
    unchanged(root)
    finish(app)
    print('PASS: eight global public workspace callbacks retain actual shared capacity; ninth refuses until settlement, then originals and fresh zero-editor query succeed without document or disk effects')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-workspace-symbols-') as directory:
        try:
            for name, run in [('public', public), ('native', native), ('held', held), ('capacity', capacity)]:
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
                        assert app.process.restored, 'Workspace-symbol interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
