#!/usr/bin/env python3
"""Original Linux Back/Forward shortcuts against native history and saved bytes."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import CTRL_SHIFT_S, Editor, eventually, wait_screen
from smart_typing_pty import save, undo, redo

BACK = b"\x1b[45;7u"  # Original Linux Ctrl+Alt+-; enhanced terminal delivery.
FORWARD = b"\x1b[45;6u"  # Original Linux Ctrl+Shift+-.
END = b"\x1b[F"
BOTTOM = b"\x1b[1;5F"
TOP = b"\x1b[1;5H"


def original():
    return "".join(f"row {row:02} 猫🙂 value\r\n" for row in range(1, 26)).encode()


def add_at_line_end(contents, row, insertion):
    lines = contents.decode().splitlines(keepends=True)
    assert lines[row - 1].endswith("\r\n")
    lines[row - 1] = lines[row - 1][:-2] + insertion + "\r\n"
    return "".join(lines).encode()


def active(app, name, row=None):
    """Qualify the active editor from its status, not an Explorer/tab label."""
    snapshot = ""

    def ready():
        nonlocal snapshot
        app.read()
        snapshot = app.screen.text().splitlines()[31]
        resource = snapshot.split("|")[0].strip().removesuffix("*").strip()
        return resource == name and (row is None or f"Ln {row}, Col " in snapshot)

    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Waiting for active {name!r}, row {row}:\n{app.screen.text()}") from error


def open_file(app, path):
    app.send(b"\x0f")  # Ctrl+O, original native Open File command.
    wait_screen(app, "Open File (absolute or workspace-relative)")
    app.paste(str(path))
    app.send(b"\r")
    active(app, path.name)


def go_line(app, name, row):
    app.send(b"\x07")  # Ctrl+G.
    wait_screen(app, "Go to Line")
    app.send(str(row) + "\r")
    active(app, name, row)


@contextmanager
def editor(root, name):
    case = root / name
    case.mkdir()
    config = case / "config"
    config.mkdir()
    (config / "settings.json").write_text(json.dumps({"vscli.languageServer.enabled": False}), encoding="utf-8")
    paths = {name: case / name for name in ("first.txt", "second.txt", "third.txt", "fourth.txt")}
    for path in paths.values():
        path.write_bytes(original())
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 paths["first.txt"], enhanced=True, extra_env={"PATH": ""})
    finished = False
    try:
        active(app, "first.txt", 1)
        yield app, paths, case
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


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-navigation-history-pty-") as directory:
        root = Path(directory)
        with editor(root, "dirty-journey") as (app, paths, _case):
            first, second = paths["first.txt"], paths["second.txt"]
            go_line(app, "first.txt", 15)
            app.send(END)
            app.send(" DIRTY")
            dirty = add_at_line_end(original(), 15, " DIRTY")
            open_file(app, second)
            assert first.read_bytes() == original(), "Switching editors saved dirty text"
            app.send(BACK)
            active(app, "first.txt", 15)
            app.send(FORWARD)
            active(app, "second.txt", 1)
            app.send(BACK)
            active(app, "first.txt", 15)
            app.send("!")
            edited = add_at_line_end(original(), 15, " DIRTY!")
            save(app, first, edited)
            undo(app, first, dirty)
            redo(app, first, edited)
            assert second.read_bytes() == original()
        print("PASS: original Ctrl+Alt+- / Ctrl+Shift+- reuse dirty Unicode/CRLF buffers; navigation preserves exact saved positions and Undo/Redo")

        with editor(root, "branch-after-back") as (app, paths, _case):
            open_file(app, paths["second.txt"])
            open_file(app, paths["third.txt"])
            app.send(BACK)
            active(app, "second.txt", 1)
            open_file(app, paths["fourth.txt"])
            app.send(FORWARD)
            # Input ordering plus exact persistence proves the unavailable old
            # Forward destination cannot receive this edit. No target is retried.
            app.send("MARK")
            save(app, paths["fourth.txt"], b"MARK" + original())
            assert paths["third.txt"].read_bytes() == original()
            undo(app, paths["fourth.txt"], original())
            redo(app, paths["fourth.txt"], b"MARK" + original())
        print("PASS: a new editor switch after Back retires the old Forward branch without editing its former target")

        with editor(root, "save-as") as (app, paths, case):
            first, second = paths["first.txt"], paths["second.txt"]
            destination = case / "renamed.txt"
            go_line(app, "first.txt", 15)
            app.send(END)
            app.send(" DIRTY")
            dirty = add_at_line_end(original(), 15, " DIRTY")
            open_file(app, second)
            app.send(BACK)
            active(app, "first.txt", 15)
            app.send(CTRL_SHIFT_S)
            wait_screen(app, "Save As")
            app.paste(str(destination))
            app.send(b"\r")
            active(app, "renamed.txt", 15)
            eventually(lambda: app.read() and destination.read_bytes() == dirty)
            open_file(app, second)
            app.send(BACK)
            active(app, "renamed.txt", 15)
            app.send("!")
            edited = add_at_line_end(original(), 15, " DIRTY!")
            save(app, destination, edited)
            undo(app, destination, dirty)
            redo(app, destination, edited)
            assert first.read_bytes() == original()
            assert second.read_bytes() == original()
        print("PASS: Save As keeps navigation attached to the native model's current path and leaves the old Unicode/CRLF source unchanged")

        with editor(root, "ordinary-motion") as (app, paths, _case):
            app.send(BOTTOM)
            active(app, "first.txt", 26)
            app.send(TOP)
            active(app, "first.txt", 1)
            app.send(BACK)
            active(app, "first.txt", 26)
            app.send(FORWARD)
            active(app, "first.txt", 1)
            app.send(BACK)
            active(app, "first.txt", 26)
            app.send("ENDPOINT")
            save(app, paths["first.txt"], original() + b"ENDPOINT")
            undo(app, paths["first.txt"], original())
            redo(app, paths["first.txt"], original() + b"ENDPOINT")
        print("PASS: ordinary Ctrl+End / Ctrl+Home movement records large line journeys and original Back/Forward restores exact native edit endpoints")


if __name__ == "__main__":
    run()
