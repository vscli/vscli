#!/usr/bin/env python3
"""Installed clangd save formatting through native automatic service settings.

Run with an already built binary: python3 tests/clangd_format_on_save_pty.py BIN
This is a real clangd qualification, not a desktop-extension parity claim.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from pty_smoke import CTRL_S, CTRL_Z, Editor, wait_screen
from save_automation_pty import persisted
from smart_typing_pty import REDO


CLANGD = Path("/usr/bin/clangd")
DENSE = "int main(){int x=1;return x;}\r\n// 猫🙂 baseline\r\n"
PREFIX = "// native format 猫🙂\r\n"
RAW = (PREFIX + DENSE).encode()
FORMATTED = (PREFIX + "int main() {\r\n  int x = 1;\r\n  return x;\r\n}\r\n"
             "// 猫🙂 baseline\r\n").encode()


def configuration(enabled):
    return {
        "editor.formatOnSave": enabled,
        "editor.formatOnSaveMode": "file",
        "editor.tabSize": 2,
        "editor.insertSpaces": True,
        "files.autoSave": "off",
        "breadcrumbs.enabled": False,
        "[cpp]": {
            "vscli.languageServer.enabled": True,
            "vscli.languageServer.program": str(CLANGD),
            "vscli.languageServer.args": ["--background-index=false", "--clang-tidy=false"],
        },
    }


def launch(root, source, settings):
    # No --lsp argument: the normal native language-service worker resolves the
    # full program path from user settings. PATH cannot locate Node or clangd.
    return Editor(root, "--settings", settings, "--config-dir", root / "config",
                  "--extension-node", root / "missing-node", "--no-session", source,
                  enhanced=True, auto_lsp=True, extra_env={"PATH": ""})


def save_exact(app, source, expected):
    app.send(CTRL_S)
    persisted(app, source, expected)
    snapshot = wait_screen(app, "Saved", absent=("main.cpp *",), timeout=6)
    assert "Formatting skipped" not in snapshot, snapshot


def clean_up_failure(app):
    try:
        if app.process.poll() is None:
            app.process.kill()
        app.process.wait(timeout=3)
        assert app.process.restored, "clangd save workflow leaked terminal mode"
        app.close_fds()
    except Exception as error:
        print(f"clangd save PTY cleanup: {error}", file=sys.stderr)


def run():
    assert CLANGD.is_file(), "Actual qualification requires /usr/bin/clangd"
    version = subprocess.run([str(CLANGD), "--version"], check=True,
                             capture_output=True, text=True, timeout=5).stdout.strip()
    print(f"Actual native formatter: {version}")
    with tempfile.TemporaryDirectory(prefix="vscli-clangd-save-pty-") as directory:
        root = Path(directory)
        source = root / "main.cpp"
        settings = root / "user.json"
        source.write_bytes(DENSE.encode())
        # Pin line-layout knobs as well as indentation so the byte oracle does
        # not depend on clang-format's evolving short-function defaults.
        (root / ".clang-format").write_text(
            "BasedOnStyle: LLVM\nIndentWidth: 2\n"
            "AllowShortFunctionsOnASingleLine: None\n", encoding="utf-8")
        settings.write_text(json.dumps(configuration(True)), encoding="utf-8")
        app = launch(root, source, settings)
        complete = False
        try:
            wait_screen(app, "Language server ready", "main.cpp", timeout=8)
            app.paste(PREFIX)
            wait_screen(app, "native format", "main.cpp *", "int main(){int x=1;return x;}")
            assert source.read_bytes() == DENSE.encode()
            save_exact(app, source, FORMATTED)
            wait_screen(app, "int main() {", "int x = 1;", "return x;", "native format")
            app.send(CTRL_Z)
            wait_screen(app, "int main(){int x=1;return x;}", "main.cpp *", "native format")
            assert source.read_bytes() == FORMATTED, "Undo unexpectedly wrote the backing file"

            # A real settings-loader publication changes the next save policy;
            # the subsequent raw save must preserve the formatter's Redo entry.
            settings.write_text(json.dumps(configuration(False)), encoding="utf-8")
            wait_screen(app, "Settings reloaded", timeout=6)
            save_exact(app, source, RAW)
            app.send(REDO)
            wait_screen(app, "int main() {", "int x = 1;", "main.cpp *")
            assert source.read_bytes() == RAW
            save_exact(app, source, FORMATTED)
            app.send(CTRL_Z)
            wait_screen(app, "int main(){int x=1;return x;}", "main.cpp *")
            save_exact(app, source, RAW)
            app.send(REDO)
            save_exact(app, source, FORMATTED)
            app.finish()
            complete = True
        except BaseException:
            print(f"Actual clangd format-on-save PTY failure:\n{app.screen.text()}", file=sys.stderr)
            raise
        finally:
            if not complete:
                clean_up_failure(app)

        assert source.read_bytes() == FORMATTED
        app = launch(root, source, settings)
        complete = False
        try:
            wait_screen(app, "Language server ready", "int main() {", "int x = 1;",
                        "native format", timeout=8)
            assert source.read_bytes() == FORMATTED
            app.finish()
            complete = True
        finally:
            if not complete:
                clean_up_failure(app)
    print("PASS: actual installed clangd via native user settings, empty PATH/missing Node, "
          "Ctrl+S Unicode/CRLF formatting, one Undo, settings-disabled raw save, Redo and restart")


if __name__ == "__main__":
    run()
