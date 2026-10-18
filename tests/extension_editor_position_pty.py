#!/usr/bin/env python3
"""Public Position replacements reach native UTF-16 coordinates and history."""
from pathlib import Path
import hashlib
import tempfile
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import command
from profile_migration_pty import finish_after_failure


def run():
    fixture = Path(__file__).resolve().parent / "fixtures/editor-position-extension"
    originals = {path: hashlib.sha256(path.read_bytes()).digest() for path in fixture.iterdir() if path.is_file()}
    with tempfile.TemporaryDirectory(prefix="vscli-editor-position-") as temporary:
        root = Path(temporary)
        file = root / "position.txt"
        original = "λ🙂 original\r\n".encode()
        file.write_bytes(original)
        app = Editor(root, "--extension", fixture, file, enhanced=True)
        try:
            wait_screen(app, "1 commands", timeout=15)
            command(app, "Replace at API position")
            wait_screen(app, "Position replacement applied", timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == "λ🙂α\r\n original\r\ntail".encode())
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert all(hashlib.sha256(path.read_bytes()).digest() == digest for path, digest in originals.items())
        print("PASS: public replace(Position) at UTF-16 and clamped endpoints → native CRLF normalization → actual save/Undo restores exact bytes; fixture unchanged")


if __name__ == "__main__":
    run()
