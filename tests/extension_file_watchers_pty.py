#!/usr/bin/env python3
"""Public external OS watcher events and disposal retain native dirty history."""
import json
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 original\r\n'


def run(root):
    source = root / 'a.txt'
    source.write_bytes(ORIGINAL.encode())
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/file-watcher-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'files.autoSave': 'off',
        'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }))
    node = shutil.which('node')
    assert node, 'OS watcher qualification requires real optional Node host'
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', source, enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.txt')
    app.paste('dirty ')
    command(app, 'Watch files arm')
    wait_screen(app, 'API watching files', 'a.txt *')
    # Establish OS subscription readiness from a real event rather than a timer.
    deadline, index, observed = time.monotonic() + 8, 0, None
    while time.monotonic() < deadline and observed is None:
        (root / f'watch-{index}.rs').write_bytes('猫 created\r\n'.encode())
        index += 1
        step = time.monotonic() + .12
        while time.monotonic() < step:
            app.read()
            match = re.search(r'API created: (watch-\d+\.rs)', app.screen.text())
            if match:
                observed = match.group(1)
                break
            time.sleep(.01)
    assert observed, 'No public OS watcher create event\n' + app.screen.text()
    watched = root / observed
    watched.write_bytes('🙂 changed\r\n'.encode())
    wait_screen(app, f'API changed: {observed}', 'a.txt *')
    watched.unlink()
    wait_screen(app, f'API deleted: {observed}', 'a.txt *')
    command(app, 'Watch files dispose')
    wait_screen(app, 'API watcher disposed')
    (root / 'watch-after.rs').write_bytes(b'late\r\n')
    deadline = time.monotonic() + .3
    while time.monotonic() < deadline:
        app.read()
        time.sleep(.01)
    command(app, 'Watch files verify')
    wait_screen(app, 'API disposed watcher stayed silent', 'a.txt *')
    assert source.read_bytes() == ORIGINAL.encode()
    app.send(CTRL_Z)
    command(app, 'Watch files clean identity')
    wait_screen(app, 'API watcher retained native Undo', absent=('a.txt *',))
    assert source.read_bytes() == ORIGINAL.encode()
    app.finish()
    print('PASS: public OS create/change/delete and disposal preserve dirty Unicode CRLF '
          'document identity, original keyboard Undo and disk bytes')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-watch-api-') as directory:
        try:
            run(Path(directory).resolve())
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Watcher interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
