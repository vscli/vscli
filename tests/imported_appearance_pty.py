#!/usr/bin/env python3
"""Import, render, reload, switch themes and restart without mutating VS Code files."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from pty_smoke import BINARY, CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def command(app, name):
    app.send(b"\x1bOP")
    app.send(name + "\r")


def painted(app, color):
    app.read()
    return any(app.screen.colors.get(cell) == color and char == "λ"
               for cell, char in app.screen.cells.items())


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-imported-appearance-") as temporary:
        root = Path(temporary)
        source = root / "User"
        package = root / "vscode-extensions" / "fixture.theme"
        native = root / "native"
        source.mkdir()
        package.mkdir(parents=True)
        settings = {"workbench.colorTheme": "appearance.id", "workbench.colorCustomizations": {
            "editor.foreground": "#010203", "editor.background": "#050607",
            "[appearance.id]": {"editor.foreground": "#102030"},
            "[VSCLI Light]": {"editor.foreground": "#405060"},
            "statusBar.background": "#aabbcc"}}
        (source / "settings.json").write_text("// preserve source comments\n" + json.dumps(settings))
        (source / "keybindings.json").write_text(json.dumps([
            {"key": "f6", "command": "type", "args": {"text": " imported"}}]))
        (package / "package.json").write_text(json.dumps({"contributes": {"themes": [
            {"id": "appearance.id", "label": "Appearance Display", "path": "theme.json"}]}}))
        (package / "theme.json").write_text('{"include":"base.json","name":"Different File Name"}')
        (package / "base.json").write_text('{"colors":{"editor.foreground":"#abcdef"}}')
        originals = {path: hashlib.sha256(path.read_bytes()).hexdigest()
                     for directory in [source, package] for path in directory.rglob("*") if path.is_file()}
        arguments = [BINARY, "--import-vscode", str(source), "--vscode-extensions", str(package.parent),
                     "--config-dir", str(native)]
        preview = json.loads(subprocess.run(arguments, check=True, capture_output=True, timeout=15).stdout)
        assert preview["activated_profile"] is None and not native.exists()
        applied = json.loads(subprocess.run([*arguments, "--apply-import"], check=True,
                                            capture_output=True, timeout=15).stdout)
        active = Path(applied["activated_profile"])
        text = root / "sample.txt"
        text.write_bytes("λ🙂 original\r\n".encode())
        options = ["--config-dir", native, "--extension-node", root / "missing-node", text]
        app = Editor(root, *options, enhanced=True)
        try:
            eventually(lambda: painted(app, (16, 32, 48)))
            app.send(b"\x1b[17~")  # The imported F6 binding.
            app.send(CTRL_S)
            eventually(lambda: app.read() and b" imported" in text.read_bytes())
            saved = text.read_bytes()
            settings["workbench.colorCustomizations"]["[appearance.id]"]["editor.foreground"] = "#223344"
            (active / "settings.json").write_text(json.dumps(settings))
            eventually(lambda: painted(app, (34, 51, 68)), timeout=10)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == "λ🙂 original\r\n".encode())
            assert saved != text.read_bytes()
            command(app, "Preferences: Color Theme")
            wait_screen(app, "VSCLI Light")
            app.send("VSCLI Light\r")
            eventually(lambda: painted(app, (64, 80, 96)))
            command(app, "Preferences: Color Theme Report")
            wait_screen(app, "statusBar.background")
            app.send(b"\x1b")
            app.finish()
        except BaseException:
            if app.process.poll() is None:
                app.process.kill()
                app.process.wait()
            app.close_fds()
            raise
        app = Editor(root, *options, enhanced=True)
        try:
            eventually(lambda: painted(app, (64, 80, 96)))
            settings.pop("workbench.colorCustomizations")
            (active / "settings.json").write_text(json.dumps(settings))
            eventually(lambda: painted(app, (30, 30, 30)), timeout=10)
            app.finish()
        except BaseException:
            if app.process.poll() is None:
                app.process.kill()
                app.process.wait()
            app.close_fds()
            raise
        for path, digest in originals.items():
            assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, path
        assert text.read_bytes() == "λ🙂 original\r\n".encode()
        print("PASS: imported theme ID/custom colors, bindings, CRLF edit/save/undo, live reload, picker, restart, reset and original-byte preservation without Node")


if __name__ == "__main__":
    run()
