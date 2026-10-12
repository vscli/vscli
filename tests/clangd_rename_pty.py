#!/usr/bin/env python3
"""Real clangd F2 preparation and grouped text history through native settings."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from pty_smoke import CTRL_A, CTRL_S, CTRL_Z, wait_screen
from save_automation_pty import persisted
from smart_typing_pty import REDO
from clangd_format_on_save_pty import CLANGD, configuration, launch, clean_up_failure

ORIGINAL = 'int value = 1;\r\nint main(){return value;}\r\n// 猫🙂 baseline\r\n'
PREFIX = '// dirty 猫🙂\r\n'
DIRTY = PREFIX + ORIGINAL
RENAMED = DIRTY.replace('value', 'renamed')
F2 = b'\x1bOQ'


def run():
    assert CLANGD.is_file(), 'Qualification requires installed clangd'
    print(subprocess.run([str(CLANGD), '--version'], check=True,
                         capture_output=True, text=True, timeout=5).stdout.strip())
    with tempfile.TemporaryDirectory(prefix='vscli-clangd-rename-') as directory:
        root = Path(directory)
        source, settings = root / 'main.cpp', root / 'user.json'
        source.write_bytes(ORIGINAL.encode())
        values = configuration(False)
        values['files.refactoring.autoSave'] = True  # Single resource: default save rule.
        settings.write_text(json.dumps(values), encoding='utf-8')
        app = launch(root, source, settings)
        completed = False
        try:
            wait_screen(app, 'Language server ready', 'int value = 1;', timeout=10)
            app.paste(PREFIX)
            wait_screen(app, 'dirty 猫🙂', 'main.cpp *')
            app.send(b'\x1b[C' * 4)
            app.send(F2)
            wait_screen(app, 'Rename', 'value', timeout=10)
            app.send(CTRL_A)
            app.paste('renamed')
            app.send(b'\r')
            wait_screen(app, 'int renamed = 1;', 'return renamed;', 'Renamed atomically', timeout=10)
            assert source.read_bytes() == ORIGINAL.encode()
            app.send(CTRL_Z)
            wait_screen(app, 'int value = 1;', 'return value;', absent=('int renamed',))
            assert source.read_bytes() == ORIGINAL.encode()
            app.send(REDO)
            wait_screen(app, 'int renamed = 1;', 'return renamed;')
            app.send(CTRL_S)
            persisted(app, source, RENAMED.encode())
            wait_screen(app, 'Saved', absent=('main.cpp *',))
            app.send(CTRL_Z)
            wait_screen(app, 'int value = 1;', 'return value;', 'main.cpp *')
            assert source.read_bytes() == RENAMED.encode(), 'Undo must retain the committed disk baseline'
            app.send(REDO)
            wait_screen(app, 'int renamed = 1;', 'return renamed;', absent=('main.cpp *',))
            app.finish()
            completed = True
        except BaseException:
            print(f'Actual clangd rename failure:\n{app.screen.text()}', file=sys.stderr)
            raise
        finally:
            if not completed:
                clean_up_failure(app)
    print('PASS: installed clangd, native settings/no --lsp, empty PATH/missing Node, '
          'F2 preparation, both C++ occurrences, grouped Undo/Redo, exact Unicode CRLF '
          'Save, committed baseline and terminal restoration')


if __name__ == '__main__':
    run()
