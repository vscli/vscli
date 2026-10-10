#!/usr/bin/env python3
"""Native smart typing through real terminal input, with no executables on PATH."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen


REDO = b"\x19"  # Linux Ctrl+Y; the helper fixes the keyboard profile on every OS.
BACKSPACE = b"\x7f"
RIGHT = b"\x1b[C"
END = b"\x1b[F"
SELECT_LEFT = b"\x1b[1;2D"


@contextmanager
def editor(root, name, original, settings=None):
    case = root / name
    case.mkdir()
    source = case / "main.cpp"
    source.write_bytes(original)
    config = case / "config"
    config.mkdir()
    if settings is not None:
        (config / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 source, enhanced=True, extra_env={"PATH": ""})
    finished = False
    try:
        yield app, source
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


def save(app, source, expected):
    """Observe persistence, including CRLF and UTF-8 bytes; never normalize EOLs."""
    app.send(CTRL_S)
    try:
        eventually(lambda: app.read() and source.read_bytes() == expected)
    except AssertionError as error:
        raise AssertionError(f"Expected {expected!r}, saved {source.read_bytes()!r}") from error


def undo(app, source, expected):
    app.send(CTRL_Z)
    save(app, source, expected)


def redo(app, source, expected):
    app.send(REDO)
    save(app, source, expected)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-smart-typing-pty-") as directory:
        root = Path(directory)

        with editor(root, "generated-close", b"\r\n") as (app, source):
            app.send("(")
            save(app, source, b"()\r\n")
            wait_screen(app, "Ln 1, Col 2")
            app.send(")")
            wait_screen(app, "Ln 1, Col 3")
            save(app, source, b"()\r\n")
            undo(app, source, b"\r\n")
            redo(app, source, b"()\r\n")
            wait_screen(app, "Ln 1, Col 3")
            app.send("x")
            save(app, source, b"()x\r\n")
            undo(app, source, b"()\r\n")
        print("PASS: generated close skips, one Undo removes the pair, Redo restores the outside cursor")

        with editor(root, "generated-delete", b"\r\n") as (app, source):
            app.send("(")
            save(app, source, b"()\r\n")
            app.send(BACKSPACE)
            save(app, source, b"\r\n")
            undo(app, source, b"()\r\n")
            wait_screen(app, "Ln 1, Col 2")
            redo(app, source, b"\r\n")
            undo(app, source, b"()\r\n")
            app.send(")")
            app.send("x")
            save(app, source, b"()x\r\n")
            undo(app, source, b"()\r\n")
        print("PASS: Backspace removes a generated pair atomically; Undo restores its ownership")

        with editor(root, "manual-pair", b"()\r\n") as (app, source):
            app.send(RIGHT)
            app.send(")")
            save(app, source, b"())\r\n")
            undo(app, source, b"()\r\n")
            app.send(BACKSPACE)
            save(app, source, b")\r\n")
            undo(app, source, b"()\r\n")
            redo(app, source, b")\r\n")
        print("PASS: manually loaded pairs neither skip a closer nor delete both characters")

        with editor(root, "physical-enter", b"\r\n") as (app, source):
            app.send("{")
            save(app, source, b"{}\r\n")
            app.send(b"\r")  # Physical Enter, not a programmatic editing command.
            expanded = b"{\r\n    \r\n}\r\n"
            save(app, source, expanded)
            wait_screen(app, "Ln 2, Col 5")
            app.send("x")
            save(app, source, b"{\r\n    x\r\n}\r\n")
            undo(app, source, expanded)
            undo(app, source, b"{}\r\n")
            redo(app, source, expanded)
            wait_screen(app, "Ln 2, Col 5")
            app.send("x")
            save(app, source, b"{\r\n    x\r\n}\r\n")
            undo(app, source, expanded)
        print("PASS: physical Enter expands C++ brackets with CRLF indentation and exact Undo/Redo")

        original = "猫🙂 value\r\n".encode()
        surrounded = "猫🙂 (value)\r\n".encode()
        with editor(root, "unicode-reversed-surround", original) as (app, source):
            app.send(END)
            app.send(SELECT_LEFT * 5)
            app.send("(")
            save(app, source, surrounded)
            undo(app, source, original)
            redo(app, source, surrounded)
            # The retained selection covers only the content, not its delimiters.
            app.send("X")
            save(app, source, "猫🙂 (X)\r\n".encode())
            undo(app, source, surrounded)
        print("PASS: reversed Unicode selection surrounds and restores exact bytes through Undo/Redo")

        disabled = {
            "editor.autoClosingBrackets": "always",
            "[cpp]": {
                "editor.autoClosingBrackets": "never",
                "editor.autoClosingQuotes": "never",
                "editor.autoClosingDelete": "never",
                "editor.autoClosingOvertype": "never",
                "editor.autoSurround": "never",
                "editor.autoIndent": "none",
            },
        }
        with editor(root, "disabled-pair-and-indent", b"    \r\n", disabled) as (app, source):
            app.send(END)
            app.send("(")
            save(app, source, b"    (\r\n")
            undo(app, source, b"    \r\n")
            app.send('"')
            save(app, source, b'    "\r\n')
            undo(app, source, b"    \r\n")
            app.send(b"\r")
            save(app, source, b"    \r\n\r\n")
            undo(app, source, b"    \r\n")
            redo(app, source, b"    \r\n\r\n")
        with editor(root, "disabled-surround", original, disabled) as (app, source):
            app.send(END)
            app.send(SELECT_LEFT * 5)
            app.send("(")
            save(app, source, "猫🙂 (\r\n".encode())
            undo(app, source, original)
            redo(app, source, "猫🙂 (\r\n".encode())
        print("PASS: language-scoped disabled pairing, quotes, surrounding and indentation remain literal")

        with editor(root, "settings-reload-retirement", b"\r\n", {}) as (app, source):
            settings_file = source.parent / "config" / "settings.json"
            app.send("(")
            save(app, source, b"()\r\n")
            settings_file.write_text(json.dumps(disabled), encoding="utf-8")
            wait_screen(app, "Settings reloaded")
            app.send(BACKSPACE)
            save(app, source, b")\r\n")
            undo(app, source, b"()\r\n")
            wait_screen(app, "Ln 1, Col 2")
            settings_file.write_text("{}", encoding="utf-8")
            wait_screen(app, "Settings reloaded")
            # Returning to the original options must not restore retired ownership.
            app.send(")")
            save(app, source, b"())\r\n")
            undo(app, source, b"()\r\n")
        print("PASS: disabling then restoring settings cannot revive generated-pair ownership through Undo")


if __name__ == "__main__":
    run()
