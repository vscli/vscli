#!/usr/bin/env python3
"""Real Unix PTY coverage for explicitly selected, shared extension sessions."""
import json
import os
import signal
import sys
import time
from pathlib import Path
import subprocess
import tempfile
import zipfile
from pty_smoke import BINARY, CTRL_A, CTRL_S, CTRL_Z, Editor as PtyEditor, eventually, text

LIVE = []
DESCENDANTS = set()


class Editor(PtyEditor):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        LIVE.append(self)

    def finish(self, *args, **kwargs):
        super().finish(*args, **kwargs)
        LIVE.remove(self)


F7, F9, F10 = b"\x1b[18~", b"\x1b[20~", b"\x1b[21~"


def archive(root, name, version="1.0.0", fail=False, hang=False, descendants=False):
    prefix = name + version[0] + " "
    manifest = {
        "publisher": "session", "name": name, "version": version, "main": "extension.cjs",
        "contributes": {
            "commands": [{"command": f"session.{name}.{command}", "title": f"Session {name} {command}"}
                         for command in ("insert", "observe", "stale", "crash")],
            "keybindings": [
                {"key": "f9", "command": f"session.{name}.insert", "when": "editorTextFocus"},
                {"key": "f7", "command": f"session.{name}.stale", "when": "editorTextFocus"},
                {"key": "f10", "command": f"session.{name}.crash", "when": "editorTextFocus"},
            ],
        },
    }
    report = "require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath, 'descendant.pid'), String(descendant.pid));"
    if hang:
        # Exercise the leader-report/child-report gap deterministically.
        report = f"setTimeout(() => {{ {report} }}, 150);"
    spawn = f"""
  const descendant = require('node:child_process').spawn(process.execPath, ['-e', 'setInterval(() => {{}}, 1000)'], {{ stdio: 'inherit' }});
  {report}
""" if descendants else ""
    source = f"""
const vscode = require('vscode');
exports.activate = context => {{
  require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath, '{name}.pid'), String(process.pid));
  {spawn}
  const register = (suffix, call) => context.subscriptions.push(vscode.commands.registerCommand('session.{name}.' + suffix, call));
  register('insert', async () => {{
    const editor = vscode.window.activeTextEditor;
    const applied = await editor.edit(edit => edit.insert(new vscode.Position(0, 0), '{prefix}'));
    await vscode.window.showInformationMessage('{name} inserted=' + applied + ';v' + editor.document.version);
  }});
  register('observe', () => vscode.window.showInformationMessage('{name} sees=' + vscode.window.activeTextEditor.document.getText()));
  register('stale', async () => {{
    const applied = await vscode.window.activeTextEditor.edit(edit => edit.insert(new vscode.Position(0, 0), 'STALE'));
    await vscode.window.showInformationMessage('{name} stale=' + applied);
  }});
  register('crash', () => process.exit(7));
  {"throw new Error('deliberate session failure');" if fail else "return new Promise(() => {});" if hang else ""}
}};
"""
    result = root / f"{name}-{version}.vsix"
    with zipfile.ZipFile(result, "w", zipfile.ZIP_DEFLATED) as writer:
        writer.writestr("extension/package.json", json.dumps(manifest))
        writer.writestr("extension/extension.cjs", source)
    return result


def manage(store, operation, value):
    subprocess.run([BINARY, "--extensions-dir", str(store), operation, str(value)], check=True, capture_output=True)


def wait(app, expected):
    try:
        eventually(lambda: app.read() and expected in app.screen.text(), timeout=8)
    except AssertionError as error:
        raise AssertionError(f"Waiting for {expected!r}:\n{app.screen.text()}") from error


def command(app, query):
    app.send(b"\x1bOP")
    app.send(query + "\r")


def picker(app):
    app.send(b"\x1b[120;6u")
    wait(app, "Installed Extensions")


def activate(app, down=0, count=4):
    picker(app)
    if down:
        app.send(b"\x1b[B" * down)
    app.send(b"\r")
    wait(app, "Run installed extension?")
    app.send(b"\r")
    wait(app, f"({count} commands)")


def save(app, file, expected):
    app.send(CTRL_S)
    eventually(lambda: app.read() and text(file) == expected)


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def executing(pid):
    # Orphan zombies may await container PID1, but execute no code and retain no pipes.
    status = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return bool(status) and not status.startswith("Z")


def stopped_descendant(pid):
    if executing(pid):
        return False
    DESCENDANTS.discard(pid)
    return True


