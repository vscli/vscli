#!/usr/bin/env python3
"""Source application selection, built-in native data, disabled data and dirty migration."""
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
from profile_migration_pty import fixture, command, finish_after_failure
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-builtin-migration-") as temporary:
        root = Path(temporary)
        user, extensions = fixture(root)
        (user / "keybindings.json").write_text(json.dumps([
            {"key": "f8", "command": "editor.action.insertSnippet", "args": {"name": "Builtin snippet"}}]))
        application = root / "application/resources/app"
        (application / "extensions").mkdir(parents=True)
        (application / "package.json").write_text('{"version":"1.118.0"}')
        (application / "product.json").write_text('{"applicationName":"code"}')
        for name, body in [("data", "BUILTIN "), ("disabled", "DISABLED ")]:
            package = application / "extensions" / name
            package.mkdir()
            (package / "package.json").write_text(json.dumps({
                "publisher": "fixture", "name": name, "version": "1.0.0", "main": "main.cjs",
                "contributes": {"snippets": [{"language": "plaintext", "path": "snippets.json"}]}}))
            (package / "main.cjs").write_text("throw new Error('built-in code must remain ungranted')")
            (package / "snippets.json").write_text(json.dumps({
                "Builtin snippet": {"prefix": "builtin", "body": body}}))
        (user / "globalStorage").mkdir()
        with sqlite3.connect(user / "globalStorage/state.vscdb") as database:
            database.execute("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            database.execute("INSERT INTO ItemTable VALUES (?, ?)",
                             ("extensionsIdentifiers/disabled", '[{"id":"fixture.disabled"}]'))
        originals = {path: hashlib.sha256(path.read_bytes()).hexdigest()
                     for directory in [user, extensions, application]
                     for path in directory.rglob("*") if path.is_file()}
        native, initial_store = root / "native", root / "initial-store"
        native.mkdir()
        text = root / "document.txt"
        original = "λ🙂 original\r\n".encode()
        text.write_bytes(original)
        app = Editor(root, "--config-dir", native, "--extensions-dir", initial_store,
                     "--extension-node", root / "no-node", text, enhanced=True)
        try:
            app.send("dirty ")
            command(app, "Preferences: Import from VS Code")
            wait_screen(app, "custom User directory")
            app.send("p")
            wait_screen(app, "User directory path")
            app.send(str(user) + "\r")
            wait_screen(app, "Custom VS Code / Default")
            app.send("e")
            wait_screen(app, "extensions directory path")
            app.send(str(extensions) + "\r")
            wait_screen(app, "Review VS Code Import", "fixture.native@1.0.0")
            app.send("b")
            wait_screen(app, "Custom VS Code / Default", "application/built-ins")
            app.send("b")
            wait_screen(app, "application or resources/app path")
            app.send(str(application) + "\r")
            wait_screen(app, "Review VS Code Import", "3 selected IDs", "2 source built-ins")
            app.send("\r")
            wait_screen(app, "VS Code Migration Complete", "3 local extensions", "0 code execution grants", timeout=15)
            assert text.read_bytes() == original
            app.send(b"\x1b")
            wait_screen(app, "dirty λ🙂 original")
            app.send(b"\x1b[19~")
            wait_screen(app, "dirty BUILTIN λ🙂 original", timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == b"dirty BUILTIN " + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == b"dirty " + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == original)
            command(app, "Preferences: Roll Back VS Code Import")
            wait_screen(app, "Previous profile restored", timeout=15)
            app.send(b"\x1b")
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        for path, digest in originals.items():
            assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, path
        assert not (user / "globalStorage/state.vscdb-wal").exists()
        assert not (user / "globalStorage/state.vscdb-shm").exists()
        print("PASS: native application picker → built-in snapshot → source-disabled snippets hidden → ungranted native snippet data → dirty Unicode/CRLF save/Undo → live rollback; originals unchanged")


if __name__ == "__main__":
    run()
