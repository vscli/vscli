#!/usr/bin/env python3
"""Header-free native workbench, actual top-row mouse targeting and CRLF undo."""
import fcntl
from pathlib import Path
import struct
import tempfile
import termios

from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def click(app, column, row):
    # SGR coordinates are one-based; the screen and native layout are zero-based.
    app.send(f'\x1b[<0;{column + 1};{row + 1}M\x1b[<0;{column + 1};{row + 1}m')


def row_text(app, row, width):
    return ''.join(app.screen.cells.get((row, column), ' ') for column in range(width))


def saved(app, path, expected, height, width):
    def acknowledged():
        app.read()
        return (path.read_bytes() == expected
                and 'Saved' in row_text(app, height - 2, width)
                and '*' not in row_text(app, height - 1, width).split('|')[0])
    eventually(acknowledged)


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-header-layout-pty-') as directory:
        root = Path(directory)
        first, second = root / 'alpha.txt', root / 'beta.txt'
        original = b'alpha\r\nsecond line\r\n'
        first.write_bytes(original)
        second.write_bytes(b'beta\r\n')
        app = Editor(root, first, second, enhanced=True, mouse=True,
                     extra_env={'PATH': ''})
        finished = False
        try:
            # Hide Explorer so screen coordinates directly describe editor rows.
            app.send(b'\x02')
            wait_screen(app, absent=('EXPLORER',))
            for width, height in [(110, 32), (50, 12), (80, 24)]:
                fcntl.ioctl(app.slave, termios.TIOCSWINSZ,
                            struct.pack('HHHH', height, width, 0, 0))

                def drawn():
                    app.read()
                    top = row_text(app, 0, width)
                    return ('alpha.txt' in top and 'beta.txt' in top
                            and 'Ln ' in row_text(app, height - 1, width))

                try:
                    eventually(drawn)
                except AssertionError as error:
                    raise AssertionError(f'Workbench after resize to {width}x{height}:\n{app.screen.text()}') from error
                top = row_text(app, 0, width)
                assert 'VSCLI' not in top, top
                click(app, top.index('alpha.txt'), 0)
                eventually(lambda: app.read()
                           and 'alpha.txt' in row_text(app, 1, width)
                           and 'alpha' in row_text(app, 2, width))
                # Click the actual first content cell after the tab/breadcrumb rows.
                click(app, row_text(app, 2, width).index('alpha'), 2)
                app.send('Z')
                app.send(CTRL_S)
                saved(app, first, b'Z' + original, height, width)
                app.send(CTRL_Z)
                app.send(CTRL_S)
                saved(app, first, original, height, width)
                click(app, row_text(app, 0, width).index('beta.txt'), 0)
                eventually(lambda: app.read()
                           and 'beta.txt' in row_text(app, 1, width)
                           and 'beta' in row_text(app, 2, width))
                assert second.read_bytes() == b'beta\r\n'
            app.finish()
            finished = True
        finally:
            if not finished:
                app.process.kill()
                app.process.wait(timeout=3)
                app.close_fds()
        print('PASS: title-free top-row tabs, breadcrumbs, resized mouse editing, exact CRLF Save/Undo and terminal restoration')


if __name__ == '__main__':
    run()
