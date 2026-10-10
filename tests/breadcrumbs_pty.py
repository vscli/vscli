#!/usr/bin/env python3
"""Native Outline tree keyboard journeys with no JavaScript executable on PATH."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from navigation_history_pty import BACK, FORWARD, active, go_line
from pty_smoke import CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import save, undo, redo

CASES = json.loads((Path(__file__).parent / 'vscode-reference' / 'breadcrumbs-cases.json').read_text())
ORIGINAL = CASES[0]['text'].encode()
UP, DOWN = b'\x1b[A', b'\x1b[B'
LEFT, RIGHT = b'\x1b[D', b'\x1b[C'
ESCAPE = b'\x1b[27u'


def records(root):
    log = root / 'peer.jsonl'
    return [json.loads(line) for line in log.read_text().splitlines(keepends=True)
            if line.endswith('\n')] if log.exists() else []


def palette(app, label):
    app.send(b'\x1bOP')  # Original F1 command palette, no new Outline shortcut.
    wait_screen(app, 'Command Palette')
    app.paste(label)
    app.send(b'\r')
    eventually(lambda: app.read() and 'Command Palette' not in app.screen.text())


FOCUS = b'\x1b[59;6u'  # Original Linux Ctrl+Shift+Semicolon.
SELECT = b'\x1b[46;6u'  # Original Linux Ctrl+Shift+Period.


def trail(app, present=(), absent=()):
    snapshot = ''
    def ready():
        nonlocal snapshot
        app.read()
        # One native pane: tab row0, active editor Breadcrumbs row1.
        snapshot = ''.join(app.screen.cells.get((1, column), ' ') for column in range(26, 110))
        return all(name in snapshot for name in present) and not any(name in snapshot for name in absent)
    try:
        eventually(ready, timeout=8)
    except AssertionError as error:
        raise AssertionError(f'Breadcrumb header {present}/{absent}: {snapshot}\n{app.screen.text()}') from error


@contextmanager
def editor(root, name, *, held=False, provider=True):
    case = root / name
    case.mkdir()
    source = case / 'main.cpp'
    source.write_bytes(ORIGINAL)
    peer = Path(__file__).resolve().parent / 'fixtures' / 'breadcrumbs_server.py'
    args = ['--extension-node', case / 'missing-node', '--no-session']
    if provider:
        args += ['--lsp', sys.executable, '--lsp-arg', peer,
                 '--lsp-arg', case / 'peer.jsonl', '--lsp-arg', case / 'release',
                 '--lsp-language', 'cpp']
        if held:
            args += ['--lsp-arg', '--hold-first']
    else:
        args += ['--no-lsp']
    app = Editor(case, *args, source, enhanced=True, extra_env={'PATH': ''})
    finished = False
    try:
        active(app, 'main.cpp', 1)
        if provider:
            wait_screen(app, 'Language server ready', timeout=8)
        yield app, source, case
        app.finish()
        assert app.process.restored, 'Breadcrumbs workflow leaked terminal mode'
        finished = True
    except BaseException:
        print(f'Breadcrumbs PTY failure ({name}):\n{app.screen.text()}', file=sys.stderr)
        raise
    finally:
        if not finished:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, 'Failed Breadcrumbs workflow leaked terminal mode'
                app.close_fds()
            except Exception as error:
                print(f'Breadcrumbs PTY cleanup: {error}', file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-breadcrumbs-pty-') as directory:
        root = Path(directory)
        with editor(root, 'file-only', provider=False) as (app, source, _case):
            trail(app, present=('main.cpp',), absent=('Widget', 'render('))
            app.send(SELECT)
            wait_screen(app, 'Folder and file dropdowns are not supported')
            app.send(ESCAPE)
            active(app, 'main.cpp', 1)
            app.send('猫')
            save(app, source, '猫'.encode() + ORIGINAL)
            undo(app, source, ORIGINAL)
        print('PASS: native file Breadcrumbs focus and explicit unsupported dropdown preserve editing and exact UnicodeCRLF Undo without Node/LSP')

        with editor(root, 'hierarchical') as (app, source, case):
            go_line(app, 'main.cpp', 6)
            trail(app, present=('main.cpp', 'demo', 'Widget', 'render('))
            app.send(SELECT)
            wait_screen(app, 'Symbols')
            app.send(DOWN + b'\r')
            active(app, 'main.cpp', 8)
            assert source.read_bytes() == ORIGINAL, 'Breadcrumb reveal saved native work'
            app.send(BACK)
            active(app, 'main.cpp', 6)
            app.send(FORWARD)
            active(app, 'main.cpp', 8)
            app.send('猫')
            expected = ORIGINAL.replace(b'reset()', '猫reset()'.encode(), 1)
            save(app, source, expected)
            undo(app, source, ORIGINAL)
            redo(app, source, expected)
            assert not any(row['event'] == 'gate-timeout' for row in records(case))
        print('PASS: original CtrlShiftPeriod sibling picker Enter reveals collapsed identifier and BackForward plus UnicodeCRLF SaveUndoRedo preserve terminal state')

        with editor(root, 'shared-held', held=True) as (app, source, case):
            eventually(lambda: app.read() and any(row['event'] == 'request' for row in records(case)), timeout=8)
            palette(app, 'Outline: Focus')
            app.send(FOCUS + ESCAPE)
            active(app, 'main.cpp', 1)
            app.send('é')
            app.send(CTRL_Z)
            assert len([row for row in records(case) if row['event'] == 'request']) == 1
            assert source.read_bytes() == ORIGINAL
            (case / 'release').touch()
            go_line(app, 'main.cpp', 6)
            trail(app, present=('demo', 'Widget', 'render('))
            eventually(lambda: app.read() and len([row for row in records(case) if row['event'] == 'request']) == 2, timeout=8)
            requests = [row for row in records(case) if row['event'] == 'request']
            assert max(row['pending'] for row in requests) == 1
            assert requests[1]['version'] > requests[0]['version'], 'Same-byte Undo revived obsolete symbols'
            app.send(SELECT)
            wait_screen(app, 'Symbols')
            app.send(ESCAPE)
            active(app, 'main.cpp', 6)
            save(app, source, ORIGINAL)
        print('PASS: Outline and Breadcrumbs share one held actual request; editUndo rejects old version and fresh picker Escape remains native and preserves disk')


if __name__ == '__main__':
    run()
