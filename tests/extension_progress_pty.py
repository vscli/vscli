#!/usr/bin/env python3
"""Public Window progress keeps native editing usable and cleans up actual tasks."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait, save
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, eventually

FIXTURE = Path(__file__).parent / 'fixtures/window-progress-extension'
ORIGINAL = '猫🙂 original\r\n'


def run(root):
    file = root / 'a.txt'
    file.write_bytes(ORIGINAL.encode())
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'files.autoSave': 'off',
        'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }))
    node = str(Path(shutil.which('node')).resolve())
    options = ('--config-dir', config, '--no-session', '--extension-node', node)
    app = Editor(root, *options, '--extension', FIXTURE, file, enhanced=True,
                 extra_env={'PATH': ''})
    wait(app, '(4 commands)')
    app.paste('dirty ')
    command(app, 'Progress begin')
    wait(app, '⟳ Profile')
    app.paste('local ')
    command(app, 'Progress step')
    wait(app, '⟳ Profile: phase 猫')
    command(app, 'Progress finish')
    wait(app, 'Progress value preserved')
    eventually(lambda: app.read() and '⟳ Profile' not in app.screen.text())
    command(app, 'Progress fail')
    wait(app, 'Progress failure preserved')
    save(app, file, 'dirty local ' + ORIGINAL)
    app.send(CTRL_Z)
    save(app, file, 'dirty ' + ORIGINAL)
    app.send(CTRL_Z)
    save(app, file, ORIGINAL)
    command(app, 'Progress begin')
    wait(app, '⟳ Profile')
    command(app, 'Extensions: Stop Host')
    wait(app, 'Extension host stopped')
    eventually(lambda: app.read() and '⟳ Profile' not in app.screen.text())
    app.paste('!')
    save(app, file, '!' + ORIGINAL)
    app.send(CTRL_Z)
    save(app, file, ORIGINAL)
    app.finish()
    print('PASS: public status progress, exact task values/errors, concurrent native editing, Unicode CRLF save/Undo and held-task host cleanup')

    app = Editor(root, *options, '--extension', FIXTURE, enhanced=True,
                 extra_env={'PATH': ''})
    wait(app, '(4 commands)')
    wait(app, 'No open editors')
    command(app, 'Progress begin')
    wait(app, '⟳ Profile')
    command(app, 'Progress step')
    wait(app, '⟳ Profile: phase 猫')
    command(app, 'Progress finish')
    wait(app, 'Progress value preserved')
    wait(app, 'No open editors')
    app.finish()
    print('PASS: empty-workbench progress completes without a phantom document')

    app = Editor(root, '--config-dir', config, '--no-session', '--extension-node',
                 root / 'missing-node', file, enhanced=True, extra_env={'PATH': ''})
    app.paste('native ')
    save(app, file, 'native ' + ORIGINAL)
    app.send(CTRL_Z)
    save(app, file, ORIGINAL)
    app.finish()
    print('PASS: a fresh native editor restarts without Node and preserves editing, disk bytes and keyboard Undo')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-progress-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            for app in list(LIVE):
                stop_surface_process(app)
