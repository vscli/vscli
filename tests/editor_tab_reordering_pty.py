#!/usr/bin/env python3
"""Four native same-group reorder reports across five real terminal sessions.

Original Linux keys; no Node/LSP, empty PATH, exact Unicode/CRLF and saved bytes.
Source-only candidate: execution and platform qualification belong to CI/root.
"""
import json
from pathlib import Path
import tempfile

from editor_group_tabs_pty import (
    CLOSE, FIRST, SECOND, SPLIT, RIGHT, PREVIOUS, NEXT, ORIGINAL,
    active, launched, open_file, save, setup, strips, undo, redo,
)
from sticky_editor_tabs_pty import chord, palette, state, tab, unchanged
from pty_smoke import CTRL_P, CTRL_Z, eventually, wait_screen

MOVE_LEFT = b"\x1b[5;6~"   # Original Ctrl+Shift+PageUp, distinct from Ctrl+PageUp.
MOVE_RIGHT = b"\x1b[6;6~"  # Original Ctrl+Shift+PageDown.
LEFT_COMMAND = "View: Move Editor Left"
RIGHT_COMMAND = "View: Move Editor Right"


def quick_open(app, path):
    app.send(CTRL_P)
    wait_screen(app, "Go to File")
    app.paste(path.name)

    def exact_selected_result():
        app.read()
        lines = app.screen.text().splitlines()
        titles = [index for index, line in enumerate(lines) if "Go to File" in line]
        if len(titles) != 1 or titles[0] + 3 >= len(lines):
            return False
        row = titles[0]
        title = lines[row]
        if "┌" not in title or "┐" not in title:
            return False
        left, right = title.index("┌"), title.rindex("┐")
        return lines[row + 3][left + 1:right].strip(" │") == path.name

    eventually(exact_selected_result)
    app.send(b"\r")
    wait_screen(app, absent=("Go to File",))
    active(app, path.name)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-reorder-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "order-undo-restart")
        with launched(case, config, paths, session=True) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            active(app, "c.txt", 1)
            app.send(MOVE_RIGHT)  # Edge no-op.
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            app.send(MOVE_LEFT)
            strips(app, [["a.txt", "c.txt", "b.txt"]])
            active(app, "c.txt", 1)
            palette(app, LEFT_COMMAND)
            strips(app, [["c.txt", "a.txt", "b.txt"]])
            active(app, "c.txt", 1)
            palette(app, RIGHT_COMMAND)
            strips(app, [["a.txt", "c.txt", "b.txt"]])
            app.paste("Xλ")
            active(app, "c.txt", 3)
            app.send(CTRL_Z)
            active(app, "c.txt", 1)
            app.send(MOVE_RIGHT)  # A visual move must preserve pending Redo.
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            edited = "Xλ".encode() + ORIGINAL
            redo(app, paths["c"], edited)
            undo(app, paths["c"], ORIGINAL)
            app.send(MOVE_LEFT)
            strips(app, [["a.txt", "c.txt", "b.txt"]])
            active(app, "c.txt", 1)
            unchanged(paths)
        with launched(case, config, paths, restore=True, session=True) as app:
            strips(app, [["a.txt", "c.txt", "b.txt"]])
            active(app, "c.txt", 1)
            app.send(MOVE_LEFT)
            strips(app, [["c.txt", "a.txt", "b.txt"]])
            active(app, "c.txt", 1)
            unchanged(paths)
        print("PASS: original move keys/palette preserve caret, exact Unicode/CRLF Undo/Redo and clean-session visual order")

        case, config, paths = setup(root, "sticky-crossing")
        with launched(case, config, paths) as app:
            chord(app)
            state(app, [[tab("a", sticky=True)]])
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            app.send(MOVE_LEFT)
            state(app, [[tab("a", sticky=True), tab("c"), tab("b")]])
            app.send(MOVE_LEFT)
            state(app, [[tab("c", sticky=True), tab("a", sticky=True), tab("b")]])
            active(app, "c.txt", 1)
            app.send(CLOSE)  # Protected close focuses the nonsticky MRU.
            active(app, "b.txt", 1)
            state(app, [[tab("c", sticky=True), tab("a", sticky=True), tab("b")]])
            app.send(NEXT)
            active(app, "c.txt", 1)
            app.send(MOVE_RIGHT)
            state(app, [[tab("a", sticky=True), tab("c", sticky=True), tab("b")]])
            app.send(MOVE_RIGHT)
            state(app, [[tab("a", sticky=True), tab("b"), tab("c")]])
            active(app, "c.txt", 1)
            app.send(CLOSE)
            active(app, "b.txt", 1)
            state(app, [[tab("a", sticky=True), tab("b")]])
            unchanged(paths)
        print("PASS: crossing the sticky prefix pins/unpins exactly the moved tab and ordinary close respects current protection")

        case, config, paths = setup(root, "preview-promotion")
        path = config / "settings.json"
        settings = json.loads(path.read_text(encoding="utf-8"))
        settings.update({"workbench.editor.enablePreview": True,
                         "workbench.editor.enablePreviewFromQuickOpen": True})
        path.write_text(json.dumps(settings), encoding="utf-8")
        with launched(case, config, paths) as app:
            quick_open(app, paths["b"])
            strips(app, [["a.txt", "b.txt"]])
            app.send(MOVE_RIGHT)  # Last preview remains replaceable after no-op.
            strips(app, [["a.txt", "b.txt"]])
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "c.txt"]])
            app.send(MOVE_LEFT)
            strips(app, [["c.txt", "a.txt"]])
            active(app, "c.txt", 1)
            quick_open(app, paths["d"])
            strips(app, [["c.txt", "d.txt", "a.txt"]])
            quick_open(app, paths["b"])
            strips(app, [["c.txt", "b.txt", "a.txt"]])
            unchanged(paths)
        print("PASS: edge no-op preserves replaceable preview; actual reorder commits it before later Quick Open replacement")

        case, config, paths = setup(root, "shared-group-views")
        with launched(case, config, paths) as app:
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            open_file(app, paths["b"])
            app.send(SPLIT)
            active(app, "b.txt", 1)  # Split copies the source view.
            open_file(app, paths["a"])
            app.send(RIGHT)
            active(app, "a.txt", 2)
            open_file(app, paths["c"])
            app.send(PREVIOUS)
            active(app, "a.txt", 2)
            app.send(MOVE_LEFT)
            strips(app, [["a.txt", "b.txt"], ["a.txt", "b.txt", "c.txt"]])
            active(app, "a.txt", 2)
            app.paste("Y")
            edited = "猫Y🙂 alpha\r\nsecond β line\r\n".encode()
            active(app, "a.txt", 3)
            app.send(FIRST)
            active(app, "b.txt", 1)
            app.send(PREVIOUS)
            active(app, "a.txt", 4)  # Shared insertion maps the old scalar caret.
            strips(app, [["a.txt", "b.txt"], ["a.txt", "b.txt", "c.txt"]])
            save(app, paths["a"], edited)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], edited)
            app.send(SECOND)
            active(app, "a.txt")
            strips(app, [["a.txt", "b.txt"], ["a.txt", "b.txt", "c.txt"]])
            unchanged(paths, except_names=("a",))
            assert paths["a"].read_bytes() == edited
        print("PASS: only the focused group reorders; shared Unicode/CRLF edits, historical carets and exact Undo/Redo remain intact")


if __name__ == "__main__":
    run()
