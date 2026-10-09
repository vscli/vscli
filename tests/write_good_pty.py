#!/usr/bin/env python3
"""Real terminal workflows for unchanged pinned Write Good Linter diagnostics."""
import json
import os
from pathlib import Path
import sys
import tempfile
import time

from extension_sessions_pty import Editor, LIVE, command, wait, save
from extension_surfaces_pty import stop_surface_process
from prepare_write_good import prepare
from pty_smoke import CTRL_A, CTRL_Z, wait_screen

VERY = '猫🙂 This is very unique.\r\n'
PASSIVE = '猫🙂 The cat was killed.\r\n'


def open_file(app, path):
    app.send(b'\x0f')  # Original Linux Ctrl+O.
    wait(app, 'Open File (absolute or workspace-relative)')
    app.send(str(path) + '\r')
    wait_screen(app, path.name, 'Ln 1, Col', absent=('Open File (absolute or workspace-relative)', 'No open editors'), timeout=8)


def problems(app, expected):
    command(app, 'View: Problems')
    wait_screen(app, 'Problems', expected, timeout=8)


def run(root, package, only_save=False):
    root.mkdir()
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'write-good.debounce-time-in-ms': 0,
        'write-good.only-lint-on-save': only_save,
    }))
    source = root / 'document.md'
    source.write_bytes(VERY.encode())
    app = Editor(root, '--config-dir', config, '--extension', package, '--no-session', enhanced=True)
    wait_screen(app, 'Extension ready:', 'No open editors', timeout=8)
    open_file(app, source)  # Open after the unchanged extension registered listeners.
    problems(app, 'weasel word')
    assert source.read_bytes() == VERY.encode()
    app.send(b'\r')  # Native Problems selection navigates to its UTF-16 range.
    wait_screen(app, 'Ln 1, Col', absent=('Problems · Enter',), timeout=8)
    app.send(CTRL_A)
    app.paste(PASSIVE)
    assert source.read_bytes() == VERY.encode(), 'Diagnostics or editing saved without consent'
    if only_save:
        problems(app, 'No results')
        deadline = time.monotonic() + .25
        while time.monotonic() < deadline:
            app.read()
            assert 'passive voice' not in app.screen.text(), 'Only-save extension linted before saving'
            time.sleep(.01)
        app.send(b'\x1b[27u')
        save(app, source, PASSIVE)
        problems(app, 'passive voice')
        app.send(b'\x1b[27u')
    else:
        problems(app, 'passive voice')
        app.send(b'\x1b[27u')
        save(app, source, PASSIVE)
    app.send(CTRL_Z)
    save(app, source, VERY)
    problems(app, 'weasel word')
    app.send(b'\x1b[27u')
    command(app, 'File: Close Editor')
    wait(app, 'No open editors')
    problems(app, 'No results')
    app.send(b'\x1b[27u')
    open_file(app, source)
    problems(app, 'weasel word')
    app.send(b'\x1b[27u')
    command(app, 'Extensions: Stop Selected Package')
    wait(app, 'Stop Selected Extension ID (publisher.name)')
    app.send('travisthetechie.write-good-linter\r')
    problems(app, 'No results')
    app.send(b'\x1b[27u')
    assert source.read_bytes() == VERY.encode()
    app.finish()
    print(f'PASS: unchanged Write Good 0.1.7 {"only-save" if only_save else "live"} diagnostics, Problems navigation, Unicode/CRLF save/undo, close and owner retirement')


if __name__ == '__main__':
    package = Path(os.environ['VSCLI_WRITE_GOOD']).resolve()
    prepare(package, verify_only=True)
    with tempfile.TemporaryDirectory(prefix='vscli-write-good-pty-') as directory:
        try:
            run(Path(directory) / 'live', package)
            run(Path(directory) / 'only-save', package, only_save=True)
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    stop_surface_process(app)
                    assert app.process.restored, 'Terminal mode leaked on cleanup'
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
