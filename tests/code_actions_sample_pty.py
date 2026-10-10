#!/usr/bin/env python3
"""Actual terminal sample conformance for unchanged official Code Actions 0.0.2."""
import os
from pathlib import Path
import sys
import tempfile

from extension_sessions_pty import Editor, LIVE, command, wait, save
from extension_surfaces_pty import stop_surface_process
from prepare_code_actions_sample import prepare
from pty_smoke import CTRL_Z, wait_screen

ORIGINAL = '猫🙂 :) emoji\r\n'
DIRTY = 'dirty 猫🙂 :) emoji\r\n'
QUICK_FIX = b'\x1b[46;5u'  # Original Ctrl+. delivered through enhanced terminal input.
HOME = b'\x1b[H'
RIGHT = b'\x1b[C'
SHIFT_END = b'\x1b[1;2F'
SHIFT_RIGHT = b'\x1b[1;2C'
ESCAPE = b'\x1b[27u'


def picker(app, *, full=True):
    app.send(HOME + RIGHT * 9)
    app.send(SHIFT_END if full else SHIFT_RIGHT * 2)
    app.send(QUICK_FIX)
    return wait_screen(app, 'Code Actions', 'Convert to 😺', 'Convert to 😀', 'Convert to 💩', timeout=8)


def select_title(app, title):
    # Read rendered row order, reset to its first row, then navigate natively.
    rows = [line for line in app.screen.text().splitlines()
            if 'Convert to ' in line or 'Learn more...' in line]
    index = next((index for index, line in enumerate(rows) if title in line), None)
    assert index is not None, f'Missing {title!r}:\n{app.screen.text()}'
    app.send(b'\x1b[A' * 12)
    app.send(b'\x1b[B' * index + b'\r')



def run(root, package, *, full=True):
    root.mkdir()
    source = root / 'document.md'
    source.write_bytes(ORIGINAL.encode())
    app = Editor(root, '--extension', package, '--no-session', source, enhanced=True)
    wait_screen(app, 'Extension ready:', 'document.md', timeout=8)
    command(app, 'View: Problems')
    wait_screen(app, 'Problems', "When you say 'emoji'", timeout=8)
    app.send(ESCAPE)
    app.send(HOME)
    app.paste('dirty ')
    assert source.read_bytes() == ORIGINAL.encode()
    screen = picker(app, full=full)
    rows = [line for line in screen.splitlines() if 'Convert to ' in line or 'Learn more...' in line]
    assert len(rows) == (5 if full else 4), f'Two-provider aggregate row count:\n{screen}'
    if full:
        select_title(app, 'Learn more...')
        wait_screen(app, 'failed', timeout=8)
        assert source.read_bytes() == ORIGINAL.encode()
        picker(app, full=True)
    select_title(app, 'Convert to 😀')
    expected = DIRTY.replace(':)', '😀')
    save(app, source, expected)
    app.send(CTRL_Z)
    save(app, source, DIRTY)
    app.send(CTRL_Z)
    save(app, source, ORIGINAL)
    command(app, 'File: Close Editor')
    wait(app, 'No open editors')
    app.finish()
    print(f'PASS: unchanged official sample 0.0.2 {"two matching providers" if full else "empty newer provider"}, native Ctrl+., isolated disabled command, Unicode/CRLF and per-action Undo')


if __name__ == '__main__':
    package = Path(os.environ['VSCLI_CODE_ACTIONS_SAMPLE']).resolve()
    prepare(package, verify_only=True)
    with tempfile.TemporaryDirectory(prefix='vscli-code-actions-sample-pty-') as directory:
        try:
            run(Path(directory) / 'both', package)
            run(Path(directory) / 'older', package, full=False)
            prepare(package, verify_only=True)
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
