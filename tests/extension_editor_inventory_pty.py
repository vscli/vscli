#!/usr/bin/env python3
"""Actual selected host editor inventory, shared pane views and native histories."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 A\r\nlast\r\n'


def unchanged(root):
    assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode('utf-8')
    assert (root / 'b.txt').read_bytes() == b'other\r\n'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode('utf-8'))
    (root / 'b.txt').write_bytes(b'other\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-inventory-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual editor inventory qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp',
                 enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    return app


def shared_and_replace(app):
    command(app, 'Editor inventory shared panes')
    wait_screen(app, 'API inventory shared;editors=distinct;document=shared;focus=first')
    command(app, 'Editor inventory focus and language')
    wait_screen(app, 'API inventory focus/language;stable=true;visibleEvents=quiet')
    command(app, 'Editor inventory replace second pane')
    wait_screen(app, 'API inventory replaced;first=retained;second=other', 'a.cpp ●', 'b.txt')


def retained(root):
    app = ready(root)
    shared_and_replace(app)
    command(app, 'Editor inventory inactive pane')
    wait_screen(app, 'API inventory inactive;exactPane=true;focus=other')
    command(app, 'Editor inventory retired editor')
    wait_screen(app, 'API inventory retired;replacement=untouched')
    command(app, 'Editor inventory show original')
    wait_screen(app, 'API inventory original;identity=true;dirty=true', 'a.cpp ●')
    command(app, 'Editor inventory arm movement')
    wait_screen(app, 'API inventory native movement armed')
    app.send(b'\x1b[C')
    wait_screen(app, 'API inventory native movement;pane=first;event=true')
    app.send(CTRL_Z)
    command(app, 'Editor inventory original clean')
    wait_screen(app, 'API inventory undo;clean=true', absent=('a.cpp ●',))
    app.send(b'\x1b[122;6u')
    command(app, 'Editor inventory original dirty')
    wait_screen(app, 'API inventory redo;dirty=true', 'a.cpp ●')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: two stable scoped editors share one CRLF model; preserveFocus, original focus, inactive exact-pane setter/native event, retired-editor refusal and shared Undo/Redo preserve disks')


def columns_empty(root):
    app = ready(root)
    shared_and_replace(app)
    command(app, 'Editor inventory show original')
    wait_screen(app, 'API inventory original;identity=true;dirty=true', 'a.cpp ●')
    app.send(CTRL_Z)
    command(app, 'Editor inventory original clean')
    wait_screen(app, 'API inventory undo;clean=true', absent=('a.cpp ●',))
    command(app, 'View: Close Editor Group')
    command(app, 'Editor inventory verify closed group')
    wait_screen(app, 'API inventory close group;survivor=stable;column=1;event=true', 'b.txt')
    command(app, 'File: Close All Editors')
    command(app, 'Editor inventory verify empty')
    wait_screen(app, 'API inventory empty;visible=0;active=undefined')
    unchanged(root)
    app.finish()
    unchanged(root)
    print('PASS: original group close publishes surviving editor column change with stable identity; close-all publishes visible[]/active undefined and preserves clean disk bytes')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-inventory-') as directory:
        try:
            for name, run in [('retained', retained), ('empty', columns_empty)]:
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
                        assert app.process.restored, 'Editor inventory interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
