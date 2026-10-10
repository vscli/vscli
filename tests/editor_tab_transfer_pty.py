#!/usr/bin/env python3
"""Four native transfer reports across six real terminal sessions.

Original Linux keys, no Node/LSP, empty PATH; source-only candidate until run.
Default transfer policy plus right/down creation; directional move/copy deferred.
"""
import json
from pathlib import Path
import tempfile

from editor_group_tabs_pty import (
    FIRST, SPLIT, RIGHT, ORIGINAL, active, launched, open_file, save, setup,
    strips, undo, redo,
)
from editor_nested_layout_pty import geometry
from editor_tab_reordering_pty import quick_open
from sticky_editor_tabs_pty import chord, palette, state, tab, unchanged
from pty_smoke import CTRL_Z, wait_screen

MOVE_PREVIOUS = b"\x1b[1;7D"  # Original Ctrl+Alt+Left.
MOVE_NEXT = b"\x1b[1;7C"      # Original Ctrl+Alt+Right.
MOVE_FIRST = b"\x1b[49;4u"    # Original Shift+Alt+1.
MOVE_LAST = b"\x1b[57;4u"     # Original Shift+Alt+9.


def configured(root, name, values):
    case, config, paths = setup(root, name)
    path = config / "settings.json"
    original = json.loads(path.read_text(encoding="utf-8"))
    original.update(values)
    path.write_text(json.dumps(original), encoding="utf-8")
    return case, config, paths


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-transfer-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "original-key-history-restart")
        with launched(case, config, paths, session=True) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            app.send(RIGHT * 2)
            active(app, "c.txt", 3)
            app.send(MOVE_NEXT)
            strips(app, [["a.txt", "b.txt"], ["c.txt"]])
            active(app, "c.txt", 3)
            app.paste("Xλ")
            active(app, "c.txt", 5)
            app.send(CTRL_Z)
            active(app, "c.txt", 3)
            app.send(MOVE_PREVIOUS)
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            active(app, "c.txt", 3)
            edited = "猫🙂Xλ alpha\r\nsecond β line\r\n".encode()
            redo(app, paths["c"], edited)
            undo(app, paths["c"], ORIGINAL)
            app.send(MOVE_NEXT)
            strips(app, [["a.txt", "b.txt"], ["c.txt"]])
            active(app, "c.txt", 3)
            palette(app, "View: Move Editor into First Group")
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            active(app, "c.txt", 3)
            palette(app, "View: Move Editor into Next Group")
            strips(app, [["a.txt", "b.txt"], ["c.txt"]])
            active(app, "c.txt", 3)
            unchanged(paths)
        with launched(case, config, paths, restore=True, session=True) as app:
            strips(app, [["a.txt", "b.txt"], ["c.txt"]])
            active(app, "c.txt", 3)
            app.send(MOVE_FIRST)
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            active(app, "c.txt", 3)
            app.send(MOVE_PREVIOUS)  # First group edge: no wrap or creation.
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            active(app, "c.txt", 3)
            unchanged(paths)
        print("PASS: original transfer keys/palette carry Unicode/CRLF caret and pending Redo, collapse source and restore clean session")

        case, config, paths = configured(root, "sticky-preview-dedup", {
            "workbench.editor.enablePreview": True,
            "workbench.editor.enablePreviewFromQuickOpen": True,
        })
        with launched(case, config, paths) as app:
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            chord(app)
            state(app, [[tab("a", sticky=True)]])
            app.send(SPLIT)
            state(app, [[tab("a", sticky=True)], [tab("a")]])
            quick_open(app, paths["c"])
            strips(app, [["a.txt"], ["a.txt", "c.txt"]])
            active(app, "c.txt", 1)
            app.send(FIRST)
            active(app, "a.txt", 3)
            app.send(MOVE_NEXT)  # Reuses target A, carries sticky, collapses source.
            state(app, [[tab("a", sticky=True), tab("c")]])
            active(app, "a.txt", 3)
            quick_open(app, paths["d"])
            state(app, [[tab("a", sticky=True), tab("d")]])  # C remained preview.
            app.send(MOVE_NEXT)  # Preview D becomes a committed moved editor.
            state(app, [[tab("a", sticky=True)], [tab("d")]])
            active(app, "d.txt", 1)
            quick_open(app, paths["b"])
            state(app, [[tab("a", sticky=True)], [tab("d"), tab("b")]])
            unchanged(paths)
        print("PASS: deduplicating transfer preserves sticky target identity and unrelated preview; moved preview becomes committed")

        case, config, paths = configured(root, "down-creation-capacity", {
            "workbench.editor.openSideBySideDirection": "down",
        })
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            app.send(MOVE_NEXT)
            geometry(app, {1: ["a.txt"], 2: ["b.txt"]}, lambda positions:
                     positions[1]["column"] == positions[2]["column"] and
                     positions[1]["row"] < positions[2]["row"])
            open_file(app, paths["c"])
            app.send(MOVE_NEXT)
            strips(app, [["a.txt"], ["b.txt"], ["c.txt"]])
            open_file(app, paths["d"])
            app.send(MOVE_NEXT)
            strips(app, [["a.txt"], ["b.txt"], ["c.txt"], ["d.txt"]])
            active(app, "d.txt", 1)
            app.send(MOVE_NEXT)
            wait_screen(app, "Editor group limit reached (4)")
            strips(app, [["a.txt"], ["b.txt"], ["c.txt"], ["d.txt"]])
            active(app, "d.txt", 1)
            app.send(MOVE_FIRST)
            strips(app, [["a.txt", "d.txt"], ["b.txt"], ["c.txt"]])
            active(app, "d.txt", 1)
            app.send(MOVE_LAST)
            strips(app, [["a.txt"], ["b.txt"], ["c.txt", "d.txt"]])
            active(app, "d.txt", 1)
            unchanged(paths)
        print("PASS: right/down setting honors down topology, four-group refusal is inert, original first/last moves collapse source")

        for index, values in enumerate([
            {"workbench.editor.openPositioning": "left"},
            {"workbench.editor.closeEmptyGroups": False},
        ]):
            case, config, paths = configured(root, f"unsupported-policy-{index}", values)
            with launched(case, config, paths) as app:
                open_file(app, paths["b"])
                app.paste("dirty λ ")
                active(app, "b.txt", 9)
                app.send(MOVE_NEXT)
                wait_screen(app, "Group transfer rejected")
                strips(app, [["a.txt", "b.txt"]])
                active(app, "b.txt", 9)
                expected = "dirty λ ".encode() + ORIGINAL
                save(app, paths["b"], expected)
                undo(app, paths["b"], ORIGINAL)
                redo(app, paths["b"], expected)
                unchanged(paths, except_names=("b",))
        print("PASS: unsupported nonright insertion/empty-group policy refuses before mutation, preserving dirty Undo/Redo and exact saved bytes")


if __name__ == "__main__":
    run()
