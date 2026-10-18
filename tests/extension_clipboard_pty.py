#!/usr/bin/env python3
"""Public clipboard/native paste with isolated helper processes, never the personal clipboard."""
from pathlib import Path
import hashlib
import os
import shutil
import sys
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import command
from smart_typing_pty import save, undo, redo
from profile_migration_pty import finish_after_failure

SEED = 'seed 猫🙂\r\n'.encode()
VALUE = 'λ🙂 clipboard\r\nsecond\r\n'.encode()
ORIGINAL = 'original 猫🙂\r\n'.encode()


def run():
    fixture = Path(__file__).resolve().parent / 'fixtures/clipboard-extension'
    originals = {p: hashlib.sha256(p.read_bytes()).digest() for p in fixture.iterdir() if p.is_file()}
    node = shutil.which('node')
    assert node
    with tempfile.TemporaryDirectory(prefix='vscli-clipboard-') as temporary:
        root = Path(temporary)
        helpers = root / 'bin'
        helpers.mkdir()
        clipboard = root / 'clipboard'
        clipboard.write_bytes(SEED)
        helper = "#!/usr/bin/python3\nimport os,pathlib,sys\nfile=pathlib.Path(os.environ['VSCLI_TEST_CLIPBOARD_FILE'])\nname=pathlib.Path(sys.argv[0]).name\nwriting=name in ('wl-copy','pbcopy') or '-in' in sys.argv\nif writing:file.write_bytes(sys.stdin.buffer.read())\nelse:sys.stdout.buffer.write(file.read_bytes())\n"
        helper = helper.replace('#!/usr/bin/python3', '#!' + sys.executable, 1)
        for name in ['wl-copy', 'wl-paste', 'xclip', 'pbcopy', 'pbpaste']:
            path = helpers / name
            path.write_text(helper)
            path.chmod(0o755)
        file = root / 'original.txt'
        file.write_bytes(ORIGINAL)
        app = Editor(root, '--extension-node', node, '--extension', fixture, file, enhanced=True,
                     extra_env={'PATH': str(helpers), 'VSCLI_TEST_CLIPBOARD_FILE': str(clipboard)})
        try:
            wait_screen(app, '4 commands', timeout=15)
            app.send('dirty ')
            command(app, 'Roundtrip extension clipboard')
            wait_screen(app, 'Clipboard roundtrip complete', timeout=15)
            assert clipboard.read_bytes() == VALUE
            assert file.read_bytes() == ORIGINAL
            app.send(b'\x16')
            expected = b'dirty ' + VALUE + ORIGINAL
            save(app, file, expected)
            undo(app, file, b'dirty ' + ORIGINAL)
            redo(app, file, expected)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        app = Editor(root, '--extension-node', root / 'missing-node', file, enhanced=True, extra_env={'PATH': ''})
        try:
            app.send('restart ')
            save(app, file, b'restart ' + expected)
            undo(app, file, expected)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert clipboard.read_bytes() == VALUE
        print('PASS: public clipboard exact Unicode CRLF read/write and native Ctrl+V share isolated helpers; dirty save/Undo/Redo and no-Node restart preserve original work')
    with tempfile.TemporaryDirectory(prefix='vscli-clipboard-unavailable-') as temporary:
        root = Path(temporary)
        file = root / 'original.txt'
        file.write_bytes(ORIGINAL)
        app = Editor(root, '--extension-node', node, '--extension', fixture, file, enhanced=True, extra_env={'PATH': ''})
        try:
            wait_screen(app, '4 commands', timeout=15)
            app.send('retained ')
            command(app, 'Refuse unavailable clipboard')
            wait_screen(app, 'Clipboard backend unavailable', timeout=15)
            assert file.read_bytes() == ORIGINAL
            save(app, file, b'retained ' + ORIGINAL)
            undo(app, file, ORIGINAL)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        print('PASS: unavailable system clipboard rejects read/write while native dirty editing/save/Undo remains usable')
    assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())


if __name__ == '__main__':
    run()
