#!/usr/bin/env python3
"""Native clean-session restart journeys; no JavaScript runtime or external server."""
import json
import os
import signal
import tempfile
import time
from pathlib import Path
from pty_smoke import Editor, eventually, CTRL_S, CTRL_Z

LIVE = []
def editor(root, *args, **kwargs):
    app = Editor(root, *args, enhanced=True, **kwargs)
    LIVE.append(app)
    return app

def finish(app, **kwargs):
    app.finish(**kwargs)
    LIVE.remove(app)

def palette(app, command):
    app.send(b"\x1bOP")
    app.send(command + "\r")

def wait(app, text):
    eventually(lambda: app.read() and text in app.screen.text())

def run():
    with tempfile.TemporaryDirectory(prefix="vscli-session-pty-") as temporary:
        root = Path(temporary)
        config = root / "config"
        first = root / "first.txt"
        second = root / "second.txt"
        first.write_bytes("猫 abc\r\nsecond line\r\n".encode())
        second.write_bytes(b"EXPLICIT\r\n")
        original = first.read_bytes()
        try:
            app = editor(root, first, "--config-dir", config)
            wait(app, "second line")
            app.send(b"\x1b[C" * 2)  # Cursor after 猫 and space.
            palette(app, "View: Split Editor")
            wait(app, "2 · first.txt")
            app.send(b"\x1b[B")
            finish(app)
            app = editor(root, "--config-dir", config, "--restore-session")
            wait(app, "Restored clean-file session")
            assert "1 · first.txt" in app.screen.text() and "2 · first.txt" in app.screen.text()
            app.send("!")
            app.send(CTRL_S)
            eventually(lambda: app.read() and first.read_bytes() == "猫 abc\r\nsec!ond line\r\n".encode())
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and first.read_bytes() == original)
            finish(app)
            print("PASS: opt-in normal restart restores shared groups and active cursor; Unicode/CRLF edit/save/undo preserves bytes")

            first.unlink()
            app = editor(root, "--config-dir", config, "--restore-session")
            wait(app, "previous metadata retained")
            assert "No open editors" in app.screen.text() and not first.exists()
            first.write_bytes(original)
            palette(app, "File: Restore Previous Clean Session")
            wait(app, "Restored clean-file session")
            assert "second line" in app.screen.text()
            finish(app)
            # Explicit files suppress startup restore and retain their initial caret.
            app = editor(root, second, "--config-dir", config, "--restore-session")
            wait(app, "EXPLICIT")
            assert "second line" not in app.screen.text()
            app.send("!")
            app.send(CTRL_S)
            eventually(lambda: app.read() and second.read_bytes() == b"!EXPLICIT\r\n")
            app.send(CTRL_Z)
            app.send(CTRL_S)
            finish(app)
            assert second.read_bytes() == b"EXPLICIT\r\n"
            print("PASS: missing session files create no phantom buffer, retry succeeds, explicit CLI files retain focus and cursor")

            disabled = root / "disabled-config"
            app = editor(root, first, "--config-dir", disabled, "--no-session")
            wait(app, "second line")
            finish(app)
            assert not (disabled / "state" / "sessions").exists()
            app = editor(root, "--config-dir", disabled, "--no-session")
            wait(app, "No open editors")
            palette(app, "File: Restore Previous Clean Session")
            wait(app, "Session storage is disabled or unavailable")
            assert "No open editors" in app.screen.text()
            finish(app)
            assert not (disabled / "state" / "sessions").exists()
            app = editor(root, "--config-dir", config, "--restore-session")
            wait(app, "Restored clean-file session")
            palette(app, "File: Close All Editors")
            wait(app, "No open editors")
            finish(app)
            app = editor(root, "--config-dir", config, "--restore-session")
            wait(app, "Restored clean-file session")
            assert "No open editors" in app.screen.text()
            finish(app)
            print("PASS: --no-session creates no session state; explicit close-all persists an empty workbench")

            # Save a clean layout, then simulate two independently recovered variants.
            app = editor(root, first, "--config-dir", config)
            wait(app, "second line")
            finish(app)
            recovery = root / "recovery"
            recovery.mkdir()
            (recovery / "old.json").write_text(json.dumps({"version": 1, "documents": [
                {"path": str(first), "text": "alpha recovered\r\n", "disk_content": original.decode(), "cursor": 3},
                {"path": str(first), "text": "beta recovered\r\n", "disk_content": original.decode(), "cursor": 2},
            ]}))
            app = editor(root, "--config-dir", config, "--restore-session", "--recovery-dir", recovery, recovery=True)
            wait(app, "Restored clean-file session")
            assert "beta recovered" in app.screen.text()
            app.send("!")
            app.send(CTRL_S)
            eventually(lambda: app.read() and first.read_bytes() == b"be!ta recovered\r\n")
            # Quit asks about the independent alpha buffer; discarding it cannot replace beta.
            finish(app, discard=True)
            assert first.read_bytes() == b"be!ta recovered\r\n"
            for path in (config / "state" / "sessions").glob("*/*.json"):
                metadata = path.read_text()
                assert "alpha recovered" not in metadata and "beta recovered" not in metadata
            print("PASS: duplicate dirty recovery variants keep identity and caret; clean session metadata never stores buffer text")
        finally:
            for app in LIVE[:]:
                try:
                    if app.process.poll() is None:
                        os.kill(app.process.pid, signal.SIGTERM)
                        deadline = time.monotonic() + 3
                        while app.process.poll() is None and time.monotonic() < deadline:
                            app.read()
                            time.sleep(0.02)
                    if app.process.poll() is None:
                        app.process.kill()
                    app.process.wait(timeout=3)
                    app.close_fds()
                finally:
                    LIVE.remove(app)

if __name__ == "__main__":
    run()
