#!/usr/bin/env python3
"""Installed native configuration through original commands and physical input."""
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

from pty_smoke import BINARY, Editor, eventually, wait_screen
from smart_typing_pty import save, undo, redo

ESC = b"\x1b"
END = b"\x1b[F"
LEFT = b"\x1b[D"
SELECT_LEFT = b"\x1b[1;2D"
ADD_COMMENT = b"\x0b\x03"  # Linux Ctrl+K Ctrl+C, original command.
REMOVE_COMMENT = b"\x0b\x15"  # Linux Ctrl+K Ctrl+U.
BLOCK_COMMENT = b"\x1b[97;6u"  # Enhanced Ctrl+Shift+A.
DELETE = b"\x1b[3~"
LOADED = "language configuration loaded; code disabled"
READY = "language configuration ready"


def command(app, query):
    app.send(b"\x1bOP")
    app.send(query + "\r")


def archive(root, version="1.0.0", opening="«", closing="»"):
    path = root / f"native-{version}.vsix"
    manifest = {"publisher": "fixture", "name": "native", "version": version,
                "main": "main.cjs", "activationEvents": ["*"],
                "engines": {"vscode": "^1.95.0"},
                "contributes": {"languages": [{"id": "cpp", "configuration": "cpp.json"}]}}
    configuration = {"autoClosingPairs": [{"open": opening, "close": closing}],
                     "surroundingPairs": [[opening, closing]], "brackets": [[opening, closing]],
                     "autoCloseBefore": "!", "comments": {"lineComment": "#", "blockComment": ["<!--", "-->"]}}
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as package:
        package.writestr("extension/package.json", json.dumps(manifest))
        package.writestr("extension/cpp.json", json.dumps(configuration))
        package.writestr("extension/main.cjs", "require('node:fs').writeFileSync('executed-package-code','unexpected');throw new Error('must not execute');")
    return path


def install_cli(root, store, package):
    subprocess.run([BINARY, "--extensions-dir", str(store), "--install-extension", str(package)],
                   cwd=root, check=True, capture_output=True, timeout=15)


