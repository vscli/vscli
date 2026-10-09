#!/usr/bin/env python3
"""Inspect actual legacy/enhanced events without editing or saving native text."""
import fcntl
import os
from pathlib import Path
import signal
import struct
import sys
import tempfile
import termios

from pty_smoke import Editor, CTRL_S, eventually


def wait(app, expected):
    try:
        eventually(lambda: app.read() and expected in app.screen.text(), timeout=8)
    except AssertionError as error:
        raise AssertionError(f"Waiting for {expected!r}:\n{app.screen.text()}") from error


def resize(app, width, height):
    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
    os.kill(app.process.pid, signal.SIGWINCH)


def run(enhanced):
    with tempfile.TemporaryDirectory(prefix='vscli-keyboard-') as directory:
        root = Path(directory)
        source = root / 'protected.txt'
        source.write_bytes(b'disk\r\n')
        app = Editor(root, source, enhanced=enhanced)
        finished = False
        try:
            app.paste('unsaved🙂')
            wait(app, 'unsaved🙂disk')
            app.send(b'\x1bOP')
            app.send('Keyboard Inspector\r')
            wait(app, 'Keyboard Inspector · Esc closes')
            wait(app, 'F12')
            wait(app, 'PgDn')
            wait(app, 'Reported modifiers:')
            wait(app, 'Protocol: ' + ('enhanced' if enhanced else 'legacy'))
            app.send(CTRL_S)
            wait(app, 'Shortcut: Ctrl + S')
            wait(app, 'Mapped command: workbench.action.files.save')
            assert source.read_bytes() == b'disk\r\n', 'Inspector executed Save'
            app.send(b'\x0b')
            wait(app, 'chord prefix')
            app.send(b'\x03')
            wait(app, 'editor.action.addCommentLine')
            if enhanced:
                app.send(b'\x1b[112;6u')
                wait(app, 'Shortcut: Ctrl + Shift + P')
                wait(app, 'Mapped command: workbench.action.showCommands')
                # P is the highlighted keycap. Its foreground becomes the theme
                # background; the nearby O key remains ordinary foreground.
                def highlighted():
                    app.read()
                    positions = [(pos, value) for pos, value in app.screen.cells.items()
                                 if value == 'P' and 14 <= pos[0] <= 16]
                    return any(app.screen.colors.get(pos) != app.screen.colors.get((pos[0], pos[1] - 5))
                               for pos, _ in positions)
                eventually(highlighted)
            app.paste('this must not edit')
            resize(app, 60, 16)
            wait(app, 'Compact view')
            resize(app, 110, 32)
            wait(app, 'PgDn')
            wait(app, 'highlight = last event, not held keys')
            app.send(b'\x1b')
            wait(app, 'unsaved🙂disk')
            assert 'Keyboard Inspector · Esc closes' not in app.screen.text()
            app.send(CTRL_S)
            eventually(lambda: app.read() and source.read_bytes() == 'unsaved🙂disk\r\n'.encode())
            app.finish()
            finished = True
            print(f"PASS: {'enhanced' if enhanced else 'legacy'} keyboard layout, modifiers/chords, resize, Escape and unsaved integrity")
        finally:
            if not finished:
                try:
                    app.process.kill()
                    app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    print(f'PTY cleanup failed: {error}', file=sys.stderr)


if __name__ == '__main__':
    run(False)
    run(True)
