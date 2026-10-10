#!/usr/bin/env python3
"""Native preview tabs through original Linux keys, with no Node or LSP.

Six reports cover eight isolated terminal sessions. Preview mode is opt-in for
Quick Open; explicit Open remains committed. These journeys do not qualify
Explorer graphical double-clicks, sticky tabs, or persisted preview modes.
"""
import json
from pathlib import Path
import sys
import tempfile

from editor_group_tabs_pty import (
    FIRST, SECOND, SPLIT, RIGHT, ORIGINAL, active, launched, open_file,
    palette, save, setup, strips,
)
from pty_smoke import CTRL_P, CTRL_Z, eventually, wait_screen
from smart_typing_pty import REDO

KEEP_EDITOR = b"\x0b\r"  # Original Linux Ctrl+K, Enter chord.
LEFT = b"\x1b[D"  # Original Left, preserving the copied split view.


def configured(root, name, *, enabled=True, quick=True):
    case, config, paths = setup(root, name)
    path = config / "settings.json"
    values = json.loads(path.read_text(encoding="utf-8"))
    values.update({"workbench.editor.enablePreview": enabled,
                   "workbench.editor.enablePreviewFromQuickOpen": quick})
    path.write_text(json.dumps(values), encoding="utf-8")
    return case, config, paths


def quick_open(app, path):
    """Issue the original command once, then positively wait for its result row."""
    app.send(CTRL_P)
    wait_screen(app, "Go to File")
    app.paste(path.name)

    def matching_result():
        app.read()
        lines = app.screen.text().splitlines()
        titles = [index for index, line in enumerate(lines) if "Go to File" in line]
        if len(titles) != 1:
            return False
        # Restrict readiness to an exact popup result row below the input, not
        # filename text already visible in a tab strip, Explorer or query.
        title = lines[titles[0]]
        left, right = title.index("┌"), title.rindex("┐")
        return any(line[left + 1:right].strip(" │") == path.name
                   for line in lines[titles[0] + 3:titles[0] + 18])

    try:
        eventually(matching_result)
    except AssertionError as error:
        raise AssertionError(f"Quick Open result {path.name}:\n{app.screen.text()}") from error
    app.send(b"\r")
    wait_screen(app, absent=("Go to File",))
    active(app, path.name)


