#!/usr/bin/env python3
"""Real native frames, public options/reveal events and protected Unicode CRLF history."""
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import struct
import sys
import tempfile
import termios
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 header\r\n' + ''.join(f'    line {line:02} 猫🙂\r\n' for line in range(1, 97))


def unchanged(root):
    assert (root / 'a.cpp').read_bytes() == ORIGINAL.encode('utf-8')
    assert (root / 'b.txt').read_bytes() == b'other\r\n'


def ready(root):
    (root / 'a.cpp').write_bytes(ORIGINAL.encode('utf-8'))
    (root / 'b.txt').write_bytes(b'other\r\n')
    extension = root / 'extension'
    shutil.copytree(Path(__file__).parent / 'fixtures/editor-presentation-extension', extension)
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual editor presentation qualification requires Node')
    app = Editor(root, '--extension', extension, '--extension-node', str(Path(node).resolve()),
                 '--config-dir', config, '--no-session', root / 'a.cpp',
                 enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.cpp')
    app.paste('dirty ')
    return app


def options(root):
    app = ready(root)
    command(app, 'Editor presentation shared panes')
    wait_screen(app, 'API presentation shared;identity=true;focus=first')
    command(app, 'Editor presentation native options')
    wait_screen(app, 'API presentation options;sharedIndent=true;perPane=true;Interval=3')
    command(app, 'Editor presentation auto indentation')
    wait_screen(app, 'API presentation auto;detected=4;tabSizeAlias=true')
    command(app, 'Editor presentation explicit indentation')
    wait_screen(app, 'API presentation explicit;tab=4;indent=2;spaces=true')
    app.send(b'\t')
    command(app, 'Editor presentation native tab changed')
    wait_screen(app, 'API presentation native Tab;indent=2;sharedText=true')
    app.send(CTRL_Z)
    command(app, 'Editor presentation verify dirty history')
    wait_screen(app, 'API presentation undo;dirtyBaseline=preserved')
    app.send(b'\x1b[122;6u')
    command(app, 'Editor presentation native tab changed')
    wait_screen(app, 'API presentation native Tab;indent=2;sharedText=true')
    app.send(CTRL_Z)
    command(app, 'Editor presentation verify dirty history')
    wait_screen(app, 'API presentation undo;dirtyBaseline=preserved')
    app.send(CTRL_Z)
    command(app, 'Editor presentation verify clean history')
    wait_screen(app, 'API presentation original Undo;clean=true', absent=('a.cpp *',))
    app.send(b'\x1b[122;6u')
    command(app, 'Editor presentation verify dirty history')
    wait_screen(app, 'API presentation undo;dirtyBaseline=preserved', 'a.cpp *')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: native options events distinguish shared indentation from per-pane cursor/line numbers; auto/tabSize alias and actual two-space Tab preserve UnicodeCRLF Undo/Redo and disks')


def viewport(root):
    app = ready(root)
    command(app, 'Editor presentation rendered ranges')
    wait_screen(app, 'API presentation rendered;ranges=actual;UTF16=true')
    command(app, 'Editor presentation shared panes')
    wait_screen(app, 'API presentation shared;identity=true;focus=first')
    for query, receipt in [
        ('Editor presentation reveal top', 'API presentation reveal top;event=true;inactive=true;selections=preserved'),
        ('Editor presentation reveal center', 'API presentation reveal center;event=true;inactive=true;selections=preserved'),
        ('Editor presentation reveal inside', 'API presentation reveal inside;quiet=true;selections=preserved'),
        ('Editor presentation reveal default', 'API presentation reveal default;event=true;inactive=true;selections=preserved'),
    ]:
        command(app, query)
        wait_screen(app, receipt)
    command(app, 'Editor presentation arm resize')
    wait_screen(app, 'API presentation resize armed')
    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 110, 0, 0))
    os.kill(app.process.pid, signal.SIGWINCH)
    wait_screen(app, 'API presentation resized;event=true;selections=preserved')
    command(app, 'Editor presentation retire second editor')
    wait_screen(app, 'API presentation retired;options/reveal=refused;replacement=untouched', 'b.txt')
    unchanged(root)
    app.finish(discard=True)
    unchanged(root)
    print('PASS: actual visible UTF16 ranges follow inactive-pane Default/Center/IfOutside/AtTop reveal and resize without changing focus/selections; retired options/reveal refuse replacement mutation')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-editor-presentation-') as directory:
        try:
            for name, run in [('options', options), ('viewport', viewport)]:
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
                        assert app.process.restored, 'Editor presentation interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
