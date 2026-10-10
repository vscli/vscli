#!/usr/bin/env python3
"""Native Outline tree keyboard journeys with no JavaScript executable on PATH."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from navigation_history_pty import BACK, FORWARD, active
from pty_smoke import CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import save, undo, redo

CASES = json.loads((Path(__file__).parent / 'vscode-reference' / 'outline-cases.json').read_text())
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


def sidebar(app):
    # Restrict to the rendered sidebar, so source code containing the same
    # symbol name cannot satisfy a tree-visibility oracle.
    app.read()
    return '\n'.join(''.join(app.screen.cells.get((row, column), ' ')
                              for column in range(25)) for row in range(1, 31))


def tree(app, present=(), absent=()):
    snapshot = ''

    def ready():
        nonlocal snapshot
        snapshot = sidebar(app)
        return 'OUTLINE' in snapshot.upper() and all(name in snapshot for name in present) \
            and not any(name in snapshot for name in absent)

    try:
        eventually(ready, timeout=8)
    except AssertionError as error:
        raise AssertionError(f'Outline visibility {present}/{absent}:\n{snapshot}\n{app.screen.text()}') from error


@contextmanager
def editor(root, name, *, held=False, provider=True):
    case = root / name
    case.mkdir()
    source = case / 'main.cpp'
    source.write_bytes(ORIGINAL)
    peer = Path(__file__).resolve().parent / 'fixtures' / 'outline_server.py'
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
        assert app.process.restored, 'Outline workflow leaked terminal mode'
        finished = True
    except BaseException:
        print(f'Outline PTY failure ({name}):\n{app.screen.text()}', file=sys.stderr)
        raise
    finally:
        if not finished:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, 'Failed Outline workflow leaked terminal mode'
                app.close_fds()
            except Exception as error:
                print(f'Outline PTY cleanup: {error}', file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-outline-pty-') as directory:
        root = Path(directory)
        with editor(root, 'hierarchy') as (app, source, case):
            palette(app, 'Outline: Focus')
            tree(app, present=('demo', 'Widget', 'render(', 'reset()', 'main()'))
            assert source.read_bytes() == ORIGINAL
            palette(app, 'Outline: Collapse All')
            tree(app, present=('demo', 'main()'), absent=('Widget', 'render(', 'reset()'))
            app.send(RIGHT)
            tree(app, present=('demo', 'Widget'), absent=('render(', 'reset()'))
            app.send(DOWN + RIGHT)
            tree(app, present=('Widget', 'render(', 'reset()'))
            app.send(DOWN + b'\r')
            active(app, 'main.cpp', 5)
            assert source.read_bytes() == ORIGINAL, 'Outline reveal saved native work'
            app.send(BACK)
            active(app, 'main.cpp', 1)
            app.send(FORWARD)
            active(app, 'main.cpp', 5)
            app.send('猫')
            expected = ORIGINAL.replace(b'render(int', '猫render(int'.encode(), 1)
            save(app, source, expected)
            undo(app, source, ORIGINAL)
            redo(app, source, expected)
            assert not any(row['event'] == 'gate-timeout' for row in records(case))
        print('PASS: native-only hierarchical Outline collapse/expand/Enter and Back/Forward preserve exact Unicode/CRLF save/Undo/Redo and terminal restoration')

        with editor(root, 'held-stale', held=True) as (app, source, case):
            palette(app, 'Outline: Focus')
            eventually(lambda: app.read() and any(row['event'] == 'request' for row in records(case)), timeout=8)
            app.send(b'\r' + ESCAPE)
            active(app, 'main.cpp', 1)
            app.send('é')
            app.send(CTRL_Z)
            palette(app, 'Outline: Focus')
            assert len([row for row in records(case) if row['event'] == 'request']) == 1
            assert source.read_bytes() == ORIGINAL
            (case / 'release').touch()
            tree(app, present=('demo', 'Widget', 'render('))
            eventually(lambda: app.read() and len([row for row in records(case) if row['event'] == 'request']) == 2, timeout=8)
            requests = [row for row in records(case) if row['event'] == 'request']
            assert max(row['pending'] for row in requests) == 1
            assert requests[1]['version'] > requests[0]['version'], 'Byte-equal Undo reused an obsolete symbol proof'
            app.send(ESCAPE)
            active(app, 'main.cpp', 1)
            save(app, source, ORIGINAL)
        print('PASS: pending Outline Enter is inert; edit/Undo retires same-byte old replies and queues one current native symbol request')

        with editor(root, 'unsupported', provider=False) as (app, source, _case):
            palette(app, 'Outline: Focus')
            tree(app, present=('No document-symbol',), absent=('Widget', 'render('))
            app.send(b'\r' + ESCAPE)
            active(app, 'main.cpp', 1)
            app.send('X')
            save(app, source, b'X' + ORIGINAL)
            undo(app, source, ORIGINAL)
        print('PASS: unsupported Outline stays explicit and nonactionable; Escape returns to the existing native editor without Node')


if __name__ == '__main__':
    run()
