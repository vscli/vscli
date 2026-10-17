#!/usr/bin/env python3
"""Real selected extension findFiles; query/cancel preserve native history/disks."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 A\r\nlast\r\n'
FILES = {'a.txt': ORIGINAL, 'src/keep.rs': 'keep\r\n',
         'src/excluded.rs': 'excluded\r\n', 'src/sub/deep.rs': 'deep\r\n',
         'ignored/hidden.rs': 'hidden\r\n', 'assets/note.txt': 'note\r\n',
         '.gitignore': 'src/keep.rs\n'}


def unchanged(root):
    for name, text in FILES.items():
        assert (root / name).read_bytes() == text.encode('utf-8'), 'Search modified ' + name


def run(root):
    for name, text in FILES.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode('utf-8'))
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/find-files-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
        'files.exclude': {'**/excluded.rs': True, '**/ignored/**': True},
        'search.exclude': {'**/keep.rs': True},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Real optional findFiles host qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.txt', enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.txt')
    app.paste('dirty ')
    command(app, 'Files search scopes')
    wait_screen(app, 'API find scopes=true;relative=true;limits=true', 'a.txt *')
    unchanged(root)
    command(app, 'Files search cancellation')
    wait_screen(app, 'API find cancelled=true;pre-fresh=true', 'a.txt *')
    unchanged(root)
    app.send(CTRL_Z)
    command(app, 'Files verify clean identity')
    wait_screen(app, 'API search retained clean identity', absent=('a.txt *',))
    unchanged(root)
    app.send(b'\x1b[122;6u')
    command(app, 'Files verify dirty identity')
    wait_screen(app, 'API search retained dirty identity', 'a.txt *')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: selected public findFiles scopes/null/additive excludes/RelativePattern/limits/cancel '
          'preserve shared dirty identity, original native Undo/Redo and every Unicode CRLF disk')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-find-api-') as directory:
        try:
            run(Path(directory).resolve())
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Search interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
