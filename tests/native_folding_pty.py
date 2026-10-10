#!/usr/bin/env python3
"""Native visible folding with original commands and an empty executable PATH.

Six isolated terminal journeys use positive screen/disk receipt checks. This
source is prepared for parent execution; no provider, wrapping or upstream GUI
parity is asserted. Worker cancellation/source ABA have gated Rust App tests.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from editor_group_tabs_pty import FIRST, SECOND, SPLIT
from smart_typing_pty import REDO

SOURCE = "prefix\r\nhead猫\r\n body🙂\r\n tail\r\nafter\r\n".encode()
FOLD_ALL = b"\x0b\x1b[48;5u"  # Original Linux Ctrl+K Ctrl+0.
UNFOLD_ALL = b"\x0b\x1b[106;5u"  # Original Linux Ctrl+K Ctrl+J.
FOLD = b"\x1b[91;6u"  # Original Linux Ctrl+Shift+[.
UNFOLD = b"\x1b[93;6u"  # Original Linux Ctrl+Shift+].
DOWN = b"\x1b[B"
UP = b"\x1b[A"


@contextmanager
def launched(root, name, original=SOURCE, enabled=True):
    case = root / name
    case.mkdir()
    config = case / "config"
    config.mkdir()
    values = {"editor.folding": enabled, "editor.foldingStrategy": "indentation",
              "vscli.languageServer.enabled": False, "breadcrumbs.enabled": False,
              "editor.quickSuggestions": False, "editor.parameterHints.enabled": False,
              "files.autoSave": "off"}
    (config / "settings.json").write_text(json.dumps(values), encoding="utf-8")
    source = case / "fold.txt"
    source.write_bytes(original)
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 "--no-session", source, enhanced=True, extra_env={"PATH": ""})
    finished = False
    try:
        wait_screen(app, "prefix", "Ln 1, Col 1")
        yield app, source
        app.finish()
        finished = True
    except BaseException:
        print(f"Native folding PTY failure ({name}):\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not finished:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                app.close_fds()
            except Exception as error:
                print(f"PTY cleanup failed: {error}", file=sys.stderr)


def palette(app, command):
    """Prove the exact first selected palette result before invoking once."""
    app.send(b"\x1bOP")
    wait_screen(app, "Command Palette")
    app.paste(command)

    def selected():
        app.read()
        lines = app.screen.text().splitlines()
        titles = [i for i, line in enumerate(lines) if "Command Palette" in line]
        if len(titles) != 1:
            return False
        row = titles[0]
        title = lines[row]
        if row + 3 >= len(lines) or "┌" not in title or "┐" not in title:
            return False
        left, right = title.index("┌"), title.rindex("┐")
        actual = lines[row + 3][left + 1:right].strip(" │")
        return actual.split("  ", 1)[0].strip() == command

    eventually(selected)
    app.send(b"\r")
    wait_screen(app, absent=("Command Palette",))


def folded(app):
    wait_screen(app, "Native folds ready", "head猫", "after", absent=("body🙂", " tail"), timeout=8)


def save(app, path, expected):
    app.send(CTRL_S)
    eventually(lambda: app.read() and path.read_bytes() == expected, timeout=8)
    wait_screen(app, "Saved", timeout=8)
    assert path.read_bytes() == expected


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-native-folding-pty-") as directory:
        root = Path(directory)
        with launched(root, "original-keys-visible-rows") as (app, path):
            wait_screen(app, "body🙂", " tail")
            app.send(FOLD_ALL)
            folded(app)
            app.send(DOWN * 2)
            wait_screen(app, "Ln 5, Col 1")
            app.send(UP)
            wait_screen(app, "Ln 2, Col 1")
            app.send(UNFOLD)
            wait_screen(app, "Native folds ready", "body🙂", " tail", timeout=8)
            app.send(FOLD)
            folded(app)
            app.send(UNFOLD_ALL)
            wait_screen(app, "Unfolded all", "body🙂", " tail")
            assert path.read_bytes() == SOURCE
        print("PASS: original fold/unfold chords, visible-row Up/Down and exact untouched Unicode/CRLF bytes")

        with launched(root, "palette-four-commands") as (app, path):
            palette(app, "Fold All")
            folded(app)
            palette(app, "Unfold All")
            wait_screen(app, "Unfolded all", "body🙂")
            app.send(DOWN)
            wait_screen(app, "Ln 2, Col 1")
            palette(app, "Fold")
            folded(app)
            palette(app, "Unfold")
            wait_screen(app, "Native folds ready", "body🙂", timeout=8)
            assert path.read_bytes() == SOURCE
        print("PASS: all four exact original palette commands perform native folding without Node or LSP")

        with launched(root, "literal-edit-save-history") as (app, path):
            app.send(FOLD_ALL)
            folded(app)
            app.paste("é🙂")
            edited = "é🙂".encode() + SOURCE
            save(app, path, edited)
            app.send(CTRL_Z)
            save(app, path, SOURCE)
            app.send(REDO)
            save(app, path, edited)
            palette(app, "Unfold All")
            wait_screen(app, "Unfolded all", "é🙂prefix", "body🙂")
            assert path.read_bytes() == edited
        print("PASS: folded editing and asynchronous Save preserve UTF-8/CRLF bytes plus the original text Undo/Redo")

        with launched(root, "split-view-clear") as (app, path):
            app.send(FOLD_ALL)
            folded(app)
            app.send(SPLIT)
            wait_screen(app, "2 ·")
            palette(app, "Unfold All")
            wait_screen(app, "Unfolded all", "body🙂")
            app.send(FIRST)
            app.send(DOWN * 2)
            wait_screen(app, "Ln 5, Col 1")
            app.send(SECOND)
            app.send(DOWN * 2)
            wait_screen(app, "Ln 3, Col 1")
            assert path.read_bytes() == SOURCE
        print("PASS: explicit split copies fold intent with independent clear and per-view movement on one shared model")

        with launched(root, "configured-disabled", enabled=False) as (app, path):
            palette(app, "Fold All")
            wait_screen(app, "Folding is disabled", "body🙂", " tail")
            app.paste("é")
            save(app, path, "é".encode() + SOURCE)
            app.send(CTRL_Z)
            save(app, path, SOURCE)
            palette(app, "Unfold All")
            wait_screen(app, "Unfolded all", "body🙂")
        print("PASS: disabled folding refuses collapse while native edits, Save and scan-free Unfold All remain usable")

        large = b"prefix\r\nhead\r\n " + b"x" * (2 * 1024 * 1024) + b"\r\nafter\r\n"
        with launched(root, "over-budget-expanded", original=large) as (app, path):
            palette(app, "Fold All")
            wait_screen(app, "Folding unavailable", "exceeds", timeout=8)
            app.paste("é")
            edited = "é".encode() + large
            save(app, path, edited)
            app.send(CTRL_Z)
            save(app, path, large)
            palette(app, "Unfold All")
            wait_screen(app, "Unfolded all", "prefix")
        print("PASS: folding budget refusal preserves expanded native editing, exact large-file persistence and terminal restoration")


if __name__ == "__main__":
    run()