@contextmanager
def editor(root, name, original, setup=True):
    case = root / name
    if setup:
        case.mkdir()
        store, config = case / "store", case / "config"
        config.mkdir()
        (config / "settings.json").write_text(json.dumps({"vscli.languageServer.enabled": False,
            "editor.autoClosingBrackets": "languageDefined", "editor.autoIndent": "brackets"}), encoding="utf-8")
        install_cli(case, store, archive(case))
        (case / "main.cpp").write_bytes(original)
    store, config, source = case / "store", case / "config", case / "main.cpp"
    app = Editor(case, "--config-dir", config, "--extensions-dir", store,
                 "--extension-node", case / "missing-node", source, enhanced=True, extra_env={"PATH": ""})
    finished = False
    try:
        yield app, source, case, store
        assert not (case / "executed-package-code").exists(), "Declarative loading executed package code"
        app.finish()
        finished = True
    except BaseException:
        print(f"PTY failure in {name}:\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not finished:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                app.close_fds()
            except Exception as error:
                print(f"PTY cleanup failed: {error}", file=sys.stderr)


def ready(app, version="1.0.0"):
    # The status qualifies the current installed source generation. It is a
    # separate publication witness; no target typing is retried for an outcome.
    command(app, "Extensions: Show Installed Extensions")
    wait_screen(app, "Installed Extensions")
    wait_screen(app, f"fixture.native@{version}")
    wait_screen(app, LOADED)
    wait_screen(app, READY)
    app.send(ESC)
    eventually(lambda: app.read() and "Installed Extensions" not in app.screen.text())


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-native-configuration-pty-") as directory:
        root = Path(directory)
        original = "猫🙂 X\r\n".encode()
        with editor(root, "pairing", original) as (app, source, case, _store):
            ready(app)
            app.send(END + LEFT)
            app.send("«")
            save(app, source, "猫🙂 «X\r\n".encode())
            undo(app, source, original)
            app.send(END)
            app.send("«")
            save(app, source, "猫🙂 X«»\r\n".encode())
            app.send("»")
            save(app, source, "猫🙂 X«»\r\n".encode())
            undo(app, source, original)
            redo(app, source, "猫🙂 X«»\r\n".encode())
            assert not (case / "config" / "extensions-enabled.json").exists()
        print("PASS: installed data loads without a code grant or Node; custom Unicode pair, autoCloseBefore exclusion and CRLF EOL admission preserve exact Undo/Redo bytes")

        original = "猫🙂 value\r\n".encode()
        with editor(root, "comments", original) as (app, source, _case, _store):
            ready(app)
            app.send(ADD_COMMENT)
            commented = "# 猫🙂 value\r\n".encode()
            save(app, source, commented)
            undo(app, source, original)
            redo(app, source, commented)
            app.send(REMOVE_COMMENT)
            save(app, source, original)
            app.send(END + SELECT_LEFT * 5 + BLOCK_COMMENT)
            block = "猫🙂 <!-- value -->\r\n".encode()
            save(app, source, block)
            undo(app, source, original)
            redo(app, source, block)
            app.send(BLOCK_COMMENT)
            save(app, source, original)
        print("PASS: original Ctrl+K Ctrl+C/U and enhanced Linux Ctrl+Shift+A use installed comment delimiters with exact Unicode/CRLF save, Undo and Redo")

        original = "猫🙂 \r\n".encode()
        with editor(root, "disabled", original) as (app, source, case, _store):
            ready(app)
            app.send(END)
            app.send("«")
            save(app, source, "猫🙂 «»\r\n".encode())
            command(app, "Extensions: Show Installed Extensions")
            wait_screen(app, "Installed Extensions")
            app.send("d")
            wait_screen(app, "Disabled fixture.native;")
            wait_screen(app, READY)
            assert json.loads((case / "config" / "extensions-enabled.json").read_text())["extensions"] == {"fixture.native": False}
            app.send(ESC)
            app.send("»")
            save(app, source, "猫🙂 «»»\r\n".encode())
            undo(app, source, "猫🙂 «»\r\n".encode())
            redo(app, source, "猫🙂 «»»\r\n".encode())
        with editor(root, "disabled", b"", setup=False) as (app, source, _case, _store):
            command(app, "Extensions: Show Installed Extensions")
            wait_screen(app, "Installed Extensions")
            wait_screen(app, "disabled")
            wait_screen(app, READY)
            assert LOADED not in app.screen.text()
            app.send(ESC)
            app.send(END)
            app.send("«")
            save(app, source, "猫🙂 «»»«\r\n".encode())
            undo(app, source, "猫🙂 «»»\r\n".encode())
        print("PASS: explicit disable retires live generated ownership and remains disabled after restart without altering native history")

        with editor(root, "lifecycle", b"\r\n") as (app, source, case, _store):
            ready(app)
            app.send("«")
            save(app, source, "«»\r\n".encode())
            update = archive(case, "2.0.0", "[", "]")
            command(app, "Extensions: Install from VSIX")
            wait_screen(app, "Install Extension from local VSIX")
            app.paste(str(update))
            app.send(b"\r")
            wait_screen(app, "Installed fixture.native@2.0.0")
            wait_screen(app, LOADED)
            wait_screen(app, READY)
            app.send(ESC)
            app.send("»")
            save(app, source, "«»»\r\n".encode())
            undo(app, source, "«»\r\n".encode())
            app.send(END)
            app.send("[")
            save(app, source, "«»[]\r\n".encode())
            command(app, "Extensions: Show Installed Extensions")
            wait_screen(app, "Installed Extensions")
            app.send(DELETE)
            wait_screen(app, "Uninstalled fixture.native;")
            wait_screen(app, READY)
            app.send(ESC)
            app.send("]")
            save(app, source, "«»[]]\r\n".encode())
            undo(app, source, "«»[]\r\n".encode())
            redo(app, source, "«»[]]\r\n".encode())
        print("PASS: physical VSIX update and uninstall publish new native bindings and retire generated ownership while retaining exact CRLF save/Undo/Redo")


if __name__ == "__main__":
    run()
