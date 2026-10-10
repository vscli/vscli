#!/usr/bin/env python3
"""Native asynchronous save/autosave journeys through the actual terminal.

No optional language server, JavaScript runtime, session, or recovery process.
Real CLI workers are not artificially held: deterministic pre-commit races are
qualified by Rust tests; these journeys qualify input, disk, and exit outcomes.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from navigation_history_pty import active, open_file
from pty_smoke import CTRL_A, CTRL_S, CTRL_SHIFT_S, CTRL_SHIFT_W, CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import REDO


ESCAPE = b"\x1b[27u"
CLOSE = b"\x17"  # Original Linux Ctrl+W.
NEW = b"\x0e"  # Original Linux Ctrl+N.
ORIGINAL = "猫🙂 e\u0301 original\r\nsecond line\r\n".encode()
SECOND = "β🙂 second\r\n".encode()


def persisted(app, path, expected):
    snapshot = None

    def ready():
        nonlocal snapshot
        app.read()
        snapshot = path.read_bytes() if path.exists() else None
        return snapshot == expected

    try:
        eventually(ready, timeout=6)
    except AssertionError as error:
        raise AssertionError(f"Expected {path.name} bytes {expected!r}, got {snapshot!r}\n"
                             f"{app.screen.text()}") from error


def save(app, path, expected):
    app.send(CTRL_S)
    persisted(app, path, expected)
    wait_screen(app, "Saved", absent=(f"{path.name} *",))


def save_as(app, path, expected):
    app.send(CTRL_SHIFT_S)
    wait_screen(app, "Save As (existing files are protected)")
    app.send(CTRL_A)
    app.paste(str(path))
    app.send(b"\r")
    persisted(app, path, expected)
    active(app, path.name)
    wait_screen(app, "Saved", absent=("Save As (existing files are protected)",))


def undo_redo(app, path, before, after):
    app.send(CTRL_Z)
    save(app, path, before)
    app.send(REDO)
    save(app, path, after)


def exited(app):
    eventually(lambda: app.read() and app.process.poll() is not None, timeout=6)
    assert app.process.returncode == 0, app.output.decode(errors="replace")[-4000:]
    assert app.process.restored, "Save workflow leaked terminal mode"
    app.close_fds()
    app.automation_closed = True


def finish(app, discard=False):
    app.send(CTRL_SHIFT_W)
    if discard:
        def done():
            app.read()
            if app.process.poll() is not None:
                return True
            if "Save changes to " in app.screen.text():
                app.send(b"d")
            return False
        eventually(done, timeout=6)
    exited(app)


@contextmanager
def editor(root, name, *, settings=None, setup=True, discard=False):
    case = root / name
    config = case / "config"
    source, second = case / "main.cpp", case / "other.txt"
    if setup:
        case.mkdir()
        config.mkdir()
        source.write_bytes(ORIGINAL)
        second.write_bytes(SECOND)
        values = {"vscli.languageServer.enabled": False, "breadcrumbs.enabled": False,
                  "files.autoSave": "off"}
        values.update(settings or {})
        (config / "settings.json").write_text(json.dumps(values), encoding="utf-8")
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 "--no-lsp", "--no-session", source, enhanced=True,
                 extra_env={"PATH": ""})
    app.automation_closed = False
    completed = False
    try:
        active(app, source.name, 1)
        yield app, source, second, case
        if not app.automation_closed:
            finish(app, discard=discard)
        completed = True
    except BaseException:
        print(f"Save automation PTY failure ({name}):\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not completed and not app.automation_closed:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, "Failed save workflow leaked terminal mode"
                app.close_fds()
            except Exception as error:
                print(f"Save automation PTY cleanup: {error}", file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-save-automation-pty-") as directory:
        root = Path(directory)
        prefix = "DIRTY猫🙂 \r\n".encode()
        dirty = prefix + ORIGINAL
        with editor(root, "manual-and-save-as") as (app, source, _second, case):
            app.paste(prefix.decode())
            wait_screen(app, "DIRTY猫🙂", "main.cpp *")
            assert source.read_bytes() == ORIGINAL
            save(app, source, dirty)
            undo_redo(app, source, ORIGINAL, dirty)
            destination = case / "renamed.cpp"
            save_as(app, destination, dirty)
            assert source.read_bytes() == dirty
            undo_redo(app, destination, ORIGINAL, dirty)
            assert source.read_bytes() == dirty, "Save As rewrote the original resource"
            # Reopen the persisted resource in the same session and qualify the
            # active model rather than relying on Explorer/tab label matches.
            app.send(CLOSE)
            wait_screen(app, "No open editors")
            open_file(app, destination)
            app.paste("REOPEN ")
            reopened = b"REOPEN " + dirty
            save(app, destination, reopened)
            undo_redo(app, destination, dirty, reopened)
        with editor(root, "manual-and-save-as", setup=False) as (app, source, _second, case):
            assert source.read_bytes() == dirty
            destination = case / "renamed.cpp"
            open_file(app, destination)
            wait_screen(app, "REOPEN", "DIRTY猫🙂")
            assert destination.read_bytes() == b"REOPEN " + dirty
        print("PASS: native async Save/Save As, Unicode/CRLF Undo/Redo, independent old resource, close/reopen and disk restart")

        with editor(root, "typing-and-switch") as (app, source, second, _case):
            open_file(app, second)
            open_file(app, source)
            app.paste("FIRST ")
            # One input delivery exercises Save followed immediately by newer
            # typing. No assertion assumes preparation/authorization timing.
            app.send(CTRL_S + b"LATEST ")
            wait_screen(app, "FIRST LATEST")
            open_file(app, second)
            app.paste("OTHER ")
            save(app, second, b"OTHER " + SECOND)
            open_file(app, source)
            latest = b"FIRST LATEST " + ORIGINAL
            save(app, source, latest)
            app.send(CTRL_Z)
            save(app, source, b"FIRST " + ORIGINAL)
            app.send(REDO)
            save(app, source, latest)
            assert second.read_bytes() == b"OTHER " + SECOND
        print("PASS: Ctrl+S/typing burst and A→B→A retain latest native text, independent model ownership and Undo/Redo")

        with editor(root, "close-save-cancel") as (app, source, _second, _case):
            app.paste(prefix.decode())
            app.send(CLOSE)
            wait_screen(app, "Save changes to main.cpp?")
            app.send(ESCAPE)
            active(app, source.name)
            wait_screen(app, "DIRTY猫🙂", absent=("Save changes to ",))
            assert source.read_bytes() == ORIGINAL
            app.send(CLOSE)
            wait_screen(app, "Save changes to main.cpp?")
            app.send(b"s")
            persisted(app, source, dirty)
            wait_screen(app, "No open editors", absent=("Save changes to ",))
            assert "Untitled" not in app.screen.text()
            open_file(app, source)
            wait_screen(app, "DIRTY猫🙂")
            assert source.read_bytes() == dirty
        print("PASS: dirty Close Cancel preserves bytes; Close Save commits before last-tab welcome and reopening")

        with editor(root, "quit-save") as (app, source, _second, _case):
            app.paste(prefix.decode())
            app.send(CTRL_SHIFT_W)
            wait_screen(app, "Save changes to main.cpp?")
            app.send(b"s")
            exited(app)
            assert source.read_bytes() == dirty, "Quit completed before its actual save receipt"
        with editor(root, "quit-save", setup=False) as (app, source, _second, _case):
            wait_screen(app, "DIRTY猫🙂")
            app.paste("QUEUED ")
            # Quit in the same delivery as Save must either settle persistence
            # directly or ask to save retained newer work; no fixed sleep oracle.
            app.send(CTRL_S + CTRL_SHIFT_W)
            exited(app)
            assert source.read_bytes() == b"QUEUED " + dirty
        print("PASS: Quit Save and immediate Save→Quit exit only after exact bytes persist and terminal mode restores")

        with editor(root, "autosave-root-and-untitled", settings={
            "files.autoSave":"afterDelay", "files.autoSaveDelay":200,
            "[cpp]":{"files.autoSave":"off"},
        }, discard=True) as (app, source, second, case):
            app.paste("CPP OFF ")
            wait_screen(app, "CPP OFF", "main.cpp *")
            app.send(NEW)
            wait_screen(app, "Untitled")
            app.paste("UNSAVED UNTITLED猫🙂")
            open_file(app, second)
            app.paste("AUTO ROOT ")
            persisted(app, second, b"AUTO ROOT " + SECOND)
            wait_screen(app, "Saved", absent=("other.txt *", "Save As (existing files are protected)"))
            # An actual other-model save is the positive observation boundary
            # for the two disabled/unaddressed resources; no negative sleep.
            assert source.read_bytes() == ORIGINAL
            assert not (case / "untitled.txt").exists()
            open_file(app, source)
            wait_screen(app, "CPP OFF", "main.cpp *")
            save(app, source, b"CPP OFF " + ORIGINAL)
        with editor(root, "autosave-root-and-untitled", setup=False) as (app, source, second, _case):
            wait_screen(app, "CPP OFF")
            assert source.read_bytes() == b"CPP OFF " + ORIGINAL
            open_file(app, second)
            wait_screen(app, "AUTO ROOT")
            assert second.read_bytes() == b"AUTO ROOT " + SECOND
        print("PASS: afterDelay root persistence/restart, C++ off override and unsaved Untitled never receives an invented disk path")

        with editor(root, "autosave-language", settings={
            "files.autoSave":"off", "files.autoSaveDelay":200,
            "[cpp]":{"files.autoSave":"afterDelay"},
        }) as (app, source, second, _case):
            open_file(app, second)
            app.paste("TXT OFF ")
            wait_screen(app, "TXT OFF", "other.txt *")
            open_file(app, source)
            app.paste("AUTO CPP ")
            persisted(app, source, b"AUTO CPP " + ORIGINAL)
            wait_screen(app, "Saved", absent=("main.cpp *",))
            assert second.read_bytes() == SECOND
            app.send(CTRL_Z)
            persisted(app, source, ORIGINAL)
            wait_screen(app, "Saved", absent=("main.cpp *",))
            app.send(REDO)
            persisted(app, source, b"AUTO CPP " + ORIGINAL)
            wait_screen(app, "Saved", absent=("main.cpp *",))
            open_file(app, second)
            save(app, second, b"TXT OFF " + SECOND)
        print("PASS: C++ language afterDelay over root off, text-epoch Undo/Redo rescheduling and independent disabled plain-text model")

        with editor(root, "autosave-external-conflict", settings={
            "files.autoSave":"afterDelay", "files.autoSaveDelay":500,
        }) as (app, source, _second, case):
            app.paste(prefix.decode())
            foreign = "FOREIGN猫🙂\r\n".encode()
            source.write_bytes(foreign)
            wait_screen(app, "DIRTY猫🙂", "main.cpp *")
            wait_screen(app, "Save failed", "changed on disk", "main.cpp *", timeout=6)
            assert source.read_bytes() == foreign
            destination = case / "retained.cpp"
            save_as(app, destination, dirty)
            assert source.read_bytes() == foreign
            undo_redo(app, destination, ORIGINAL, dirty)
            assert source.read_bytes() == foreign
        print("PASS: autosave external-change failure preserves foreign disk bytes and dirty Unicode/CRLF model history recoverable through Save As")


if __name__ == "__main__":
    run()
