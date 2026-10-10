#!/usr/bin/env python3
"""Native language indentation through physical input and exact persisted bytes."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import Editor, wait_screen
from smart_typing_pty import save, undo, redo


DOWN = b"\x1b[B"
END = b"\x1b[F"
LEFT = b"\x1b[D"


@contextmanager
def editor(root, name, extension, original, mode="full", insert_spaces=True):
    case = root / name
    case.mkdir()
    source = case / f"main.{extension}"
    source.write_bytes(original)
    config = case / "config"
    config.mkdir()
    (config / "settings.json").write_text(json.dumps({
        "editor.autoIndent": mode,
        "editor.tabSize": 4,
        "editor.insertSpaces": insert_spaces,
    }))
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
            app.process.kill()
            app.process.wait(timeout=3)
            app.close_fds()


def history(app, source, original, expected):
    assert source.read_bytes() == original, "Typing must remain unsaved"
    save(app, source, expected)
    undo(app, source, original)
    redo(app, source, expected)
    undo(app, source, original)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-advanced-indent-pty-") as directory:
        root = Path(directory)
        for mode, indentation in (("brackets", b"    "), ("advanced", b""), ("full", b"")):
            original = b"if (ok)\r\n    run();\r\n// " + "猫🙂\r\n".encode()
            with editor(root, f"cpp-body-{mode}", "cpp", original, mode) as (app, source):
                app.send(DOWN + END + b"\r")
                expected = b"if (ok)\r\n    run();\r\n" + indentation + b"\r\n// " + "猫🙂\r\n".encode()
                wait_screen(app, f"Ln 3, Col {len(indentation) + 1}")
                history(app, source, original, expected)
        print("PASS: physical Enter distinguishes bracket and advanced/full C++ body rules with CRLF/Unicode save and Undo")

        original = b"if (ok)\r\n    // note {\r\n"
        with editor(root, "cpp-comment-body", "cpp", original) as (app, source):
            app.send(DOWN + END + b"\r")
            wait_screen(app, "Ln 3, Col 1")
            history(app, source, original, b"if (ok)\r\n    // note {\r\n\r\n")
        print("PASS: native C++ Enter rule handles a comment body without treating its brace as code")

        for mode in ("none", "full"):
            original = b"void f() {\r\n        \r\n}\r\n"
            with editor(root, f"cpp-electric-{mode}", "cpp", original, mode) as (app, source):
                app.send(DOWN + END + b"}")
                wait_screen(app, "Ln 2, Col 2")
                history(app, source, original, b"void f() {\r\n}\r\n}\r\n")
        print("PASS: fresh native proof aligns a manually typed C++ closing brace in none and full modes without Node or LSP")

        original = b"\tif (ok)\r\n \t  run();\r\n"
        with editor(root, "cpp-visual-tabs", "cpp", original) as (app, source):
            app.send(DOWN + END + b"\r")
            wait_screen(app, "Ln 3, Col 5")
            history(app, source, original, b"\tif (ok)\r\n \t  run();\r\n    \r\n")
        print("PASS: mixed tab/space C++ body indentation follows visual tab stops and configured space output")

        for mode, indentation in (("advanced", b"    "), ("full", b"")):
            original = b'{\r\n    "x": 1}\r\n'
            with editor(root, f"json-tail-{mode}", "json", original, mode) as (app, source):
                app.send(DOWN + END + LEFT + b"\r")
                wait_screen(app, f"Ln 3, Col {len(indentation) + 1}")
                history(app, source, original, b'{\r\n    "x": 1\r\n' + indentation + b'}\r\n')
        print("PASS: physical JSON Enter distinguishes advanced and full indentation before an existing closer")

        original = b"void f() {\r\n        \r\n}\r\n"
        with editor(root, "cpp-owned-close", "cpp", original) as (app, source):
            app.send(DOWN + END + b"{")
            save(app, source, b"void f() {\r\n        {}\r\n}\r\n")
            app.send(b"}")
            wait_screen(app, "Ln 2, Col 11")
            save(app, source, b"void f() {\r\n        {}\r\n}\r\n")
            undo(app, source, original)
        print("PASS: generated closer skips before electric alignment and preserves its one-gesture Undo")


if __name__ == "__main__":
    run()
