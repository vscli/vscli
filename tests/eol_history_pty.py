#!/usr/bin/env python3
"""Native Revert/Undo/Redo retains EOL metadata, future Enter and saved bytes."""
from pathlib import Path
import tempfile
from extension_sessions_pty import command
from pty_smoke import Editor, CTRL_S, CTRL_Z, eventually, wait_screen


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-eol-history-pty-') as directory:
        root = Path(directory).resolve()
        path = root / 'source.cpp'
        original = 'original 猫🙂\r\n'
        external = 'external 猫🙂\n'
        dirty = 'dirty λ ' + original
        path.write_bytes(original.encode())
        app = Editor(root, '--no-session', path, enhanced=True, extra_env={'PATH': ''})
        completed = False
        try:
            app.paste('dirty λ ')
            wait_screen(app, 'dirty λ original 猫🙂', 'source.cpp *', 'CRLF')
            path.write_bytes(external.encode())
            command(app, 'File: Revert File')
            wait_screen(app, 'Revert File', '[Y] Revert')
            app.send(b'y')
            wait_screen(app, 'Reloaded from disk', 'external 猫🙂', absent=('source.cpp *', 'CRLF'))
            app.send(CTRL_Z)
            wait_screen(app, 'dirty λ original 猫🙂', 'source.cpp *', 'CRLF')
            assert path.read_bytes() == external.encode()
            app.send(b'\x1b[122;6u')
            wait_screen(app, 'external 猫🙂', absent=('source.cpp *', 'CRLF'))
            app.send(CTRL_Z)
            wait_screen(app, 'dirty λ original 猫🙂', 'CRLF')
            app.send(b'\x1b[1;5F')  # original Ctrl+End
            app.send(b'\r')
            app.send(CTRL_S)
            saved = dirty + '\r\n'
            eventually(lambda: app.read() and path.read_bytes() == saved.encode())
            wait_screen(app, 'Saved', 'CRLF', absent=('source.cpp *',))
            app.send(CTRL_Z)
            wait_screen(app, 'source.cpp *', 'CRLF')
            assert path.read_bytes() == saved.encode()
            app.send(b'\x1b[122;6u')
            wait_screen(app, 'CRLF', absent=('source.cpp *',))
            app.finish()
            completed = True
        finally:
            if not completed:
                app.finish(discard=True)
    print('PASS: native Revert LF → Undo CRLF → Redo LF → Undo/Enter/Save, '
          'exact Unicode bytes, committed baseline, missing Node and terminal restoration')


if __name__ == '__main__':
    run()
