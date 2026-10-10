#!/usr/bin/env python3
"""An original Escape followed by F1 must survive one real terminal read."""
from pathlib import Path
import sys
import tempfile

from pty_smoke import CTRL_S, Editor, eventually, wait_screen


def run(enhanced):
    with tempfile.TemporaryDirectory(prefix="vscli-adjacent-escape-") as directory:
        root = Path(directory)
        source = root / "protected.txt"
        original = "disk\r\n".encode()
        edited = "unsaved🙂disk\r\n".encode()
        source.write_bytes(original)
        app = Editor(root, source, enhanced=enhanced)
        finished = False
        try:
            app.paste("unsaved🙂")
            wait_screen(app, "unsaved🙂disk")
            app.send(b"\x1bOP")
            wait_screen(app, "Command Palette")
            app.send("Keyboard Inspector\r")
            wait_screen(app, "Keyboard Inspector · Esc closes")

            # One write deliberately coalesces the dismissing Escape and the
            # original SS3 F1. Neither may consume the other's prefix.
            app.send(b"\x1b\x1bOP")
            wait_screen(app, "Command Palette", absent=("Keyboard Inspector · Esc closes",))
            assert source.read_bytes() == original
            app.send("Keyboard Inspector\r")
            wait_screen(app, "Keyboard Inspector · Esc closes")
            app.send(b"\x1b")
            wait_screen(app, "unsaved🙂disk", absent=("Keyboard Inspector · Esc closes",))

            # Exercise the same pair in editor focus, with the query in the
            # same batch. The ordinary document must never receive OP/query.
            app.send(b"\x1b\x1bOPKeyboard Inspector\r")
            wait_screen(app, "Keyboard Inspector · Esc closes")
            app.send(b"\x1b")
            wait_screen(app, "unsaved🙂disk", absent=("Keyboard Inspector · Esc closes",))
            app.send(CTRL_S)
            eventually(lambda: app.read() and source.read_bytes() == edited)
            app.finish()
            finished = True
            print(f"PASS: {'enhanced' if enhanced else 'legacy'} adjacent Escape/F1, modal and editor focus, Unicode/CRLF unsaved integrity")
        finally:
            if not finished:
                try:
                    app.process.kill()
                    app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    print(f"PTY cleanup failed: {error}", file=sys.stderr)


if __name__ == "__main__":
    run(False)
    run(True)
