#!/usr/bin/env python3
"""Native clean-session restart journeys; no JavaScript runtime or external server."""
import json
import os
import signal
import sys
import tempfile
import time
from pathlib import Path
from pty_smoke import Editor, eventually, wait_screen, CTRL_S, CTRL_Z

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

def combined(root):
    folder = root / "combined"
    folder.mkdir()
    config = folder / "config"
    file = folder / "other.cpp"
    original = "x 😀foo\r\n".encode()
    file.write_bytes(original)
    extension = folder / "extension"
    extension.mkdir()
    (extension / "package.json").write_text(json.dumps({
        "publisher": "fixture", "name": "session-prompt", "version": "1.0.0", "main": "extension.cjs",
        "contributes": {"commands": [{"command": "session.prompt", "title": "Session Prompt"}]},
    }))
    (extension / "extension.cjs").write_text(r"""
const vscode = require('vscode');
exports.activate = context => context.subscriptions.push(vscode.commands.registerCommand('session.prompt', async () => {
 const answer = vscode.window.showInputBox({title:'Session interruption'});
 require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath,'prompt.queued'), 'queued');
 await vscode.window.showInformationMessage('session answer='+await answer);
}));
""")
    bindings = folder / "bindings.json"
    bindings.write_text(json.dumps([{"key": "f6", "command": "session.prompt"}, {"key": "f7", "command": "vscli.session.restore"}]))
    server = Path(__file__).resolve().parent / "fixtures" / "symbol_server.py"
    app = editor(folder, file, "--config-dir", config)
    wait(app, "😀foo")
    finish(app)
    options = ("--config-dir", config, "--extension", extension, "--keybindings", bindings,
               "--lsp", sys.executable, "--lsp-arg", server, "--lsp-language", "cpp")
    app = editor(folder, *options)
    wait(app, "(1 commands)")
    # Establish actual provider readiness without depending on relative process startup speed.
    deadline = time.monotonic() + 5
    while "Go to Symbol in Workspace" not in app.screen.text():
        assert time.monotonic() < deadline, app.screen.text()
        app.send(b"\x14")
    app.send(b"\x1b")
    # Keep the extension request queued behind a native palette, then dispatch Restore.
    app.send(b"\x1b[17~\x1bOP")
    eventually(lambda: app.read() and (folder / "prompt.queued").exists())
    app.send("File: Restore Previous Clean Session\r")
    wait(app, "Session interruption")
    assert "No open editors" in app.screen.text(), app.screen.text()
    app.send(b"\x1b")
    wait(app, "session answer=undefined")
    assert "No open editors" in app.screen.text()
    # One input batch starts Restore and immediately replaces it with the symbol picker.
    app.send(b"\x1b[18~\x14")
    wait(app, "Go to Symbol in Workspace")
    # Symbol replies legitimately replace the transient cancellation status.
    # Require a rendered result row and the still-empty editor instead.
    wait(app, "other  [Function]  fixture")
    assert "No open editors" in app.screen.text()
    app.send(b"\x1b")
    app.send(b"\x1b[18~")
    wait(app, "Restored clean-file session")
    assert "😀foo" in app.screen.text()
    app.send("!")
    app.send(CTRL_S)
    eventually(lambda: app.read() and file.read_bytes() == b"!" + original)
    app.send(CTRL_Z)
    app.send(CTRL_S)
    finish(app)
    assert file.read_bytes() == original
    disabled = folder / "disabled"
    app = editor(folder, "--config-dir", disabled, "--no-session", "--extension", extension,
                 "--keybindings", bindings, "--lsp", sys.executable, "--lsp-arg", server, "--lsp-language", "cpp")
    wait(app, "(1 commands)")
    app.send(b"\x1b[18~")
    wait(app, "Session storage is disabled or unavailable")
    app.send(b"\x1b[17~")
    wait(app, "Session interruption")
    app.send(b"\x1b")
    wait(app, "session answer=undefined")
    deadline = time.monotonic() + 5
    while "Go to Symbol in Workspace" not in app.screen.text():
        assert time.monotonic() < deadline, app.screen.text()
        app.send(b"\x14")
    app.send(b"\x1b")
    assert "No open editors" in app.screen.text()
    finish(app)
    assert not (disabled / "state" / "sessions").exists()
    assert file.read_bytes() == original
    print("PASS: queued extension InputBox and workspace-symbol picker cancel restore; retry preserves save/undo, and --no-session preserves dialogs/symbols")


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
            wait_screen(app, "previous metadata retained", "No open editors")
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
            # Native recovery captures Document paths, which are canonical even
            # when the temporary workspace itself has a macOS /var alias.
            recovered_path = str(first.resolve(strict=True))
            (recovery / "old.json").write_text(json.dumps({"version": 1, "documents": [
                {"path": recovered_path, "text": "alpha recovered\r\n", "disk_content": original.decode(), "cursor": 3},
                {"path": recovered_path, "text": "beta recovered\r\n", "disk_content": original.decode(), "cursor": 2},
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
            combined(root)
        finally:
            original_failure = sys.exc_info()[0] is not None
            cleanup_errors = []
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
                    # Continue draining after the editor's status report: on
                    # macOS the supervising session can still be closing its PTY.
                    deadline = time.monotonic() + 3
                    while app.process.supervisor.poll() is None and time.monotonic() < deadline:
                        app.read()
                        time.sleep(0.02)
                    if app.process.supervisor.poll() is None:
                        app.process.supervisor.kill()
                    app.process.wait(timeout=3)
                except Exception as error:
                    cleanup_errors.append(error)
                finally:
                    try:
                        app.close_fds()
                    except Exception as error:
                        cleanup_errors.append(error)
                    LIVE.remove(app)
            if cleanup_errors:
                if not original_failure:
                    raise cleanup_errors[0]
                for error in cleanup_errors:
                    print(f"Cleanup error after original failure: {error}", file=sys.stderr)

if __name__ == "__main__":
    run()
