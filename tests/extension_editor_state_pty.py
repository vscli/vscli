#!/usr/bin/env python3
"""Real selected extension language/selection events and native CRLF history."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_Z, wait_screen

ORIGINAL = '猫🙂 A\r\nlast\r\n'


def unchanged(root):
    assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode('utf-8')
    assert (root / 'b.txt').read_bytes() == b'other\r\n'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode('utf-8'))
    (root / 'b.txt').write_bytes(b'other\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-state-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual editor-state gateway qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp',
                 enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    return app


def history(app, root):
    command(app, 'Editor state show original')
    wait_screen(app, 'API editor state original;identity=true;dirty=true', 'a.cpp *')
    app.send(CTRL_Z)
    command(app, 'Editor state verify original clean')
    wait_screen(app, 'API editor state original undo;clean=true', absent=('a.cpp *',))
    app.send(b'\x1b[122;6u')
    command(app, 'Editor state verify original dirty')
    wait_screen(app, 'API editor state original redo;dirty=true', 'a.cpp *')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)


def state(root):
    app = ready(root)
    command(app, 'Editor state language lifecycle')
    wait_screen(app, 'API language lifecycle;identity=true;registered=true;quiet=true')
    command(app, 'Editor state selections')
    wait_screen(app, 'API selections;reverse=true;multiple=true;invalid=refused')
    command(app, 'Editor state clamp selections')
    wait_screen(app, 'API selections clamped;UTF16=true;empty=origin')
    command(app, 'Editor state inactive selections')
    wait_screen(app, 'API inactive selections;focus=other;identity=true', 'b.txt')
    command(app, 'Editor state arm movement')
    wait_screen(app, 'API native movement armed', 'a.cpp *')
    app.send(b'\x1b[C')
    wait_screen(app, 'API native movement event;identity=true')
    unchanged(root)
    history(app, root)
    print('PASS: real language close/open same-object events, contributed language and no-op/refusal; '
          'optimistic reverse/multiple selections, UTF16 clamps, empty origin, inactive editor and native movement preserve CRLF history/disks')


def language_save(root):
    app = ready(root)
    command(app, 'Editor state prepare language save')
    wait_screen(app, 'API language save prepared;python=true;CRLF=true')
    command(app, 'Editor state save language')
    wait_screen(app, 'Save As (existing files are protected)')
    destination = root / 'state-saved.cpp'
    app.send(CTRL_A)
    app.paste(str(destination))
    app.send(b'\r')
    wait_screen(app, 'API language saved;python=true;events=close/open/save', 'state-saved.cpp')
    assert destination.read_bytes() == ORIGINAL.encode('utf-8')
    command(app, 'Editor state edit saved language')
    wait_screen(app, 'API saved language edited;python=true;dirty=true')
    app.send(CTRL_Z)
    command(app, 'Editor state verify saved language clean')
    wait_screen(app, 'API saved language undo;python=true;clean=true', absent=('state-saved.cpp *',))
    assert destination.read_bytes() == ORIGINAL.encode('utf-8')
    history(app, root)
    assert destination.read_bytes() == ORIGINAL.encode('utf-8')
    print('PASS: explicit Python language survives real SaveAs to .cpp, old/new resource events and native Undo while original dirty CRLF work survives')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-state-api-') as directory:
        try:
            for name, run in [('state', state), ('language-save', language_save)]:
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
                        assert app.process.restored, 'Editor-state interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
