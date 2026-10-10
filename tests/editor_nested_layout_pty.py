#!/usr/bin/env python3
"""Native nested editor geometry through original Linux keys and palette commands.

Four isolated reports cover local splits/collapse, shared Unicode/CRLF history,
cell resize/reset, and tiny active-only plus clean session restart. No Node/LSP,
mouse dragging, CSS-pixel resizing or graphical workbench parity is claimed.
The Unix CI workflow runs these journeys against the actual native executable.
"""
import fcntl
from pathlib import Path
import re
import struct
import tempfile
import termios

from editor_group_tabs_pty import (
    CLOSE, FIRST, SECOND, SPLIT, RIGHT, ORIGINAL, active, launched, open_file,
    save, setup, undo, redo,
)
from pty_smoke import eventually, wait_screen

THIRD = b"\x1b[51;5u"  # Original Ctrl+3, enhanced CSI-u.
FOURTH = b"\x1b[52;5u"  # Original Ctrl+4.
CTRL_B = b"\x02"  # Original Toggle Sidebar.
DOWN_SPLIT = "View: Split Editor Down"
WIDTH_UP = "View: Increase Current View Width"
WIDTH_DOWN = "View: Decrease Current View Width"
HEIGHT_UP = "View: Increase Current View Height"
HEIGHT_DOWN = "View: Decrease Current View Height"
RESET = "View: Reset Editor Group Sizes"


def palette(app, command):
    app.send(b"\x1bOP")  # Original F1, once.
    wait_screen(app, "Command Palette")
    app.paste(command)

    def selected():
        app.read()
        lines = app.screen.text().splitlines()
        titles = [index for index, line in enumerate(lines) if "Command Palette" in line]
        if len(titles) != 1:
            return False
        row = titles[0]
        if row + 3 >= len(lines):
            return False
        title = lines[row]
        if "┌" not in title or "┐" not in title:
            return False
        left, right = title.index("┌"), title.rindex("┐")
        actual = lines[row + 3][left + 1:right].strip(" │")
        return actual.split("  ", 1)[0].strip() == command

    try:
        eventually(selected)
    except AssertionError as error:
        raise AssertionError(f"Exact palette result {command}:\n{app.screen.text()}") from error
    app.send(b"\r")
    wait_screen(app, absent=("Command Palette",))


def hide_sidebar(app):
    wait_screen(app, "EXPLORER")
    app.send(CTRL_B)
    wait_screen(app, absent=("EXPLORER",))


def geometry(app, expected, relation=lambda result: True):
    """Exact unique group strip labels give independent observed row/column anchors."""
    observed = {}
    snapshot = ""

    def ready():
        nonlocal observed, snapshot
        app.read()
        snapshot = app.screen.text()
        observed = {}
        for row, line in enumerate(snapshot.splitlines()):
            matches = list(re.finditer(r"([1-4]) ·", line))
            for index, match in enumerate(matches):
                end = matches[index + 1].start() if index + 1 < len(matches) else len(line)
                names = re.findall(r"[abcd]\.txt", line[match.end():end])
                if not names:
                    continue
                number = int(match.group(1))
                assert number not in observed, f"Duplicate strip group{number}:\n{snapshot}"
                observed[number] = {"row": row, "column": match.start(), "names": names}
        return ({number: item["names"] for number, item in observed.items()} == expected
                and relation(observed))

    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Nested strips {expected}:\n{snapshot}") from error
    return observed


def resize_terminal(app, rows, columns):
    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))


