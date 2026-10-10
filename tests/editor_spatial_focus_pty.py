#!/usr/bin/env python3
"""Original directional focus chords across retained native nested groups.

Three isolated journeys, missing Node/disabled LSP/empty PATH. Logical weighted
adjacency, wrap and MRU are qualified separately from upstream CSS pixel layout.
"""
from pathlib import Path
import tempfile

from editor_group_tabs_pty import (
    FIRST, SECOND, SPLIT, CLOSE, RIGHT, ORIGINAL,
    active, launched, open_file, save, setup, undo, redo,
)
from editor_nested_layout_pty import (
    THIRD, DOWN_SPLIT, palette, hide_sidebar, geometry, resize_terminal, unchanged,
)
from pty_smoke import wait_screen

CTRL_K = b"\x1b[107;5u"
DIRECTIONS = {"left": b"\x1b[1;5D", "right": b"\x1b[1;5C",
              "up": b"\x1b[1;5A", "down": b"\x1b[1;5B"}


def focus(app, direction):
    app.send(CTRL_K)
    wait_screen(app, "(ctrl+k) waiting for second key")
    app.send(DIRECTIONS[direction])


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-spatial-focus-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "mru-and-wrap")
        expected = {1: ["a.txt"], 2: ["a.txt", "c.txt"], 3: ["c.txt"]}
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.send(SPLIT)
            open_file(app, paths["c"])
            palette(app, DOWN_SPLIT)
            active(app, "c.txt", 1)
            app.send(RIGHT * 4)
            active(app, "c.txt", 5)
            app.send(SECOND)
            active(app, "c.txt", 1)
            app.send(RIGHT)
            active(app, "c.txt", 2)
            app.send(FIRST)
            active(app, "a.txt", 3)
            base = geometry(app, expected)
            focus(app, "right")  # Both neighbors intersect; recently active group2.
            active(app, "c.txt", 2)
            geometry(app, expected, lambda seen: seen == base)
            focus(app, "down")
            active(app, "c.txt", 5)
            focus(app, "left")
            active(app, "a.txt", 3)
            focus(app, "right")  # Changed MRU now chooses group3.
            active(app, "c.txt", 5)
            focus(app, "up")
            active(app, "c.txt", 2)
            focus(app, "up")  # Outer-edge wrap, never missing-group creation.
            active(app, "c.txt", 5)
            focus(app, "down")
            active(app, "c.txt", 2)
            focus(app, "left")
            active(app, "a.txt", 3)
            focus(app, "left")  # Left outer edge wraps to most recent right pane.
            active(app, "c.txt", 2)
            palette(app, "View: Focus Editor Group Below")
            active(app, "c.txt", 5)
            palette(app, "View: Focus Left Editor Group")
            active(app, "a.txt", 3)
            palette(app, "View: Focus Right Editor Group")
            active(app, "c.txt", 5)
            palette(app, "View: Focus Editor Group Above")
            active(app, "c.txt", 2)
            geometry(app, expected, lambda seen: seen == base)
            unchanged(paths)
        print("PASS: all original focus chords/palette labels select existing geometric neighbors with MRU ties, outer wrap and independent exact carets")

        case, config, paths = setup(root, "shared-undo-and-close")
        expected = {1: ["a.txt"], 2: ["a.txt"], 3: ["a.txt"]}
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.send(SPLIT)
            palette(app, DOWN_SPLIT)
            app.send(RIGHT * 2)
            active(app, "a.txt", 5)
            app.send(SECOND)
            active(app, "a.txt", 3)
            app.send(RIGHT)
            active(app, "a.txt", 4)
            app.send(FIRST)
            active(app, "a.txt", 3)
            geometry(app, expected)
            focus(app, "right")
            active(app, "a.txt", 4)
            focus(app, "down")
            active(app, "a.txt", 5)
            app.paste("Y")
            changed = "猫🙂 aYlpha\r\nsecond β line\r\n".encode()
            active(app, "a.txt", 6)
            focus(app, "left")
            active(app, "a.txt", 3)
            focus(app, "right")
            active(app, "a.txt", 6)
            focus(app, "up")
            active(app, "a.txt", 4)
            save(app, paths["a"], changed)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], changed)
            app.send(CLOSE)
            geometry(app, {1: ["a.txt"], 2: ["a.txt"]})
            focus(app, "left")
            active(app, "a.txt")
            focus(app, "right")
            active(app, "a.txt")
            geometry(app, {1: ["a.txt"], 2: ["a.txt"]})
            assert paths["a"].read_bytes() == changed
            unchanged(paths, ("a",))
        print("PASS: shared Unicode/CRLF edits and exact Undo/Redo survive directional focus and closing only one group view")

        case, config, paths = setup(root, "tiny-hidden-topology")
        expected = {1: ["a.txt"], 2: ["a.txt", "c.txt"],
                    3: ["c.txt"], 4: ["c.txt"]}
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(RIGHT * 2)
            app.send(SPLIT)
            open_file(app, paths["c"])
            palette(app, DOWN_SPLIT)
            palette(app, DOWN_SPLIT)
            app.send(RIGHT * 4)
            active(app, "c.txt", 5)
            base = geometry(app, expected)
            resize_terminal(app, 6, 20)
            geometry(app, {4: ["c.txt"]})
            # Chord acknowledgement cannot fit the clipped status; issue once
            # after positive tiny publication, then require exact new group strip.
            app.send(CTRL_K + DIRECTIONS["left"])
            geometry(app, {1: ["a.txt"]})
            app.send(CTRL_K + DIRECTIONS["right"])
            geometry(app, {4: ["c.txt"]})
            resize_terminal(app, 3, 10)
            wait_screen(app, "enlarge", "terminal")
            app.send(CTRL_K + DIRECTIONS["left"])
            resize_terminal(app, 32, 110)
            geometry(app, expected, lambda seen: seen == base)
            active(app, "a.txt", 3)
            focus(app, "right")
            active(app, "c.txt", 5)
            geometry(app, expected, lambda seen: seen == base)
            unchanged(paths)
        print("PASS: tiny and whole-screen fallback retain directional keyboard access to hidden topology without model loss or group creation")


if __name__ == "__main__":
    run()