def unchanged(paths, *, except_name=None):
    for name, path in paths.items():
        if name != except_name:
            assert path.read_bytes() == ORIGINAL, (path, path.read_bytes())


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-preview-pty-") as directory:
        root = Path(directory)
        case, config, paths = configured(root, "clean-replacement-keep")
        with launched(case, config, paths) as app:
            quick_open(app, paths["b"])
            strips(app, [["a.txt", "b.txt"]])
            app.send(RIGHT * 2)
            active(app, "b.txt", 3)
            quick_open(app, paths["b"])
            active(app, "b.txt", 3)
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "c.txt"]])
            app.send(KEEP_EDITOR)
            quick_open(app, paths["d"])
            strips(app, [["a.txt", "c.txt", "d.txt"]])
            # Keep Editor is available through its original palette command too.
            palette(app, "View: Keep Editor")
            quick_open(app, paths["b"])
            strips(app, [["a.txt", "c.txt", "d.txt", "b.txt"]])
            unchanged(paths)
        print("PASS: opted-in Quick Open replaces only a clean preview; motion/reopen retain mode, original Ctrl+K Enter and Keep Editor palette commit without editing")

        case, config, paths = configured(root, "edit-undo-permanent")
        with launched(case, config, paths) as app:
            quick_open(app, paths["b"])
            app.send(RIGHT * 2)
            active(app, "b.txt", 3)
            app.paste("X")
            active(app, "b.txt", 4)
            app.send(CTRL_Z)
            active(app, "b.txt", 3)
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            # Undo restored cleanliness but cannot undo permanent tab commitment.
            quick_open(app, paths["b"])
            active(app, "b.txt", 3)
            app.send(REDO)
            active(app, "b.txt", 4)
            changed = "猫🙂X alpha\r\nsecond β line\r\n".encode()
            save(app, paths["b"], changed)
            app.send(CTRL_Z)
            save(app, paths["b"], ORIGINAL)
            app.send(REDO)
            save(app, paths["b"], changed)
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            unchanged(paths, except_name="b")
        print("PASS: Unicode/CRLF edit then Undo permanently commits a preview, replacement preserves its native history and exact Save/Undo/Redo bytes")

        case, config, paths = configured(root, "split-shared")
        with launched(case, config, paths) as app:
            quick_open(app, paths["b"])
            app.send(RIGHT * 2)
            active(app, "b.txt", 3)
            app.send(SPLIT)
            active(app, "b.txt", 3)
            strips(app, [["a.txt", "b.txt"], ["b.txt"]])
            app.send(FIRST)
            active(app, "b.txt", 3)
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "c.txt"], ["b.txt"]])
            app.send(SECOND)
            active(app, "b.txt", 3)
            app.send(LEFT)
            active(app, "b.txt", 2)
            app.paste("Y")
            changed = "猫Y🙂 alpha\r\nsecond β line\r\n".encode()
            save(app, paths["b"], changed)
            app.send(CTRL_Z)
            save(app, paths["b"], ORIGINAL)
            app.send(REDO)
            save(app, paths["b"], changed)
            strips(app, [["a.txt", "c.txt"], ["b.txt"]])
            unchanged(paths, except_name="b")
        print("PASS: original split/focus keys preserve independent carets and a committed destination while replacing the source preview retains shared Unicode/CRLF Undo and exact disk bytes")

        case, config, paths = configured(root, "dirty-shared-target")
        with launched(case, config, paths) as app:
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.paste("X")
            active(app, "a.txt", 4)
            open_file(app, paths["b"])
            app.send(SPLIT)
            active(app, "b.txt", 1)
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "b.txt"], ["b.txt", "c.txt"]])
            quick_open(app, paths["a"])
            active(app, "a.txt", 1)  # New per-group membership owns its own caret.
            strips(app, [["a.txt", "b.txt"], ["b.txt", "c.txt", "a.txt"]])
            changed = "猫🙂X alpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], changed)
            app.send(FIRST)
            open_file(app, paths["a"])
            active(app, "a.txt", 4)
            app.send(CTRL_Z)
            save(app, paths["a"], ORIGINAL)
            app.send(REDO)
            save(app, paths["a"], changed)
            app.send(SECOND)
            quick_open(app, paths["d"])
            # The original C preview, not the newly admitted dirty A, is replaced.
            strips(app, [["a.txt", "b.txt"], ["b.txt", "a.txt", "d.txt"]])
            unchanged(paths, except_name="a")
        print("PASS: admitting a dirty shared authority commits it before replacement, retains the existing preview, original group caret and shared Undo/Redo without touching other files")

        for enabled, quick, name in [(True, False, "default-quick-open"),
                                     (False, True, "preview-disabled")]:
            case, config, paths = configured(root, name, enabled=enabled, quick=quick)
            with launched(case, config, paths) as app:
                quick_open(app, paths["b"])
                quick_open(app, paths["c"])
                strips(app, [["a.txt", "b.txt", "c.txt"]])
                open_file(app, paths["d"])
                strips(app, [["a.txt", "b.txt", "c.txt", "d.txt"]])
                unchanged(paths)
        print("PASS: default Quick Open and disabled preview remain committed controls; original explicit Ctrl+O keeps all tabs and exact backing bytes")

        case, config, paths = configured(root, "restart-committed")
        with launched(case, config, paths, session=True) as app:
            quick_open(app, paths["b"])
            strips(app, [["a.txt", "b.txt"]])
            unchanged(paths)
        with launched(case, config, paths, restore=True, session=True) as app:
            active(app, "b.txt", 1)
            strips(app, [["a.txt", "b.txt"]])
            quick_open(app, paths["c"])
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            quick_open(app, paths["d"])
            strips(app, [["a.txt", "b.txt", "d.txt"]])
            unchanged(paths)
        print("PASS: clean session restoration deliberately commits historical preview memberships, preserves ordering and bytes, and allows a fresh transient preview; terminal modes restore")


if __name__ == "__main__":
    run()
