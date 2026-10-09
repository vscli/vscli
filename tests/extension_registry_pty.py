#!/usr/bin/env python3
"""Native Open VSX-compatible browsing/install/update workflows with a local fixture."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import zipfile
from pty_smoke import BINARY, CTRL_S, CTRL_Z, Editor, eventually


def palette(app, command):
    app.send(b"\x1bOP")
    eventually(lambda: app.read() and "Command Palette" in app.screen.text())
    app.send(command + "\r")


def wait(app, text):
    try:
        eventually(lambda: app.read() and text in app.screen.text(), timeout=8)
    except AssertionError:
        raise AssertionError(f"Waiting for {text!r}:\n{app.screen.text()}") from None


def finish(app):
    palette(app, "File: Exit")
    app.process.wait(timeout=3)
    assert app.process.returncode == 0
    assert app.process.restored, "Terminal mode leaked on exit"
    app.close_fds()


def run():
    state = {"version": "1.0.0"}
    held = threading.Event()
    release = threading.Event()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            address = f"http://127.0.0.1:{self.server.server_port}"
            version = state["version"]
            metadata = {"namespace": "fixture", "name": "registry", "version": version,
                        "targetPlatform": "universal", "displayName": "Registry Fixture", "license": "MIT",
                        "description": "Fixture command; installation does not execute code.",
                        "files": {"download": address + "/download", "sha256": address + "/checksum"}}
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                archive.writestr("extension/package.json", json.dumps({"publisher": "fixture", "name": "registry", "version": version,
                    "main": "extension.cjs", "contributes": {"commands": [{"command": "registry.command", "title": "Registry Command"}]}}))
                archive.writestr("extension/extension.cjs", "require('node:fs').writeFileSync('activated.marker','executed'); exports.activate=()=>{};")
            package = buffer.getvalue()
            code = 200
            if self.path.startswith("/api/-/search?"):
                if "query=hold" in self.path:
                    held.set()
                    release.wait(timeout=5)
                # Exercise summaries that omit platform and license.
                summary = dict(metadata)
                summary.pop("targetPlatform")
                summary.pop("license")
                body = json.dumps({"extensions": [summary]}).encode()
            elif self.path == f"/api/fixture/registry/universal/{version}" or self.path == "/api/fixture/registry/universal/latest":
                body = json.dumps(metadata).encode()
            elif self.path == "/download":
                body = package
            elif self.path == "/checksum":
                body = (hashlib.sha256(package).hexdigest() + " fixture.vsix\n").encode()
            else:
                code, body = 404, b"{}"
            self.send_response(code)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    live = []
    try:
        with tempfile.TemporaryDirectory(prefix="vscli-registry-pty-") as temporary:
            root = Path(temporary)
            storage = root / "extensions"
            file = root / "edit.txt"
            original = "native 😀\r\n".encode()
            file.write_bytes(original)
            registry = f"http://127.0.0.1:{server.server_port}"
            app = Editor(root, file, "--extensions-dir", storage, "--extension-registry", registry, "--extension-node", root / "missing-node")
            live.append(app)
            palette(app, "Extensions: Search Open VSX")
            wait(app, "Search Open VSX")
            app.send("registry\r")
            wait(app, "fixture.registry@1.0.0")
            app.send(b"\r")
            wait(app, "Installed fixture.registry@1.0.0")
            assert not (root / "activated.marker").exists()
            installed = json.loads(subprocess.check_output([BINARY, "--list-extensions", "--extensions-dir", str(storage)]))
            original_path = Path(installed[0]["path"])
            assert installed[0]["source"] == registry + "/download"
            app.send(b"\x1b")
            app.send("!")
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == b"!" + original)
            app.send(CTRL_Z)
            app.send(CTRL_S)
            eventually(lambda: app.read() and file.read_bytes() == original)
            state["version"] = "2.0.0"
            palette(app, "Extensions: Check for Updates")
            wait(app, "fixture.registry@2.0.0")
            app.send(b"\r")
            wait(app, "Installed fixture.registry@2.0.0")
            app.send(b"r")
            wait(app, "Restored fixture.registry@1.0.0")
            assert original_path.is_dir()
            assert not (root / "activated.marker").exists()
            app.send(b"\x1b")
            palette(app, "Extensions: Search Open VSX")
            wait(app, "Search Open VSX")
            app.send("hold\r")
            eventually(lambda: app.read() and held.is_set())
            app.send(b"\x1b")
            eventually(lambda: app.read() and "Open VSX" not in app.screen.text() and "Extensions" not in app.screen.text())
            app.send(b"\x10")
            wait(app, "Go to File")
            app.send("later prompt")
            release.set()
            wait(app, "Open VSX results")
            assert "Go to File" in app.screen.text() and "later prompt" in app.screen.text()
            assert "Registry Fixture" not in app.screen.text()
            assert file.read_bytes() == original
            app.send(b"\x1b")
            finish(app)
            live.remove(app)
            print("PASS: native registry browse/download/update/rollback with Node unavailable; save/undo and late-search prompt ownership preserved")
    finally:
        release.set()
        failure = sys.exc_info()[0] is not None
        errors = []
        for app in live:
            try:
                if app.process.poll() is None:
                    os.kill(app.process.pid, signal.SIGTERM)
                    try:
                        app.process.wait(timeout=3)
                    except (AssertionError, subprocess.TimeoutExpired):
                        app.process.kill()
                        app.process.wait(timeout=3)
                app.close_fds()
            except Exception as error:
                errors.append(error)
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)
        if errors:
            if failure:
                print(f"Additional cleanup errors: {errors}", file=sys.stderr)
            else:
                raise errors[0]


if __name__ == "__main__":
    run()
