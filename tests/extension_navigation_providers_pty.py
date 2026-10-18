#!/usr/bin/env python3
"""Actual navigation callbacks/public queries, original native pickers and retained data."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, CTRL_SHIFT_W, eventually, wait_screen

ORIGINAL = '猫🙂 source\r\nnext\r\n'
ONE = 'α🙂 targetA\r\n'
REDO = b'\x1b[122;6u'
QUERIES = 'API nav queries;five/fullLinks/sort/dedup=true;inactive/virtual/language/isolation=true'


def ready(root):
    for name, text in [('source.cpp', ORIGINAL), ('one.cpp', ONE), ('two.cpp', 'β🙂 targetB\r\n'),
                       ('three.cpp', 'γ🙂 targetC\r\n'), ('held.hold', 'hold\r\n')]:
        (root / name).write_bytes(text.encode())
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/navigation-providers-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({'vscli.languageServer.enabled': False,
        'breadcrumbs.enabled': False, 'files.autoSave': 'off', 'editor.formatOnSave': False,
        'editor.codeActionsOnSave': {}}), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual navigation provider qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'source.cpp', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'source.cpp')
    app.paste('dirty ')
    return app


def verify(app, query, receipt):
    command(app, query)
    wait_screen(app, receipt)


def unchanged(root, deleted=False):
    assert (root / 'source.cpp').read_bytes() == ORIGINAL.encode()
    if deleted:
        assert not (root / 'one.cpp').exists()
    else:
        assert (root / 'one.cpp').read_bytes() == ONE.encode()
    assert (root / 'two.cpp').read_bytes() == 'β🙂 targetB\r\n'.encode()
    assert (root / 'three.cpp').read_bytes() == 'γ🙂 targetC\r\n'.encode()


def source(app):
    verify(app, 'Navigation providers show source', 'API navigation source;identity=true;dirty=true')


def original_history(app):
    source(app)
    app.send(CTRL_Z)
    verify(app, 'Navigation providers verify original clean', 'API navigation original Undo;clean=true')
    app.send(REDO)
    verify(app, 'Navigation providers verify original dirty', 'API navigation original Redo;dirty=true')


def finish(app, count=1):
    app.send(CTRL_SHIFT_W)
    for _ in range(count):
        wait_screen(app, 'Unsaved Changes')
        app.send(b'd')
    eventually(lambda: app.read() and app.process.poll() is not None)
    app.finish()


def native(root):
    app = ready(root)
    verify(app, 'Navigation providers prepare shared target', 'API navigation target;retained/shared=true;dirty=true')
    (root / 'one.cpp').unlink()
    # This real watcher publication changes the retained target's baseline.
    # Complete setup before capturing the first asynchronous navigation context.
    wait_screen(app, 'one.cpp was removed from disk; buffer retained. Save As to preserve it')
    # Sorted source aggregation gives one.cpp as the first target. Every action
    # is issued once; Enter accepts the actual native location picker.
    for title in ['Language: Go to Definition', 'Language: Find References',
                  'Language: Go to Implementation', 'Language: Go to Type Definition',
                  'Language: Go to Declaration']:
        source(app)
        command(app, title)
        # Explorer already contains these resource names. Require the actual
        # provider-location modal before Enter, so it cannot edit the source.
        wait_screen(app, 'Extension Locations · Enter opens · Esc closes',
                    'one.cpp', 'two.cpp', 'three.cpp', absent=('Command Palette',))
        app.send(b'\r')
        verify(app, 'Navigation providers verify collapsed reveal',
               'API navigation reveal;selectionStart=7;collapsed=true;identity=true')
    app.send(CTRL_Z)
    verify(app, 'Navigation providers verify target undo', 'API navigation target Undo;exactSource=true')
    app.send(REDO)
    verify(app, 'Navigation providers verify target redo', 'API navigation target Redo;exactSource=true')
    original_history(app)
    unchanged(root, deleted=True)
    finish(app, 2)
    unchanged(root, deleted=True)
    print('PASS: all five original navigation commands aggregate actual providers; native picker reveals collapsed selection start in deleted retained dirty target while source/shared histories and disks survive')


def queries(root):
    app = ready(root)
    verify(app, 'Navigation providers public queries', QUERIES)
    original_history(app)
    unchanged(root)
    finish(app)
    unchanged(root)
    print('PASS: five public execute queries preserve full Location/Link fields, sorting/dedup/provider isolation, inactive focus and effective-language/virtual sources without text or disk changes')


def held(root):
    app = ready(root)
    verify(app, 'Navigation providers arm held callback', 'API navigation held armed')
    command(app, 'Language: Go to Definition')
    wait_screen(app, 'API navigation held;entered=true;source=')
    app.paste('X')
    app.send(CTRL_Z)
    verify(app, 'Navigation providers release held callback', 'API navigation held released;actualCallback=true')
    source(app)
    app.send(REDO)
    verify(app, 'Navigation providers verify held redo', 'API navigation held Redo;intentionalEdit=true;lateReply=false')
    app.send(CTRL_Z)
    source(app)
    verify(app, 'Navigation providers arm held callback', 'API navigation held armed')
    command(app, 'Language: Go to Definition')
    wait_screen(app, 'API navigation held;entered=true;source=')
    verify(app, 'Navigation providers language ABA', 'API navigation language ABA;source=unchanged')
    verify(app, 'Navigation providers release held callback', 'API navigation held released;actualCallback=true')
    original_history(app)
    unchanged(root)
    finish(app)
    unchanged(root)
    print('PASS: ignored-cancel real callback releases only after settlement; native edit+Undo/language ABA reject its late locations and preserve intentional Redo, original history and protected bytes')


def capacity(root):
    app = ready(root)
    verify(app, 'Navigation providers hold capacity', 'API navigation capacity;actualCallbacks=8;held=true')
    verify(app, 'Navigation providers verify capacity overflow', 'API navigation capacity;ninth=refused;actualStill8=true')
    verify(app, 'Navigation providers release capacity', 'API navigation capacity;originals/fresh=true;actualReleased=true')
    original_history(app)
    unchanged(root)
    finish(app)
    unchanged(root)
    print('PASS: eight actual public navigation callbacks retain shared capacity; ninth refuses, original callbacks settle and fresh query works with no native focus/source/Undo/disk loss')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-navigation-provider-') as directory:
        try:
            for name, run in [('native', native), ('queries', queries), ('held', held), ('capacity', capacity)]:
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
                        assert app.process.restored, 'Navigation interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