def unchanged(paths, except_names=()):
    for name, path in paths.items():
        if name not in except_names:
            assert path.read_bytes() == ORIGINAL, (name, path.read_bytes())


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-nested-layout-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "local-splits-collapse")
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(SPLIT)
            active(app, "a.txt", 1)
            before = geometry(app, {1: ["a.txt"], 2: ["a.txt"]},
                              lambda items: items[1]["row"] == items[2]["row"]
                              and items[1]["column"] < items[2]["column"])
            open_file(app, paths["b"])
            palette(app, DOWN_SPLIT)
            active(app, "b.txt", 1)
            open_file(app, paths["c"])
            expected = {1: ["a.txt"], 2: ["a.txt", "b.txt"], 3: ["b.txt", "c.txt"]}
            nested = geometry(app, expected, lambda items:
                              items[1]["column"] == before[1]["column"]
                              and items[1]["row"] == before[1]["row"]
                              and items[2]["column"] == before[2]["column"]
                              and items[2]["row"] == before[2]["row"]
                              and items[3]["column"] == items[2]["column"]
                              and items[3]["row"] > items[2]["row"])
            app.send(CLOSE)
            active(app, "b.txt", 1)
            geometry(app, {1: ["a.txt"], 2: ["a.txt", "b.txt"], 3: ["b.txt"]})
            app.send(CLOSE)
            active(app, "b.txt", 1)
            after = geometry(app, {1: ["a.txt"], 2: ["a.txt", "b.txt"]}, lambda items:
                             items[1]["column"] == before[1]["column"]
                             and items[2]["column"] == before[2]["column"]
                             and items[2]["row"] == before[2]["row"])
            assert 3 not in after and nested[3]["row"] > after[2]["row"]
            unchanged(paths)
        print("PASS: original local Right then Down preserves left pane; historical membership close precedes exact inner collapse without disk changes")

        case, config, paths = setup(root, "shared-history")
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.send(SPLIT)
            active(app, "a.txt", 3)
            app.send(RIGHT)
            active(app, "a.txt", 4)
            palette(app, DOWN_SPLIT)
            active(app, "a.txt", 4)
            app.send(RIGHT)
            active(app, "a.txt", 5)
            geometry(app, {1: ["a.txt"], 2: ["a.txt"], 3: ["a.txt"]})
            app.paste("X")
            first = "猫🙂 aXlpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], first)
            app.send(FIRST)
            active(app, "a.txt", 3)
            app.paste("Y")
            both = "猫🙂Y aXlpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], both)
            app.send(THIRD)
            active(app, "a.txt", 7)
            # Geometry commands must not create a document Undo boundary/edit.
            before_resize = geometry(app, {1: ["a.txt"], 2: ["a.txt"], 3: ["a.txt"]})
            palette(app, HEIGHT_UP)
            geometry(app, {1: ["a.txt"], 2: ["a.txt"], 3: ["a.txt"]},
                     lambda items: items[3]["row"] == before_resize[3]["row"] - 2)
            active(app, "a.txt", 7)
            undo(app, paths["a"], first)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], first)
            redo(app, paths["a"], both)
            unchanged(paths, ("a",))
        print("PASS: three nested shared Unicode carets and original Save/Undo/Redo preserve exact CRLF bytes through palette ratio resize")

        case, config, paths = setup(root, "cell-resize-reset")
        with launched(case, config, paths) as app:
            hide_sidebar(app)
            app.send(SPLIT)
            palette(app, DOWN_SPLIT)
            expected = {1: ["a.txt"], 2: ["a.txt"], 3: ["a.txt"]}
            base = geometry(app, expected)
            palette(app, WIDTH_UP)
            wide = geometry(app, expected, lambda items:
                            items[1]["column"] == base[1]["column"]
                            and items[2]["column"] == base[2]["column"] - 4
                            and items[3]["column"] == base[3]["column"] - 4)
            palette(app, HEIGHT_UP)
            tall = geometry(app, expected, lambda items:
                            items[3]["row"] == wide[3]["row"] - 2
                            and items[2]["row"] == wide[2]["row"]
                            and items[2]["column"] == wide[2]["column"])
            palette(app, HEIGHT_DOWN)
            geometry(app, expected, lambda items: items == wide)
            palette(app, WIDTH_DOWN)
            geometry(app, expected, lambda items: items == base)
            palette(app, WIDTH_UP)
            geometry(app, expected, lambda items: items == wide)
            palette(app, HEIGHT_UP)
            geometry(app, expected, lambda items: items == tall)
            palette(app, RESET)
            geometry(app, expected, lambda items: items == base)
            active(app, "a.txt", 1)
            unchanged(paths)
        print("PASS: original F1 width/height/reset commands change exactly four columns/two rows and preserve carets/files; no pixel parity claim")

        case, config, paths = setup(root, "tiny-session")
        expected = {number: ["a.txt"] for number in range(1, 5)}
        with launched(case, config, paths, session=True) as app:
            hide_sidebar(app)
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            app.send(SPLIT)
            palette(app, DOWN_SPLIT)
            palette(app, DOWN_SPLIT)
            active(app, "a.txt", 3)
            base = geometry(app, expected)
            app.paste("Q")
            changed = "猫🙂Q alpha\r\nsecond β line\r\n".encode()
            active(app, "a.txt", 4)
            resize_terminal(app, 6, 20)
            geometry(app, {4: ["a.txt"]})  # Only active leaf fits; all topology stays retained.
            # The20-column status is clipped: exact caret qualification resumes
            # after restoring geometry, rather than searching nonexistent text.
            assert paths["a"].read_bytes() == ORIGINAL
            resize_terminal(app, 3, 10)
            # The ten-column fallback wraps its message across distinct rows.
            wait_screen(app, "enlarge", "terminal")
            resize_terminal(app, 32, 110)
            geometry(app, expected, lambda items: items == base)
            active(app, "a.txt", 4)
            save(app, paths["a"], changed)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], changed)
            palette(app, HEIGHT_UP)
            resized = geometry(app, expected, lambda items:
                               items[4]["row"] == base[4]["row"] - 2)
            unchanged(paths, ("a",))
        with launched(case, config, paths, restore=True, session=True) as app:
            hide_sidebar(app)
            active(app, "a.txt", 4)
            geometry(app, expected, lambda items: items == resized)
            app.send(FIRST)
            active(app, "a.txt")
            app.send(FOURTH)
            active(app, "a.txt", 4)
            assert paths["a"].read_bytes() == changed
            unchanged(paths, ("a",))
        print("PASS: tiny active-only/whole-screen fallback retains four shared groups and history; clean restart restores nested ratio and focused view")


if __name__ == "__main__":
    run()
