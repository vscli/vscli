#!/usr/bin/env python3
"""Public native decorations lifecycle with real terminal editing and shared history."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 alpha\r\nsecond beta\r\n'


def unchanged(root):
    assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode('utf-8')
    assert (root / 'b.txt').read_bytes() == b'other\r\n'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode('utf-8'))
    (root / 'b.txt').write_bytes(b'other\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-decorations-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual native decoration qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp',
                 enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    command(app, 'Editor decorations shared styles')
    wait_screen(app, 'API decorations shared;views=independent;source=unchanged')
    return app


def lifecycle(root):
    app = ready(root)
    command(app, 'Editor decorations replace range')
    wait_screen(app, 'API decorations replacement;oldRange=removed;second=retained')
    command(app, 'Editor decorations clear first')
    wait_screen(app, 'API decorations cleared;first=empty;second=retained')
    command(app, 'Editor decorations dispose types')
    wait_screen(app, 'API decorations disposed;idempotent=true;forgery=refused')
    app.send(CTRL_Z)
    command(app, 'Editor decorations verify clean history')
    wait_screen(app, 'API decorations original undo;clean=true', absent=('a.cpp *',))
    app.send(b'\x1b[122;6u')
    command(app, 'Editor decorations verify dirty history')
    wait_screen(app, 'API decorations undo;dirtyBaseline=preserved', 'a.cpp *')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: actual public per-editor Range/DecorationOptions updates replace/clear/dispose and reject forged/disposed types without editing shared CRLF source or adding Undo entries; native buffer tests qualify styles separately')


def edits_retirement(root):
    app = ready(root)
    command(app, 'Editor decorations arm native start edit')
    wait_screen(app, 'API decorations native start armed')
    app.paste('Z')
    command(app, 'Editor decorations verify insertion')
    wait_screen(app, 'API decorations native insertion;source=true;shared=true')
    app.send(CTRL_Z)
    command(app, 'Editor decorations verify dirty history')
    wait_screen(app, 'API decorations undo;dirtyBaseline=preserved')
    app.send(b'\x1b[122;6u')
    command(app, 'Editor decorations verify insertion')
    wait_screen(app, 'API decorations native insertion;source=true;shared=true')
    app.send(CTRL_Z)
    command(app, 'Editor decorations verify dirty history')
    wait_screen(app, 'API decorations undo;dirtyBaseline=preserved')
    command(app, 'Editor decorations forced API insert')
    wait_screen(app, 'API decorations forced insert;edit=true;shared=true')
    app.send(CTRL_Z)
    command(app, 'Editor decorations verify dirty history')
    wait_screen(app, 'API decorations undo;dirtyBaseline=preserved')
    command(app, 'Editor decorations reentrant source edit')
    wait_screen(app, 'API decorations reentrant edit;staleRequest=issued;edit=true')
    app.send(CTRL_Z)
    command(app, 'Editor decorations reentrant undo')
    wait_screen(app, 'API decorations reentrant undo;original=retained')
    command(app, 'Editor decorations retire second editor')
    wait_screen(app, 'API decorations retired;setter=refused;replacement=untouched')
    command(app, 'Extensions: Stop Host')
    wait_screen(app, 'Extension host stopped')
    app.send(CTRL_Z)
    wait_screen(app, '猫🙂 alpha', absent=('dirty 猫🙂 alpha', 'a.cpp *'))
    app.send(b'\x1b[122;6u')
    wait_screen(app, 'dirty 猫🙂 alpha', 'a.cpp *')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: actual native insert/Undo/Redo, forced public insertion, reentrant API edit and explicit pane/host retirement preserve dirty shared CRLF work and disks; native buffer tests qualify sticky and stale styles separately')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-decorations-') as directory:
        try:
            for name, run in [('lifecycle', lifecycle), ('edits', edits_retirement)]:
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
                        assert app.process.restored, 'Editor decoration interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
