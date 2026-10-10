#!/usr/bin/env python3
"""Persistent native Breadcrumbs commands through the original terminal keys."""
from contextlib import contextmanager
from pathlib import Path
import sys
import tempfile

from navigation_history_pty import active, open_file
from pty_smoke import CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import REDO, save, undo, redo


SELECT = b"\x1b[46;6u"  # Original Linux Ctrl+Shift+Period, enhanced delivery.
ESCAPE = b"\x1b[27u"
ORIGINAL = "猫🙂 original\r\n".encode()
SETTINGS = ("{\r\n"
            "  // 猫🙂 keep this JSONC comment and byte order\r\n"
            "  \"breadcrumbs.enabled\": true, // trailing comment\r\n"
            "  \"vscli.languageServer.enabled\": false,\r\n"
            "  \"unknown.setting\": {\"breadcrumbs.enabled\": \"unchanged\"},\r\n"
            "}\r\n").encode()


def disabled(contents):
    return contents.replace(b'"breadcrumbs.enabled": true',
                            b'"breadcrumbs.enabled": false', 1)


def palette(app, label):
    app.send(b"\x1bOP")  # Original F1 palette.
    wait_screen(app, "Command Palette")
    app.paste(label)
    app.send(b"\r")
    eventually(lambda: app.read() and "Command Palette" not in app.screen.text())


def header(app, name, visible):
    snapshot = ""

    def ready():
        nonlocal snapshot
        app.read()
        # One editor pane: tab row 0, Breadcrumbs row 1. Restrict this oracle
        # to the editor area so Explorer and tab labels cannot satisfy it.
        snapshot = "".join(app.screen.cells.get((1, column), " ")
                           for column in range(26, 110))
        return (name in snapshot) == visible

    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Breadcrumb header visible={visible}: {snapshot}\n"
                             f"{app.screen.text()}") from error


def persisted(app, path, expected):
    # Exact disk bytes qualify background persistence; rendering alone could
    # merely be an optimistic presentation override.
    eventually(lambda: app.read() and path.read_bytes() == expected)
    wait_screen(app, absent=("Saving Breadcrumbs setting",))


@contextmanager
def editor(root, name, *, setup=True, workspace=False):
    case = root / name
    config = case / "config"
    source = case / "main.cpp"
    user = config / "settings.json"
    local = case / ".vscode" / "settings.json"
    if setup:
        case.mkdir()
        config.mkdir()
        source.write_bytes(ORIGINAL)
        user.write_bytes(SETTINGS)
        if workspace:
            local.parent.mkdir()
            local.write_bytes(b'{\r\n // workspace comment\r\n "breadcrumbs.enabled": true,\r\n}\r\n')
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 "--no-lsp", "--no-session", source, enhanced=True,
                 extra_env={"PATH": ""})
    finished = False
    try:
        active(app, "main.cpp", 1)
        yield app, source, user, local
        app.finish()
        assert app.process.restored, "Settings workflow leaked terminal mode"
        finished = True
    except BaseException:
        print(f"Settings PTY failure ({name}):\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not finished:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, "Failed settings workflow leaked terminal mode"
                app.close_fds()
            except Exception as error:
                print(f"Settings PTY cleanup: {error}", file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-settings-persistence-pty-") as directory:
        root = Path(directory)
        with editor(root, "user-persist") as (app, source, user, _local):
            header(app, "main.cpp", True)
            app.paste("DIRTY猫🙂 ")
            wait_screen(app, "DIRTY猫🙂", "main.cpp *")
            assert source.read_bytes() == ORIGINAL
            palette(app, "View: Toggle Breadcrumbs")
            persisted(app, user, disabled(SETTINGS))
            header(app, "main.cpp", False)
            assert source.read_bytes() == ORIGINAL, "Settings toggle saved the dirty source"
            dirty = "DIRTY猫🙂 ".encode() + ORIGINAL
            save(app, source, dirty)
            undo(app, source, ORIGINAL)
            redo(app, source, dirty)
            assert user.read_bytes() == disabled(SETTINGS)
        with editor(root, "user-persist", setup=False) as (app, source, user, _local):
            header(app, "main.cpp", False)
            assert user.read_bytes() == disabled(SETTINGS)
            app.send(SELECT)
            persisted(app, user, SETTINGS)
            header(app, "main.cpp", True)
            # toggleToOn owns Breadcrumbs focus. Return to editor before typing
            # and prove the original shortcut did not touch its native model.
            app.send(ESCAPE)
            active(app, "main.cpp", 1)
            dirty = "DIRTY猫🙂 ".encode() + ORIGINAL
            assert source.read_bytes() == dirty
            app.paste("!")
            save(app, source, b"!" + dirty)
            undo(app, source, dirty)
            redo(app, source, b"!" + dirty)
        print("PASS: F1 Breadcrumbs toggle persists exact Unicode/CRLF JSONC; restart and original Ctrl+Shift+Period restore it without changing source Undo/Redo")

        with editor(root, "workspace-target", workspace=True) as (app, source, user, local):
            original_local = local.read_bytes()
            header(app, "main.cpp", True)
            palette(app, "View: Toggle Breadcrumbs")
            persisted(app, local, disabled(original_local))
            header(app, "main.cpp", False)
            assert user.read_bytes() == SETTINGS
            app.paste("WORKSPACE猫🙂 ")
            edited = "WORKSPACE猫🙂 ".encode() + ORIGINAL
            save(app, source, edited)
            undo(app, source, ORIGINAL)
            redo(app, source, edited)
            assert user.read_bytes() == SETTINGS
            assert local.read_bytes() == disabled(original_local)
        print("PASS: workspace Breadcrumbs winner is persisted in its own JSONC file while user settings and native Unicode/CRLF history remain intact")

        with editor(root, "dirty-settings") as (app, source, user, _local):
            open_file(app, user)
            header(app, "settings.json", True)
            app.paste("// DIRTY猫🙂\r\n")
            wait_screen(app, "DIRTY猫🙂", "settings.json *")
            dirty = "// DIRTY猫🙂\r\n".encode() + SETTINGS
            palette(app, "View: Toggle Breadcrumbs")
            refusal = wait_screen(app, "unsaved changes")
            # A canonical user path refuses immediately; a parent alias can
            # reach the worker's native-identity guard first. Both are explicit
            # refusals and must retain the same bytes/header/history below.
            assert any(marker in refusal for marker in (
                "Breadcrumbs setting unchanged", "Settings write refused"
            )), refusal
            header(app, "settings.json", True)
            assert user.read_bytes() == SETTINGS
            assert source.read_bytes() == ORIGINAL
            # Undo is text-neutral on disk until explicitly saved. Its dirty
            # marker provides an independent editor-history witness.
            app.send(CTRL_Z)
            active(app, "settings.json", 1)
            wait_screen(app, absent=("settings.json *", "DIRTY猫🙂"))
            assert user.read_bytes() == SETTINGS
            app.send(REDO)
            wait_screen(app, "DIRTY猫🙂", "settings.json *")
            save(app, user, dirty)
            undo(app, user, SETTINGS)
            redo(app, user, dirty)
            assert source.read_bytes() == ORIGINAL
        print("PASS: dirty settings buffers refuse persistence, keep Breadcrumbs enabled, and retain exact original bytes and native Undo/Redo")


if __name__ == "__main__":
    run()
