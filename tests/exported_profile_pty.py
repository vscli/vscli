#!/usr/bin/env python3
"""Exported profile review, local package reconciliation and retained native editing."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from pty_smoke import BINARY, CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from imported_appearance_pty import painted
from profile_migration_pty import command, finish_after_failure, fixture


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-exported-profile-") as temporary:
        root = Path(temporary)
        user, extensions = fixture(root)
        app_root = root / "application/resources/app"
        theme = app_root / "extensions/export-theme"
        theme.mkdir(parents=True)
        (app_root / "package.json").write_text('{"version":"1.118.0"}')
        (app_root / "product.json").write_text('{"applicationName":"code"}')
        (theme / "package.json").write_text(json.dumps({"publisher": "fixture", "name": "export-theme", "version": "1.0.0",
            "contributes": {"themes": [{"id": "fixture.export.theme", "label": "Export theme", "path": "theme.json"}]}}))
        (theme / "theme.json").write_text('{"colors":{"editor.foreground":"#1a2b3c"}}')
        settings = '// export comments\r\n{"editor.tabSize":2,"workbench.colorTheme":"fixture.export.theme","unknown.setting":true,}\r\n'
        keys = '// export keys\r\n' + json.dumps([{"key": "f6", "command": "type", "args": {"text": " imported"},
            "when": "editorTextFocus && resourceExtname == .txt"}]) + '\r\n'
        snippet = '// original snippet\r\n{"Lambda":{"prefix":"lam","body":"λ🙂$0"}}'
        export = root / "personal.code-profile"
        content = {"name": "Personal λ🙂", "settings": json.dumps({"settings": settings}),
            "keybindings": json.dumps({"platform": 2, "keybindings": keys}),
            "snippets": json.dumps({"snippets": {"global.code-snippets": snippet}}),
            "extensions": json.dumps([{"identifier": {"id": "fixture.native"}, "version": "1.0.0", "disabled": True}]),
            "globalState": json.dumps({"storage": {"unknown": "preserved"}}), "future": {"retain": True}}
        export.write_bytes(json.dumps(content, indent=2, ensure_ascii=False).encode())
        originals = {path: hashlib.sha256(path.read_bytes()).digest()
            for directory in [user, extensions, app_root] for path in directory.rglob("*") if path.is_file()}
        originals[export] = hashlib.sha256(export.read_bytes()).digest()
        native = root / "native"
        native.mkdir()
        (native / "settings.json").write_text('{"editor.tabSize":8}')
        document = root / "document.txt"
        original = "λ🙂 original\r\n".encode()
        document.write_bytes(original)
        sentinel = root / "must-not-activate"
        app = Editor(root, "--config-dir", native, "--extensions-dir", root / "empty-store",
            "--extension-node", root / "no-node", document, enhanced=True,
            extra_env={"VSCLI_MIGRATION_SENTINEL": str(sentinel)})
        try:
            app.send(b"\x1b[F")
            app.send(" dirty")
            command(app, "Preferences: Import from VS Code")
            wait_screen(app, "Import from VS Code", "custom User directory")
            app.send("p")
            wait_screen(app, "User directory path")
            app.send(str(export) + "\r")
            wait_screen(app, "Review VS Code Import", "personal.code-profile", timeout=15)
            app.send("b")
            wait_screen(app, "Import from VS Code", "Personal λ🙂")
            app.send("e")
            wait_screen(app, "extensions directory path")
            app.send(str(extensions) + "\r")
            wait_screen(app, "Review VS Code Import", "fixture.native@1.0.0", timeout=15)
            app.send("b")
            wait_screen(app, "Import from VS Code", "Personal λ🙂")
            app.send("b")
            wait_screen(app, "application")
            app.send(str(app_root) + "\r")
            wait_screen(app, "Review VS Code Import", "1 source built-ins", timeout=15)
            app.send("\r")
            wait_screen(app, "VS Code Migration Complete", "2 local extensions", timeout=20)
            assert document.read_bytes() == original
            assert not sentinel.exists()
            app.send(b"\x1b")
            eventually(lambda: painted(app, (26, 43, 60)), timeout=10)
            wait_screen(app, "λ🙂 original dirty")
            app.send(b"\x1b[17~")
            app.send(CTRL_S)
            eventually(lambda: app.read() and document.read_bytes() == "λ🙂 original dirty imported\r\n".encode())
            for expected in ["λ🙂 original dirty\r\n".encode(), original]:
                app.send(CTRL_Z)
                app.send(CTRL_S)
                eventually(lambda: app.read() and document.read_bytes() == expected)
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        active = native / "imports" / json.loads((native / "active-profile.json").read_text())["profile"]
        for name, expected in [("settings.json", settings.encode()), ("keybindings.json", keys.encode()),
                ("snippets/global.code-snippets", snippet.encode()), ("source.code-profile", export.read_bytes())]:
            assert (active / name).read_bytes() == expected
        grants = json.loads((active / "extensions-enabled.json").read_text())
        assert not any(grants.get("extensions", {}).values())
        app = Editor(root, "--config-dir", native, "--extension-node", root / "no-node", document, enhanced=True)
        try:
            eventually(lambda: painted(app, (26, 43, 60)), timeout=10)
            app.send(b"\x1b[F")
            app.send(b"\x1b[17~")
            app.send(CTRL_S)
            eventually(lambda: app.read() and document.read_bytes() == "λ🙂 original imported\r\n".encode())
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and document.read_bytes() == original)
            command(app, "Preferences: Roll Back VS Code Import")
            wait_screen(app, "VS Code Migration Complete", "Previous profile restored", timeout=15)
            app.send(b"\x1b")
            app.finish()
        except BaseException:
            finish_after_failure(app)
            raise
        assert not (native / "active-profile.json").exists()
        assert not sentinel.exists()
        assert all(hashlib.sha256(path.read_bytes()).digest() == digest for path, digest in originals.items())
        print("PASS: exported profile native review, exact disabled package and built-ins, retained dirty Unicode/CRLF save/Undo, no-Node theme/shortcut restart and rollback; byte-exact originals and zero grants")

        arguments = [BINARY, "--import-vscode", str(export), "--vscode-extensions", str(extensions),
            "--vscode-application", str(app_root), "--config-dir", str(root / "cli-native")]
        report = json.loads(subprocess.run(arguments, check=True, capture_output=True, timeout=20).stdout)
        assert not report["unresolved_extensions"] and len(report["extensions"]["selected"]) == 2
        assert not (root / "cli-native").exists()
        content["extensions"] = json.dumps([{"identifier": {"id": "fixture.missing"}, "version": "9.0.0"}])
        export.write_bytes(json.dumps(content).encode())
        failed = subprocess.run(arguments + ["--apply-import", "--import-extensions"], capture_output=True, timeout=20)
        assert failed.returncode != 0 and b"fixture.missing@9.0.0" in failed.stderr
        assert not (root / "cli-native").exists()
        print("PASS: exported CLI preview is read-only and unresolved pinned packages block every destination write")


if __name__ == "__main__":
    run()
