#!/usr/bin/env python3
"""Native Close Group merge through original F1 and native-only real PTYs.

Three native-only journeys preserve model/history/file bytes and terminal mode.
The original closeGroup ID is exposed with its toolbar title in the native
palette; upstream does not list this ID in its pinned command palette.
"""
import json
from pathlib import Path
import tempfile

from editor_group_tabs_pty import (
    FIRST, SECOND, SPLIT, RIGHT, NEXT, PREVIOUS, ORIGINAL,
    active, launched, open_file, save, setup, strips, undo, redo,
)
from sticky_editor_tabs_pty import palette
from pty_smoke import wait_screen

MERGE = "View: Close Group"
CLOSE_EDITORS = "View: Close All Editors in Group"


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-close-group-merge-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "many-inactive-views")
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            app.send(PREVIOUS * 2)
            active(app, "a.txt", 1)
            app.send(SPLIT)
            active(app, "a.txt", 1)
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.send(FIRST)
            active(app, "a.txt", 1)
            app.send(NEXT)
            active(app, "b.txt", 1)
            app.paste("λ")
            active(app, "b.txt", 2)
            changed = "λ".encode() + ORIGINAL
            assert paths["b"].read_bytes() == ORIGINAL
            palette(app, MERGE)
            active(app, "b.txt", 2)
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            app.send(PREVIOUS)
            active(app, "a.txt", 3)  # Existing inactive target geometry survives.
            app.send(NEXT)
            active(app, "b.txt", 2)
            save(app, paths["b"], changed)
            undo(app, paths["b"], ORIGINAL)
            redo(app, paths["b"], changed)
            app.send(NEXT)
            active(app, "c.txt", 1)  # New inactive target is native Origin.
            assert paths["a"].read_bytes() == paths["c"].read_bytes() == paths["d"].read_bytes() == ORIGINAL
        print("PASS: Close Group keeps inactive target caret, copies active Unicode/CRLF dirty view, retains other models and exact Save/Undo/Redo bytes")

        case, config, paths = setup(root, "close-empty-false-and-inert-root")
        settings = config / "settings.json"
        values = json.loads(settings.read_text(encoding="utf-8"))
        values["workbench.editor.closeEmptyGroups"] = False
        settings.write_text(json.dumps(values), encoding="utf-8")
        with launched(case, config, paths) as app:
            app.send(RIGHT)
            active(app, "a.txt", 2)
            app.send(SPLIT)
            active(app, "a.txt", 2)
            app.paste("🙂")
            active(app, "a.txt", 3)
            changed = "猫🙂🙂 alpha\r\nsecond β line\r\n".encode()
            palette(app, MERGE)
            active(app, "a.txt", 3)
            strips(app, [["a.txt"]])
            wait_screen(app, absent=("Unsaved changes",))
            save(app, paths["a"], changed)
            palette(app, MERGE)  # Sole native root: owned no-op, never a document close.
            active(app, "a.txt", 3)
            strips(app, [["a.txt"]])
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], changed)
            assert all(paths[name].read_bytes() == ORIGINAL for name in "bcd")
        print("PASS: explicit merge removes source with closeEmptyGroups=false, preserves dirty shared work, and sole-group command is inert")

        case, config, paths = setup(root, "distinct-close-palette")
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            app.send(SPLIT)
            active(app, "b.txt", 1)
            palette(app, CLOSE_EDITORS)
            active(app, "b.txt", 1)
            strips(app, [["a.txt", "b.txt"]])
            app.send(SPLIT)
            open_file(app, paths["c"])
            palette(app, MERGE)
            active(app, "c.txt", 1)
            strips(app, [["a.txt", "b.txt", "c.txt"]])
            assert all(path.read_bytes() == ORIGINAL for path in paths.values())
        print("PASS: original Close All Editors in Group closes only source memberships while native Close Group retains and merges them; terminal restoration is checked")


if __name__ == "__main__":
    run()