def descendant(root, app, leader=None):
    pid = None

    def ready():
        nonlocal pid
        # The ready status can still describe the preceding host when a restart
        # is queued. Drain the PTY while waiting for the new child so a full
        # output buffer cannot block the editor before it handles that restart.
        app.read()
        try:
            candidate = int((root / "descendant.pid").read_text())
        except (FileNotFoundError, ValueError):
            return False
        # Activation writes its leader PID before spawning/reporting the child.
        # A previous session's report may still be present during this window.
        if candidate in DESCENDANTS or not executing(candidate):
            return False
        try:
            if leader is not None and os.getpgid(candidate) != leader:
                return False
        except ProcessLookupError:
            return False
        pid = candidate
        return True

    try:
        eventually(ready, timeout=8)
    except AssertionError as error:
        raise AssertionError(f"Waiting for a live new descendant:\n{app.screen.text()}") from error
    DESCENDANTS.add(pid)
    return pid


def run(root):
    store = root / "extensions"
    a, z = archive(root, "a"), archive(root, "z")
    manage(store, "--install-extension", a)
    manage(store, "--install-extension", z)
    file = root / "shared.txt"
    file.write_text("original")
    app = Editor(root, "--extensions-dir", store, file, enhanced=True)
    activate(app)
    activate(app, down=1, count=8)
    picker(app)
    wait(app, "Session: session.a@1.0.0 · running 1.0.0")
    wait(app, "Session: session.z@1.0.0 · running 1.0.0")
    app.send(b"\x1b")
    app.send(F9)
    wait(app, "z inserted=true")
    save(app, file, "z1 original")
    command(app, "Session a observe")
    wait(app, "a sees=z1 original")
    app.send(CTRL_Z)
    save(app, file, "original")
    # One input batch executes against the old mirror, then makes a native edit.
    app.send(F7 + b"!")
    wait(app, "z stale=false")
    save(app, file, "!original")
    app.send(CTRL_Z)
    save(app, file, "original")

    manage(store, "--install-extension", archive(root, "a", "2.0.0"))
    picker(app)
    wait(app, "session.a@2.0.0  running 1.0.0")
    app.send(b"h")
    wait(app, "(8 commands)")
    command(app, "Session a insert")
    wait(app, "a inserted=true")
    save(app, file, "a1 original")
    app.send(CTRL_Z)
    save(app, file, "original")
    activate(app, count=8)  # Explicitly replaces only A's selected generation.
    command(app, "Session a insert")
    wait(app, "a inserted=true")
    save(app, file, "a2 original")
    app.send(CTRL_Z)
    save(app, file, "original")
    manage(store, "--rollback-extension", "session.a")
    command(app, "Extensions: Restart Selected Session")
    wait(app, "(8 commands)")
    command(app, "Session a insert")
    wait(app, "a inserted=true")
    save(app, file, "a2 original")
    app.send(CTRL_Z)
    save(app, file, "original")

    manage(store, "--uninstall-extension", "session.z")
    picker(app)
    wait(app, "Session: session.z@1.0.0 · running 1.0.0")
    app.send(b"\x1b")
    command(app, "Extensions: Stop Selected Package")
    wait(app, "Stop Selected Extension ID")
    app.send("session.z\r")
    wait(app, "(4 commands)")
    app.send(F9)
    wait(app, "a inserted=true")
    save(app, file, "a2 original")
    app.send(CTRL_Z)
    save(app, file, "original")
    app.send(F10 + b"!")
    wait(app, "Extension host stopped")
    save(app, file, "!original")
    app.send(CTRL_Z)
    save(app, file, "original")
    command(app, "Extensions: Restart Selected Session")
    wait(app, "(4 commands)")
    child = int((root / "a.pid").read_text())
    app.finish()
    eventually(lambda: not alive(child))
    print("PASS: two installed packages, shared edits/undo/stale rejection, stable precedence, immutable upgrade/rollback snapshots, stop-selected and crash/restart")

    manage(store, "--install-extension", archive(root, "descendants", descendants=True))
    app = Editor(root, "--extensions-dir", store, "--extension", "session.descendants", file, enhanced=True)
    wait(app, "(4 commands)")
    first = descendant(root, app)
    command(app, "Extensions: Restart Selected Session")
    wait(app, "(4 commands)")
    second = descendant(root, app)
    assert second != first
    eventually(lambda: stopped_descendant(first))
    command(app, "Extensions: Stop Host")
    wait(app, "Extension host stopped")
    eventually(lambda: stopped_descendant(second))
    command(app, "Extensions: Restart Selected Session")
    wait(app, "(4 commands)")
    third = descendant(root, app)
    leader = int((root / "descendants.pid").read_text())
    app.send(F10 + b"!")
    wait(app, "Extension host stopped")
    eventually(lambda: stopped_descendant(third) and not alive(leader))
    save(app, file, "!original")
    app.send(CTRL_Z)
    save(app, file, "original")
    app.finish()
    print("PASS: Unix restart, stop and leader crash terminate inherited-stdio descendants while native editing continues")
    manage(store, "--uninstall-extension", "session.descendants")

    # Repeated CLI selectors start the same bounded shared cohort.
    manage(store, "--install-extension", z)
    app = Editor(root, "--extensions-dir", store, "--extension", "session.z", "--extension", "session.a", file, enhanced=True)
    wait(app, "(8 commands)")
    app.send(F9)
    wait(app, "z inserted=true")
    save(app, file, "z1 original")
    app.send(CTRL_Z)
    save(app, file, "original")
    manage(store, "--install-extension", archive(root, "failure", fail=True, descendants=True))
    picker(app)
    app.send(b"\x1b[B\r")  # Sorted installed IDs: a, failure, z.
    wait(app, "Run installed extension?")
    app.send(b"\r")
    wait(app, "deliberate session failure")
    child = int((root / "failure.pid").read_text())
    failed_descendant = int((root / "descendant.pid").read_text())
    if executing(failed_descendant):
        DESCENDANTS.add(failed_descendant)
    eventually(lambda: not alive(child) and stopped_descendant(failed_descendant))
    app.send(b"?")
    save(app, file, "?original")
    app.send(CTRL_Z)
    save(app, file, "original")
    app.finish()
    print("PASS: repeatable CLI selectors and activation failure preserve responsive native editing")

    app = Editor(root, "--extensions-dir", store, "--extension", "session.a", "--extension-node", root / "missing-node", file, enhanced=True)
    wait(app, "Cannot start")
    app.send(b"native ")
    save(app, file, "native original")
    app.finish()
    print("PASS: missing optional Node leaves native edit/save usable")

    manage(store, "--install-extension", archive(root, "hang", hang=True, descendants=True))
    journal = root / "signal-recovery"
    (root / "descendant.pid").unlink(missing_ok=True)
    app = Editor(root, "--extensions-dir", store, "--extension", "session.hang", "--recovery-dir", journal, file, recovery=True, enhanced=True)
    eventually(lambda: app.read() and (root / "hang.pid").exists())
    child = int((root / "hang.pid").read_text())
    pending_descendant = descendant(root, app, leader=child)
    app.send(CTRL_A)
    app.paste("latest unsaved during activation")
    wait(app, "latest unsaved during activation")
    started = time.monotonic()
    os.kill(app.process.pid, signal.SIGTERM)
    eventually(lambda: app.read() and app.process.poll() is not None, timeout=3)
    assert time.monotonic() - started < 3
    assert app.process.returncode != 0 and app.process.restored
    app.close_fds()
    LIVE.remove(app)
    eventually(lambda: not alive(child) and stopped_descendant(pending_descendant))
    snapshots = [json.loads(path.read_text()) for path in journal.glob("*.json")]
    assert len(snapshots) == 1
    assert snapshots[0]["documents"][0]["text"] == "latest unsaved during activation"
    assert text(file) == "native original"
    print("PASS: SIGTERM during pending activation reaps Node, terminates inherited-stdio descendant, preserves latest recovery text and restores the terminal")

