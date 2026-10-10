#!/usr/bin/env python3
"""Original native sticky-tab commands through real Linux-profile terminal input.

Five reports cover nine isolated sessions with no Node, no LSP and empty PATH.
These journeys qualify keyboard close policy, not graphical mouse-close gestures.
Local debug journeys passed; optimized and platform qualification are separate.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import re
import tempfile

from editor_group_tabs_pty import (
    CLOSE, ESCAPE, FIRST, SECOND, SPLIT, RIGHT, ORIGINAL,
    active, launched as group_launched, open_file, save, setup, undo, redo,
)
from pty_smoke import CTRL_Z, eventually, wait_screen
from smart_typing_pty import REDO

CTRL_K = b"\x1b[107;5u"  # Original Ctrl+K, enhanced CSI-u delivery.
SHIFT_ENTER = b"\x1b[13;2u"  # Original Shift+Enter, distinct from Keep Editor.
LEFT = b"\x1b[D"
PIN = "View: Pin Editor"
UNPIN = "View: Unpin Editor"
FORCED_CLOSE = "View: Close Pinned Editor"
CLOSE_GROUP = "View: Close Editor Group"  # workbench.action.closeEditorsInGroup.
CLOSE_ALL = "File: Close All Editors"


@contextmanager
def launched(case, config, paths):
    with group_launched(case, config, paths) as app:
        wait_screen(app, "enhanced keys", absent=("Command Palette", "Open File (absolute"))
        yield app


def palette(app, command):
    """Wait for the exact first result before accepting once; never retry a key."""
    app.send(b"\x1bOP")  # Original F1.
    wait_screen(app, "Command Palette")
    app.paste(command)

    def first_result():
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
        # The input is at +1, a spacer at +2 and selected first result at +3.
        # Ignore shortcut suffixes without matching query/Explorer/editor text.
        selected = lines[row + 3][left + 1:right].strip(" │")
        return selected.split("  ", 1)[0].strip() == command

    try:
        eventually(first_result)
    except AssertionError as error:
        raise AssertionError(f"Palette first result {command!r}:\n{app.screen.text()}") from error
    app.send(b"\r")
    wait_screen(app, absent=("Command Palette",))


def chord(app):
    """Original pin/unpin chord, with a positive first-key acknowledgement."""
    app.send(CTRL_K)
    wait_screen(app, "(ctrl+k) waiting for second key")
    app.send(SHIFT_ENTER)


def state(app, expected):
    """Exact ordered public strips: (filename, sticky marker, modified marker)."""
    snapshot = ""

    def ready():
        nonlocal snapshot
        app.read()
        snapshot = app.screen.text()
        observed = {}
        for line in snapshot.splitlines():
            groups = list(re.finditer(r"([1-4]) ·", line))
            for index, group in enumerate(groups):
                end = groups[index + 1].start() if index + 1 < len(groups) else len(line)
                segment = line[group.end():end]
                names = list(re.finditer(r"[abcd]\.txt", segment))
                if not names:
                    continue
                number = int(group.group(1))
                assert number not in observed, f"Duplicate group-{number} strip:\n{snapshot}"
                tabs = []
                previous_end = 0
                for name_index, name in enumerate(names):
                    following = names[name_index + 1].start() if name_index + 1 < len(names) else len(segment)
                    prefix = segment[previous_end:name.start()]
                    suffix = segment[name.end():following]
                    sticky = re.search(r"◆\s*$", prefix) is not None
                    dirty = re.match(r"\s+●(?:\s|$)", suffix) is not None
                    tabs.append((name.group(), sticky, dirty))
                    previous_end = name.end()
                observed[number] = tabs
        return observed == {index + 1: tabs for index, tabs in enumerate(expected)}

    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Sticky strips {expected!r}:\n{snapshot}") from error


def tab(name, sticky=False, dirty=False):
    return (f"{name}.txt", sticky, dirty)


def unchanged(paths, except_names=()):
    for name, path in paths.items():
        if name not in except_names:
            assert path.read_bytes() == ORIGINAL, (path, path.read_bytes())


def configured(root, name, policy=None, *, workspace_policy=None):
    case, config, paths = setup(root, name)
    path = config / "settings.json"
    values = json.loads(path.read_text(encoding="utf-8"))
    if policy is not None:
        values["workbench.editor.preventPinnedEditorClose"] = policy
    original = ("// user fixture 猫🙂\r\n" + json.dumps(values, ensure_ascii=False, indent=2).replace("\n", "\r\n") + "\r\n").encode()
    path.write_bytes(original)
    workspace_original = None
    if workspace_policy is not None:
        folder = case / ".vscode"
        folder.mkdir()
        workspace_original = ("// workspace fixture β\r\n" + json.dumps({
            "workbench.editor.preventPinnedEditorClose": workspace_policy,
            "[plaintext]": {"workbench.editor.preventPinnedEditorClose": "never"},
        }, ensure_ascii=False, indent=2).replace("\n", "\r\n") + "\r\n").encode()
        (folder / "settings.json").write_bytes(workspace_original)
    return case, config, paths, original, workspace_original


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-sticky-pty-") as directory:
        root = Path(directory)
        case, config, paths, user, _ = configured(root, "ordering-and-original-keys")
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            app.send(RIGHT * 2)
            active(app, "c.txt", 3)
            app.paste("X")
            active(app, "c.txt", 4)
            changed = "猫🙂X alpha\r\nsecond β line\r\n".encode()
            state(app, [[tab("a"), tab("b"), tab("c", dirty=True)]])
            chord(app)
            state(app, [[tab("c", True, True), tab("a"), tab("b")]])
            palette(app, UNPIN)
            state(app, [[tab("c", dirty=True), tab("a"), tab("b")]])
            open_file(app, paths["b"])
            chord(app)
            state(app, [[tab("b", True), tab("c", dirty=True), tab("a")]])
            chord(app)
            state(app, [[tab("b"), tab("c", dirty=True), tab("a")]])
            palette(app, PIN)
            state(app, [[tab("b", True), tab("c", dirty=True), tab("a")]])
            palette(app, UNPIN)
            state(app, [[tab("b"), tab("c", dirty=True), tab("a")]])
            open_file(app, paths["c"])
            active(app, "c.txt", 4)
            save(app, paths["c"], changed)
            undo(app, paths["c"], ORIGINAL)
            redo(app, paths["c"], changed)
            state(app, [[tab("b"), tab("c"), tab("a")]])
            unchanged(paths, ("c",))
            assert (config / "settings.json").read_bytes() == user
        print("PASS: enhanced original Ctrl+K Shift+Enter and exact Pin/Unpin palette commands reorder only sticky metadata; dirty Unicode/CRLF history and disk bytes survive")

        case, config, paths, _, _ = configured(root, "protected-local-global-force")
        with launched(case, config, paths) as app:
            chord(app)
            state(app, [[tab("a", True)]])
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            open_file(app, paths["a"])
            state(app, [[tab("a", True), tab("b"), tab("c")]])
            app.send(CLOSE)
            active(app, "c.txt", 1)  # Current group's most recent ordinary editor.
            state(app, [[tab("a", True), tab("b"), tab("c")]])
            chord(app)
            state(app, [[tab("a", True), tab("c", True), tab("b")]])
            open_file(app, paths["a"])
            app.send(CLOSE)
            active(app, "b.txt", 1)
            app.send(SPLIT)
            active(app, "b.txt", 1)
            chord(app)
            state(app, [[tab("a", True), tab("c", True), tab("b")], [tab("b", True)]])
            open_file(app, paths["d"])
            app.send(FIRST)
            active(app, "b.txt", 1)
            chord(app)
            state(app, [[tab("a", True), tab("c", True), tab("b", True)], [tab("b", True), tab("d")]])
            app.send(CLOSE)
            active(app, "d.txt", 1)  # Only ordinary editor is in the other group.
            state(app, [[tab("a", True), tab("c", True), tab("b", True)], [tab("b", True), tab("d")]])
            chord(app)
            app.paste("P")
            active(app, "d.txt", 2)
            changed = b"P" + ORIGINAL
            state(app, [[tab("a", True), tab("c", True), tab("b", True)], [tab("b", True), tab("d", True, True)]])
            app.send(CLOSE)
            wait_screen(app, "Pinned editor retained; no unpinned editor to focus", absent=("Unsaved Changes",))
            active(app, "d.txt", 2)
            palette(app, FORCED_CLOSE)
            wait_screen(app, "Save changes to d.txt?", "Cancel")
            app.send(ESCAPE)
            active(app, "d.txt", 2)
            assert paths["d"].read_bytes() == ORIGINAL
            save(app, paths["d"], changed)
            undo(app, paths["d"], ORIGINAL)
            redo(app, paths["d"], changed)
            chord(app)  # The original forced command also accepts an ordinary tab.
            state(app, [[tab("a", True), tab("c", True), tab("b", True)], [tab("b", True), tab("d")]])
            palette(app, FORCED_CLOSE)
            active(app, "b.txt", 1)
            state(app, [[tab("a", True), tab("c", True), tab("b", True)], [tab("b", True)]])
            assert paths["d"].read_bytes() == changed
            unchanged(paths, ("d",))
        print("PASS: ordinary Close focuses local then cross-group nonsticky MRU without closing; all-sticky dirty Close is inert, forced Close preserves Cancel/history and works in either mode")

        case, config, paths, _, _ = configured(root, "captured-group-and-all-subsets")
        with launched(case, config, paths) as app:
            app.paste("X")
            active(app, "a.txt", 2)
            changed_a = b"X" + ORIGINAL
            chord(app)
            app.send(SPLIT)
            active(app, "a.txt", 2)
            open_file(app, paths["b"])
            chord(app)
            open_file(app, paths["c"])
            app.paste("Y")
            active(app, "c.txt", 2)
            changed_c = b"Y" + ORIGINAL
            state(app, [[tab("a", True, True)], [tab("b", True), tab("c", dirty=True), tab("a", dirty=True)]])
            palette(app, CLOSE_GROUP)
            wait_screen(app, "Save changes to c.txt?", "Cancel")
            app.send(ESCAPE)
            active(app, "c.txt", 2)
            state(app, [[tab("a", True, True)], [tab("b", True), tab("c", dirty=True), tab("a", dirty=True)]])
            assert paths["a"].read_bytes() == paths["c"].read_bytes() == ORIGINAL
            save(app, paths["c"], changed_c)
            undo(app, paths["c"], ORIGINAL)
            redo(app, paths["c"], changed_c)
            palette(app, CLOSE_GROUP)
            active(app, "b.txt", 1)
            state(app, [[tab("a", True, True)], [tab("b", True)]])
            assert "Unsaved Changes" not in app.screen.text(), app.screen.text()
            assert paths["a"].read_bytes() == ORIGINAL
            app.send(FIRST)
            active(app, "a.txt", 2)
            save(app, paths["a"], changed_a)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], changed_a)
            open_file(app, paths["d"])
            app.send(SECOND)
            active(app, "b.txt", 1)
            open_file(app, paths["c"])
            state(app, [[tab("a", True), tab("d")], [tab("b", True), tab("c")]])
            palette(app, CLOSE_ALL)
            state(app, [[tab("a", True)], [tab("b", True)]])
            active(app, "b.txt", 1)
            assert paths["a"].read_bytes() == changed_a
            assert paths["c"].read_bytes() == changed_c
            unchanged(paths, ("a", "c"))
        print("PASS: Cancel retains the captured group subset; group/all close removes only nonsticky memberships, preserving a dirty shared sticky model, its Undo/Redo and exact foreign disks")

        for policy, protected in [("keyboardAndMouse", True), ("keyboard", True), ("mouse", False), ("never", False)]:
            case, config, paths, user, _ = configured(root, f"policy-{policy}", policy)
            with launched(case, config, paths) as app:
                chord(app)
                state(app, [[tab("a", True)]])
                open_file(app, paths["b"])
                open_file(app, paths["a"])
                state(app, [[tab("a", True), tab("b")]])
                app.send(CLOSE)
                active(app, "b.txt", 1)
                state(app, [[tab("a", True), tab("b")]] if protected else [[tab("b")]])
                unchanged(paths)
                assert (config / "settings.json").read_bytes() == user
        case, config, paths, user, workspace = configured(root, "workspace-over-language", "never", workspace_policy="keyboard")
        with launched(case, config, paths) as app:
            chord(app)
            open_file(app, paths["b"])
            open_file(app, paths["a"])
            state(app, [[tab("a", True), tab("b")]])
            app.send(CLOSE)
            active(app, "b.txt", 1)
            state(app, [[tab("a", True), tab("b")]])
            unchanged(paths)
            assert (config / "settings.json").read_bytes() == user
            assert (case / ".vscode/settings.json").read_bytes() == workspace
        print("PASS: all four root keyboard-close policy values and workspace-over-user precedence behave distinctly; ignored language override and original Unicode/CRLF settings bytes stay unchanged (five sessions, no mouse claim)")

        case, config, paths, _, _ = configured(root, "shared-unicode-undo")
        with launched(case, config, paths) as app:
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            chord(app)
            app.send(SPLIT)
            active(app, "a.txt", 3)  # Existing split copies the current view.
            state(app, [[tab("a", True)], [tab("a")]])
            app.send(LEFT)
            active(app, "a.txt", 2)
            app.send(FIRST)
            active(app, "a.txt", 3)
            app.send(SECOND)
            active(app, "a.txt", 2)
            app.paste("Y")
            active(app, "a.txt", 3)
            first = "猫Y🙂 alpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], first)
            app.send(FIRST)
            active(app, "a.txt", 4)  # Inactive view maps through the shared insertion.
            app.paste("X")
            active(app, "a.txt", 5)
            both = "猫Y🙂X alpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], both)
            app.send(SECOND)
            active(app, "a.txt", 3)
            undo(app, paths["a"], first)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], first)
            redo(app, paths["a"], both)
            state(app, [[tab("a", True)], [tab("a")]])
            app.send(FIRST)
            active(app, "a.txt", 5)
            app.send(CTRL_Z)
            active(app, "a.txt", 4)
            chord(app)  # Metadata change must preserve this pending Redo.
            state(app, [[tab("a", dirty=True)], [tab("a", dirty=True)]])
            app.send(REDO)
            active(app, "a.txt", 5)
            save(app, paths["a"], both)
            state(app, [[tab("a")], [tab("a")]])
            unchanged(paths, ("a",))
        print("PASS: sticky source and ordinary shared split retain distinct Unicode carets; mapped edits and cross-group Undo/Redo preserve exact CRLF bytes, including Redo across Unpin")


if __name__ == "__main__":
    run()
