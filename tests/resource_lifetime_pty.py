#!/usr/bin/env python3
"""Native Explorer path ABA preserves dirty history, baselines and original keys."""
from pathlib import Path
import tempfile
from pty_smoke import Editor, CTRL_A, CTRL_S, CTRL_Z, eventually, wait_screen

ORIGINAL = '猫🙂 disk\r\nnext\r\n'
DIRTY = 'dirty λ ' + ORIGINAL
EXPLORER = b'\x1b[101;6u'
EDITOR = b'\x1b[49;5u'
F2 = b'\x1bOQ'
REDO = b'\x1b[122;6u'


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-resource-lifetime-pty-') as directory:
        root = Path(directory).resolve()
        source = root / 'source.cpp'
        moved = root / 'moved.cpp'
        source.write_bytes(ORIGINAL.encode())
        app = Editor(root, '--no-session', source, enhanced=True, extra_env={'PATH': ''})
        completed = False
        try:
            app.paste('dirty λ ')
            wait_screen(app, 'dirty λ 猫🙂', 'source.cpp *')
            for before, after in [(source, moved), (moved, source)]:
                app.send(EXPLORER)
                app.send(F2)
                app.send(CTRL_A)
                app.paste(str(after))
                app.send(b'\r')
                eventually(lambda: app.read() and after.exists() and not before.exists()
                           and 'Renamed to' in app.screen.text())
                assert after.read_bytes() == ORIGINAL.encode()
                app.send(EDITOR)
                wait_screen(app, 'dirty λ 猫🙂', after.name + ' *')
            app.send(CTRL_Z)
            wait_screen(app, '猫🙂 disk', absent=('dirty λ', 'source.cpp *'))
            assert source.read_bytes() == ORIGINAL.encode()
            app.send(REDO)
            wait_screen(app, 'dirty λ 猫🙂', 'source.cpp *')
            app.send(CTRL_S)
            eventually(lambda: app.read() and source.read_bytes() == DIRTY.encode())
            wait_screen(app, 'Saved', absent=('source.cpp *',))
            app.send(CTRL_Z)
            wait_screen(app, '猫🙂 disk', 'source.cpp *', absent=('dirty λ',))
            assert source.read_bytes() == DIRTY.encode(), 'Undo must retain committed disk bytes'
            app.send(REDO)
            wait_screen(app, 'dirty λ 猫🙂', absent=('source.cpp *',))
            app.finish()
            completed = True
        finally:
            if not completed:
                app.finish(discard=True)
    print('PASS: native Explorer F2 rename away/back, dirty Unicode CRLF, original '
          'Undo/Redo/Save, preserved disk baseline, missing Node and terminal restoration')


if __name__ == '__main__':
    run()
