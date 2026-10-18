#!/usr/bin/env python3
"""Native migration review, live apply/rollback, dirty buffers and explicit package activation."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from pty_smoke import BINARY, CTRL_S, CTRL_Z, Editor, eventually, wait_screen


def command(app, name):
    app.send(b"\x1bOP")
    app.send(name + "\r")


def finish_after_failure(app):
    if app.process.poll() is None:
        app.process.kill()
        app.process.wait()
    app.close_fds()


def fixture(root):
    user = root / "User"
    extensions = root / "source-extensions"
    package = extensions / "fixture.native-1.0.0"
    user.mkdir()
    package.mkdir(parents=True)
    settings = {"editor.tabSize": 2, "workbench.colorCustomizations": {"editor.foreground": "#112233"}}
    (user / "settings.json").write_text("// retain original comments\n" + json.dumps(settings))
    (user / "keybindings.json").write_text(json.dumps([
        {"key": "f6", "command": "type", "args": {"text": " imported"}}]))
    manifest = {"publisher": "fixture", "name": "native", "version": "1.0.0", "main": "main.cjs",
                "engines": {"vscode": "^1.95.0"}, "activationEvents": ["onCommand:fixture.migration"],
                "contributes": {"commands": [{"command": "fixture.migration", "title": "Fixture Migration Edit"}],
                                "keybindings": [{"key": "f9", "command": "fixture.migration"}]}}
    (package / "package.json").write_text(json.dumps(manifest))
    (package / "main.cjs").write_text("""const vscode=require('vscode');
exports.activate=context=>{
 require('node:fs').writeFileSync(process.env.VSCLI_MIGRATION_SENTINEL,'activated');
 context.subscriptions.push(vscode.commands.registerCommand('fixture.migration',()=>{
   const editor=vscode.window.activeTextEditor;
   return editor.edit(edit=>edit.insert(new vscode.Position(0,0),'extension '));
 }));
};
""")
    (extensions / "extensions.json").write_text(json.dumps([{"identifier": {"id": "fixture.native"},
        "version": "1.0.0", "relativeLocation": package.name,
        "location": {"scheme": "file", "path": str(package)}}]))
    return user, extensions


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-profile-migration-") as temporary:
        root = Path(temporary)
        user, extensions = fixture(root)
        originals = {path: hashlib.sha256(path.read_bytes()).hexdigest()
                     for directory in [user, extensions] for path in directory.rglob("*") if path.is_file()}
        native, initial_store = root / "native", root / "initial-store"
        native.mkdir()
        (native / "settings.json").write_text('{"editor.tabSize":8}')
        text, sentinel = root / "document.txt", root / "activated-package"
        original = "λ🙂 original\r\n".encode()
        text.write_bytes(original)
        app = Editor(root, "--config-dir", native, "--extensions-dir", initial_store,
                     "--extension-node", root / "no-node", text, enhanced=True,
                     extra_env={"VSCLI_MIGRATION_SENTINEL": str(sentinel)})
        try:
            app.send(b"\x1b[F")
            app.send(" dirty")
            command(app, "Preferences: Import from VS Code")
            wait_screen(app, "Import from VS Code", "custom User directory")
            app.send("p")
            wait_screen(app, "User directory path")
            app.send(str(user) + "\r")
            wait_screen(app, "Import from VS Code", "Custom VS Code / Default")
            app.send("e")
            wait_screen(app, "extensions directory path")
            app.send(str(extensions) + "\r")
            wait_screen(app, "Review VS Code Import", "fixture.native@1.0.0")
            assert not sentinel.exists()
            app.send("\r")
            wait_screen(app, "VS Code Migration Complete", "1 local extensions", timeout=15)
            assert text.read_bytes() == original
            assert not sentinel.exists()
            app.send(b"\x1b")
            wait_screen(app, "λ🙂 original dirty")
            app.send(b"\x1b[17~")
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == "λ🙂 original dirty imported\r\n".encode())
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == "λ🙂 original dirty\r\n".encode())
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == original)
            command(app, "Preferences: Roll Back VS Code Import")
            wait_screen(app, "VS Code Migration Complete", "Previous profile restored", timeout=15)
            app.send(b"\x1b")
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert not sentinel.exists()
        assert json.loads((native / "settings.json").read_text())["editor.tabSize"] == 8
        print("PASS: native profile review and package copy without Node, live imported shortcuts, dirty Unicode/CRLF save/Undo and rollback")

        arguments = [BINARY, "--import-vscode", str(user), "--vscode-extensions", str(extensions),
                     "--config-dir", str(native), "--import-extensions", "--apply-import", "--activate-imported-extensions"]
        report = json.loads(subprocess.run(arguments, capture_output=True, check=True, timeout=20).stdout)
        assert report["extensions_imported"] == 1
        assert report["extensions_execution_enabled"] == 1
        copied_store = Path(report["activated_profile"]) / "extensions"
        app = Editor(root, "--config-dir", native, "--extensions-dir", copied_store, text, enhanced=True,
                     extra_env={"VSCLI_MIGRATION_SENTINEL": str(sentinel)})
        try:
            app.send(b"\x1bOP")
            app.send("Fixture Migration Edit")
            wait_screen(app, "Fixture Migration Edit", timeout=10)
            app.send("\r")
            eventually(lambda: app.read() and sentinel.exists(), timeout=15)
            wait_screen(app, "extension λ🙂 original", timeout=10)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == b"extension " + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and text.read_bytes() == original)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        for path, digest in originals.items():
            assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, path
        print("PASS: explicit imported execution grant, immutable local package activation, native extension edit/save/Undo and original-source preservation")


if __name__ == "__main__":
    run()
