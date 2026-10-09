#!/usr/bin/env python3
"""Default native LSP startup through real Unix terminal input, without --lsp."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait
from pty_smoke import CTRL_S, CTRL_Z, eventually

HINTS = b"\x1b[32;6u"


def open_file(app, path):
    app.send(b"\x0f")  # Original Linux Ctrl+O.
    wait(app, "Open File (absolute or workspace-relative)")
    app.send(str(path) + "\r")
    eventually(lambda: app.read() and "No open editors" not in app.screen.text()
               and "Open File (absolute or workspace-relative)" not in app.screen.text())


def edit_save_undo(app, path, original):
    app.send(b"\x1b[27;2u")
    app.send(b"\x1b[1;5H")  # Ctrl+Home.
    app.send("//")
    app.send(CTRL_S)
    eventually(lambda: app.read() and path.read_bytes() == b"//" + original)
    app.send(CTRL_Z)
    app.send(CTRL_S)
    eventually(lambda: app.read() and path.read_bytes() == original)


def native(root):
    source = root / "main.cpp"
    original = b"sum(1, 2)\r\n"
    source.write_bytes(original)
    settings = root / "user.json"
    marker = root / "started.pid"
    rust_marker = root / "rust.pid"
    rust = root / "main.rs"
    rust.write_text("fn main() {}\n")
    fixture = Path(__file__).resolve().parent / "fixtures" / "auto_language_server.py"
    settings.write_text(json.dumps({"[cpp]": {"vscli.languageServer.program": sys.executable,
        "vscli.languageServer.args": [str(fixture), str(marker)]},
        "[rust]": {"vscli.languageServer.program": sys.executable,
        "vscli.languageServer.args": [str(fixture), str(rust_marker)]}}))
    config = root / "native-config"
    app = Editor(root, "--settings", settings, "--config-dir", config, root, enhanced=True, auto_lsp=True)
    wait(app, "No open editors")
    assert not marker.exists()
    open_file(app, source)
    wait(app, "Language server ready")
    app.send(HINTS)
    wait(app, "Parameter Hints 1/1")
    wait(app, "int count")
    edit_save_undo(app, source, original)
    open_file(app, rust)
    eventually(lambda: app.read() and rust_marker.exists() and "Language server ready" in app.screen.text())
    app.send(b"\x1b[5;5~")  # Original Ctrl+PageUp selects previous C++ tab.
    eventually(lambda: app.read() and len(marker.read_text().splitlines()) == 2
               and "Language server ready" in app.screen.text())
    marker.with_suffix(".crash").touch()
    wait(app, "Language server stopped")
    marker.with_suffix(".crash").unlink()
    command(app, "Language: Restart Server")
    eventually(lambda: app.read() and len(marker.read_text().splitlines()) == 3
               and "Language server ready" in app.screen.text())
    command(app, "Language: Disable Services")
    wait(app, "Language services disabled")
    app.send(HINTS)
    eventually(lambda: app.read() and "Parameter Hints 1/1" not in app.screen.text())
    command(app, "Language: Enable Services")
    wait(app, "Language server ready")
    app.send(HINTS)
    wait(app, "Parameter Hints 1/1")
    app.send(b"\x1b[27;2u")
    app.finish()
    assert len(marker.read_text().splitlines()) == 4
    # Restoration installs the active clean file asynchronously. Its new identity
    # must start the right server without canceling restoration or losing caret/text.
    app = Editor(root, "--settings", settings, "--config-dir", config,
                 "--restore-session", enhanced=True, auto_lsp=True)
    eventually(lambda: app.read() and len(marker.read_text().splitlines()) == 5
               and "Language server ready" in app.screen.text())
    assert "No open editors" not in app.screen.text()
    assert "sum(1, 2)" in app.screen.text()
    app.send(HINTS)
    wait(app, "Parameter Hints 1/1")
    edit_save_undo(app, source, original)
    app.finish()
    print("PASS: empty startup, automatic C++/Rust tab transition, crash/restart, restored-session hints, disable/enable and CRLF save/undo")

    app = Editor(root, "--settings", settings, "--no-lsp", source, enhanced=True, auto_lsp=True)
    edit_save_undo(app, source, original)
    app.finish()
    assert len(marker.read_text().splitlines()) == 5
    print("PASS: --no-lsp leaves editing/save/undo usable without starting configured server")

    app = Editor(root, source, enhanced=True, auto_lsp=True, extra_env={"PATH": ""})
    wait(app, "clangd is not installed")
    edit_save_undo(app, source, original)
    app.finish()
    print("PASS: missing installed clangd reports useful notice while native editing works with empty PATH")


def real(root):
    source = root / "actual.cpp"
    original = b"int sum(int left, int right); int main(){return sum(1, 2);}\r\n"
    source.write_bytes(original)
    (root / "compile_flags.txt").write_text("-std=c++17\n")
    app = Editor(root, root, enhanced=True, auto_lsp=True)
    wait(app, "No open editors")
    open_file(app, source)
    wait(app, "Language server ready")
    app.send(b"\x07")  # Original Ctrl+G line:column.
    wait(app, "Go to Line")
    app.send(f"1:{original.index(b'1, 2') + 4}\r")
    app.send(HINTS)
    wait(app, "Parameter Hints 1/1")
    wait(app, "int right")
    edit_save_undo(app, source, original)
    app.finish()
    print("PASS: vscli workspace → Ctrl+O C++ → installed clangd → active parameter → native CRLF save/undo, no --lsp")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="vscli-auto-lsp-pty-") as temporary:
        try:
            root = Path(temporary)
            if "--real-clangd" in sys.argv:
                real(root)
            else:
                native(root)
        finally:
            for app in LIVE[:]:
                if app.process.poll() is None:
                    os.kill(app.process.pid, signal.SIGTERM)
                    try:
                        app.process.wait(timeout=6)
                    except subprocess.TimeoutExpired:
                        app.process.kill()
                        app.process.wait(timeout=3)
                app.close_fds()
                LIVE.remove(app)

            for marker in root.glob("*.pid"):
                for value in marker.read_text().splitlines():
                    try:
                        pid = int(value)
                        if os.getpgid(pid) == pid:
                            os.killpg(pid, signal.SIGKILL)
                    except (ProcessLookupError, ValueError):
                        pass
