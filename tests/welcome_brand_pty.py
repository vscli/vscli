#!/usr/bin/env python3
"""Qualify fallback cells and Kitty wire lifecycle in real PTYs, without terminal probing."""
import fcntl
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import pty_smoke
from pty_smoke import CTRL_S, CTRL_Z, eventually
from extension_sessions_pty import Editor, LIVE, command, wait, save


class GraphicsScreen(pty_smoke.Screen):
    def __init__(self):
        super().__init__()
        self.wire = b''
        self.graphics = []

    def feed(self, data):
        self.wire += data
        while self.wire:
            start = self.wire.find(b'\x1b_G')
            if start < 0:
                hold = 2 if self.wire.endswith(b'\x1b_') else 1 if self.wire.endswith(b'\x1b') else 0
                if len(self.wire) > hold:
                    super().feed(self.wire[:-hold] if hold else self.wire)
                    self.wire = self.wire[-hold:] if hold else b''
                return
            if start:
                super().feed(self.wire[:start])
                self.wire = self.wire[start:]
            end = self.wire.find(b'\x1b\\')
            if end < 0:
                return
            self.graphics.append(self.wire[3:end])
            self.wire = self.wire[end + 2:]


def resize(app, columns, rows):
    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, columns * 10, rows * 20))
    os.kill(app.process.pid, signal.SIGWINCH)


def visible_graphic(app):
    return any('\U0010eeee' in value for value in app.screen.cells.values())


def run(root, kitty):
    root.mkdir()
    config = root / 'config'
    source = root / 'hello.txt'
    source.write_bytes('original 猫\r\n'.encode())
    app = Editor(root, '--config-dir', config, '--no-session', enhanced=True,
                 extra_env={'TERM': 'xterm-kitty' if kitty else 'xterm-256color',
                            'KITTY_WINDOW_ID': '42', 'NO_COLOR': ''})
    resize(app, 140, 40)
    wait(app, 'User settings')
    wait(app, str(config / 'settings.json'))
    if kitty:
        eventually(lambda: app.read() and visible_graphic(app))
        assert any(b'a=T' in item and b's=180,v=180' in item for item in app.screen.graphics)
    else:
        eventually(lambda: app.read() and any(c in ('▀', '▄') for c in app.screen.cells.values()))
        assert not app.screen.graphics
    command(app, 'Keyboard Inspector')
    wait(app, 'Keyboard Inspector · Esc closes')
    assert not visible_graphic(app), 'Inspector must hide welcome image placeholders'
    app.send(CTRL_S)
    wait(app, 'Mapped command: workbench.action.files.save')
    assert not (config / 'settings.json').exists()
    app.send(b'\x1b')
    wait(app, 'User settings')
    if kitty:
        eventually(lambda: app.read() and visible_graphic(app))
    command(app, 'Help: Getting Started')
    wait(app, 'start automatically')
    assert not visible_graphic(app)
    app.send(b'\x1b')
    wait(app, 'User settings')
    app.send(b'\x0f')
    app.send(str(source))
    app.send(b'\r')
    wait(app, 'original 猫')
    assert not visible_graphic(app)
    app.send('dirty ')
    app.send(b'\x17')
    wait(app, 'Save changes')
    app.send(b'\x1b')
    assert source.read_bytes() == 'original 猫\r\n'.encode()
    save(app, source, 'dirty original 猫\r\n')
    app.send(CTRL_Z)
    save(app, source, 'original 猫\r\n')
    app.send(b'\x17')
    wait(app, 'Recent files')
    wait(app, 'hello.txt')
    resize(app, 80, 24)
    wait(app, 'User settings')
    if kitty:
        eventually(lambda: app.read() and any(b'a=T' in item and b's=100,v=100' in item for item in app.screen.graphics))
        assert any(b'a=d,d=I' in item for item in app.screen.graphics)
    app.send(b'\x1b[44;5u')  # Ctrl+, opens the advertised real path.
    wait(app, 'settings.json')
    app.send(CTRL_S)
    eventually(lambda: app.read() and (config / 'settings.json').exists())
    assert (config / 'settings.json').read_text() == '{\n}\n'
    app.finish()
    if kitty:
        ids = {part for item in app.screen.graphics for part in item.split(b';')[0].split(b',') if part.startswith(b'i=')}
        assert ids == {b'i=' + str(0x56000000 | (app.process.pid & 0x00ffffff)).encode()}, ids
        assert b'a=d,d=I' in app.screen.graphics[-1], 'Exit must free the uploaded payload'
    else:
        assert not app.screen.graphics
    assert source.read_bytes() == 'original 猫\r\n'.encode()
    print('PASS: ' + ('Kitty' if kitty else 'cell fallback') + ' welcome → inspector → help → open/edit/cancel/save/undo → close/recents → resize → actual settings → cleanup')


if __name__ == '__main__':
    pty_smoke.Screen = GraphicsScreen
    # Explicitly remove inherited multiplexers; never enable their passthrough.
    os.environ.pop('TMUX', None)
    os.environ.pop('STY', None)
    with tempfile.TemporaryDirectory(prefix='vscli-welcome-brand-') as directory:
        try:
            run(Path(directory) / 'fallback', False)
            run(Path(directory) / 'kitty', True)
        finally:
            failed = sys.exc_info()[0] is not None
            for app in list(LIVE):
                try:
                    if app.process.poll() is None:
                        os.kill(app.process.pid, signal.SIGTERM)
                        try:
                            app.process.wait(timeout=4)
                        except (AssertionError, subprocess.TimeoutExpired):
                            app.process.kill()
                            app.process.wait(timeout=3)
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    if not failed:
                        raise
                    print(f'Additional cleanup error: {error}', file=sys.stderr)