if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="vscli-session-pty-") as directory:
        root = Path(directory)
        try:
            run(root)
        finally:
            original_failure = sys.exc_info()[0] is not None
            cleanup_errors = []
            for app in LIVE:
                try:
                    if app.process.poll() is None:
                        try:
                            os.kill(app.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            app.process.wait(timeout=3)
                        except (AssertionError, subprocess.TimeoutExpired):
                            app.process.kill()
                            app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    cleanup_errors.append(error)
            # Collect children only while their fixture leader still owns the group.
            leaders = set()
            for path in root.glob("*.pid"):
                if path.name == "descendant.pid":
                    continue
                try:
                    pid = int(path.read_text())
                    if executing(pid) and os.getpgid(pid) == pid:
                        leaders.add(pid)
                except (ProcessLookupError, FileNotFoundError, ValueError):
                    pass
            try:
                pid = int((root / "descendant.pid").read_text())
                if executing(pid) and os.getpgid(pid) in leaders:
                    DESCENDANTS.add(pid)
            except (ProcessLookupError, FileNotFoundError, ValueError):
                pass
            for pid in leaders:
                try:
                    os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            for pid in list(DESCENDANTS):
                try:
                    if not stopped_descendant(pid):
                        os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    DESCENDANTS.discard(pid)
            if cleanup_errors:
                if original_failure:
                    print(f"Additional PTY cleanup errors: {cleanup_errors}", file=sys.stderr)
                else:
                    raise cleanup_errors[0]
