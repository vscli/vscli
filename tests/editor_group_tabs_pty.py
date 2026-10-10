#!/usr/bin/env python3
"""Committed editor-group tabs through original Linux shortcuts and native PTYs.

Six isolated journeys preserve byte-exact Unicode/CRLF files and terminal mode.
They qualify native App views, dirty close, session restart and an actual held LSP
save participant separately from the pure upstream membership comparator.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import re
import sys
import tempfile
import threading
import time


def fixture_server(root):
    """A real bounded framed peer: one positive held source-action callback."""
    output_lock = threading.Lock()
    trace_lock = threading.Lock()
    documents = {}
    count = 0

    def trace(kind, **fields):
        nonlocal count
        with trace_lock:
            count += 1
            assert count <= 128
            with (root / "protocol.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps({"kind": kind, **fields}, ensure_ascii=False) + "\n")

    def send(message):
        body = json.dumps({"jsonrpc": "2.0", **message}, ensure_ascii=False).encode()
        assert len(body) <= 2 * 1024 * 1024
        with output_lock:
            sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()

    def held(ident):
        deadline = time.monotonic() + 15
        while not (root / "release-action").exists():
            if time.monotonic() >= deadline:
                send({"id": ident, "error": {"code": -32603, "message": "fixture gate expired"}})
                trace("expired", id=ident)
                return
            time.sleep(0.01)
        send({"id": ident, "result": []})
        trace("released", id=ident)

    while True:
        headers, total = {}, 0
        while True:
            line = sys.stdin.buffer.readline(8193)
            if not line:
                return
            total += len(line)
            assert total <= 8192
            if line == b"\r\n":
                break
            key, value = line.decode().split(":", 1)
            headers[key.lower()] = value.strip()
        size = int(headers["content-length"])
        assert 0 <= size <= 16 * 1024 * 1024
        body = sys.stdin.buffer.read(size)
        assert len(body) == size
        message = json.loads(body)
        method, ident, params = message.get("method"), message.get("id"), message.get("params", {})
        if method == "initialize":
            send({"id": ident, "result": {"capabilities": {
                "textDocumentSync": {"openClose": True, "change": 1, "save": {"includeText": True}},
                "codeActionProvider": {"codeActionKinds": ["source.fixAll"]}}}})
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            doc = params["textDocument"]
            text = doc["text"] if method.endswith("didOpen") else params["contentChanges"][-1]["text"]
            documents[doc["uri"]] = text
            trace("open" if method.endswith("didOpen") else "change", uri=doc["uri"], text=text)
            if method.endswith("didOpen"):
                send({"method": "window/showMessage", "params": {"type": 3, "message": "group save peer ready"}})
        elif method == "textDocument/codeAction":
            assert params["context"]["only"] == ["source.fixAll"]
            assert params["context"]["triggerKind"] == 2
            uri = params["textDocument"]["uri"]
            trace("entered", id=ident, uri=uri, text=documents[uri])
            threading.Thread(target=held, args=(ident,), daemon=True).start()
        elif method == "$/cancelRequest":
            trace("cancel", id=params["id"])
        elif method == "textDocument/didSave":
            trace("saved", uri=params["textDocument"]["uri"], text=params["text"])
        elif method == "shutdown":
            send({"id": ident, "result": None})
        elif method == "exit":
            return


if __name__ == "__main__" and sys.argv[1:2] == ["--server"]:
    fixture_server(Path(sys.argv[2]))
    raise SystemExit(0)

from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import REDO

NEXT = b"\x1b[6;5~"  # Original Ctrl+PageDown: global sequential navigation.
PREVIOUS = b"\x1b[5;5~"  # Original Ctrl+PageUp.
SPLIT = b"\x1b[92;5u"  # Original Ctrl+backslash with enhanced delivery.
FIRST = b"\x1b[49;5u"  # Original Ctrl+1.
SECOND = b"\x1b[50;5u"  # Original Ctrl+2.
CLOSE = b"\x17"  # Original Ctrl+W.
ESCAPE = b"\x1b[27u"
RIGHT = b"\x1b[C"
ORIGINAL = "猫🙂 alpha\r\nsecond β line\r\n".encode()


def status(app):
    lines = [line for line in app.screen.text().splitlines() if " | " in line and ("Ln " in line or "No open editors" in line)]
    assert len(lines) == 1, app.screen.text()
    return lines[0]


def active(app, name, column=None):
    def ready():
        app.read()
        lines = [line for line in app.screen.text().splitlines() if " | " in line and "Ln " in line]
        if len(lines) != 1:
            return False
        line = lines[0]
        actual = line.split("|")[0].strip().removesuffix("*").strip()
        return actual == name and (column is None or f"Ln 1, Col {column} " in line)
    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Waiting for active {name}, column {column}:\n{app.screen.text()}") from error


def strips(app, expected):
    """Find public group labels anywhere on screen; no fixed row/column oracle."""
    def ready():
        app.read()
        observed = {}
        for line in app.screen.text().splitlines():
            matches = list(re.finditer(r"([1-4]) ·", line))
            for index, match in enumerate(matches):
                end = matches[index + 1].start() if index + 1 < len(matches) else len(line)
                names = re.findall(r"[abcd]\.(?:txt|cpp)", line[match.end():end])
                if names:
                    observed[int(match.group(1))] = names
        return observed == {index + 1: names for index, names in enumerate(expected)}
    try:
        eventually(ready)
    except AssertionError as error:
        raise AssertionError(f"Waiting for group strips {expected}:\n{app.screen.text()}") from error


def palette(app, command):
    app.send(b"\x1bOP")  # Original F1.
    wait_screen(app, "Command Palette")
    app.send(command + "\r")
    wait_screen(app, absent=("Command Palette",))


def open_file(app, path):
    app.send(b"\x0f")
    wait_screen(app, "Open File (absolute or workspace-relative)")
    app.paste(str(path))
    app.send(b"\r")
    active(app, path.name)


def save(app, path, expected):
    app.send(CTRL_S)
    eventually(lambda: app.read() and path.read_bytes() == expected, timeout=6)
    wait_screen(app, "Saved", timeout=6)
    assert "*" not in status(app).split("|")[0], app.screen.text()


def undo(app, path, expected):
    app.send(CTRL_Z)
    save(app, path, expected)


def redo(app, path, expected):
    app.send(REDO)
    save(app, path, expected)


def records(case):
    path = case / "protocol.jsonl"
    if not path.exists():
        return []
    data = path.read_bytes()
    assert len(data) <= 128 * 16 * 1024
    return [json.loads(line) for line in data.splitlines(keepends=True) if line.endswith(b"\n")]


def acknowledged(app, case, kind, predicate=lambda item: True):
    eventually(lambda: app.read() and any(item["kind"] == kind and predicate(item) for item in records(case)), timeout=6)


def setup(root, name, *, peer=False):
    case = root / name
    case.mkdir()
    config = case / "config"
    config.mkdir()
    suffix = "cpp" if peer else "txt"
    paths = {name: case / f"{name}.{suffix}" for name in "abcd"}
    for path in paths.values():
        path.write_bytes(ORIGINAL)
    values = {"breadcrumbs.enabled": False, "editor.quickSuggestions": False,
              "editor.parameterHints.enabled": False, "files.autoSave": "off"}
    if peer:
        values.update({"editor.codeActionsOnSave": {"source.fixAll": "explicit"},
                       "[cpp]": {"vscli.languageServer.program": str(Path(sys.executable).resolve()),
                                 "vscli.languageServer.args": [str(Path(__file__).resolve()), "--server", str(case)]}})
    else:
        values["vscli.languageServer.enabled"] = False
    (config / "settings.json").write_text(json.dumps(values), encoding="utf-8")
    return case, config, paths


@contextmanager
def launched(case, config, paths, *, restore=False, peer=False, session=False):
    args = ["--config-dir", config, "--extension-node", case / "missing-node"]
    if not session:
        args.append("--no-session")
    if restore:
        args.append("--restore-session")
    else:
        args.append(paths["a"])
    app = Editor(case, *args, enhanced=True, auto_lsp=peer, extra_env={"PATH": ""})
    finished = False
    try:
        if restore:
            wait_screen(app, "Restored clean-file session")
        else:
            active(app, paths["a"].name, 1)
        yield app
        app.finish()
        finished = True
    except BaseException:
        print(f"Group tabs PTY failure ({case.name}):\n{app.screen.text()}", file=sys.stderr)
        if peer:
            print(json.dumps(records(case), ensure_ascii=False, indent=2), file=sys.stderr)
        raise
    finally:
        if not finished:
            (case / "release-action").touch()
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, "Failed group workflow leaked terminal mode"
                app.close_fds()
            except Exception as error:
                print(f"Group tabs cleanup: {error}", file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix="vscli-group-pty-") as directory:
        root = Path(directory)
        case, config, paths = setup(root, "navigation")
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            open_file(app, paths["b"])
            open_file(app, paths["d"])
            strips(app, [["a.txt", "b.txt", "d.txt", "c.txt"]])
            app.send(SPLIT)
            active(app, "d.txt", 1)
            open_file(app, paths["c"])
            strips(app, [["a.txt", "b.txt", "d.txt", "c.txt"], ["d.txt", "c.txt"]])
            app.send(FIRST)
            active(app, "d.txt", 1)
            app.send(NEXT)
            active(app, "c.txt", 1)
            app.send(NEXT)
            active(app, "d.txt", 1)
            app.send(PREVIOUS)
            active(app, "c.txt", 1)
            palette(app, "View: Next Editor in Group")
            active(app, "a.txt", 1)
            palette(app, "View: Previous Editor in Group")
            active(app, "c.txt", 1)
            strips(app, [["a.txt", "b.txt", "d.txt", "c.txt"], ["d.txt", "c.txt"]])
            assert all(path.read_bytes() == ORIGINAL for path in paths.values())
        print("PASS: independent ordered group strips and original split/focus/global/InGroup navigation preserve committed tabs and bytes")

        case, config, paths = setup(root, "shared-views")
        with launched(case, config, paths) as app:
            app.send(RIGHT * 2)
            active(app, "a.txt", 3)
            open_file(app, paths["b"])
            app.send(SPLIT)
            open_file(app, paths["a"])
            active(app, "a.txt", 1)  # Ordinary new membership starts at origin.
            app.send(RIGHT)
            active(app, "a.txt", 2)
            open_file(app, paths["c"])
            app.send(FIRST)
            open_file(app, paths["a"])
            active(app, "a.txt", 3)
            app.paste("X")
            first = "猫🙂X alpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], first)
            app.send(SECOND)
            open_file(app, paths["a"])
            active(app, "a.txt", 2)
            app.paste("Y")
            both = "猫Y🙂X alpha\r\nsecond β line\r\n".encode()
            save(app, paths["a"], both)
            app.send(FIRST)
            open_file(app, paths["a"])
            undo(app, paths["a"], first)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], first)
            redo(app, paths["a"], both)
            assert paths["b"].read_bytes() == paths["c"].read_bytes() == ORIGINAL
        print("PASS: distinct Unicode group carets survive inactive tabs; shared edits and cross-group Undo/Redo preserve exact CRLF bytes")

        case, config, paths = setup(root, "dirty-close")
        with launched(case, config, paths) as app:
            app.send(SPLIT)
            strips(app, [["a.txt"], ["a.txt"]])
            app.paste("DIRTY ")
            changed = b"DIRTY " + ORIGINAL
            wait_screen(app, "a.txt *")
            app.send(CLOSE)
            active(app, "a.txt")
            strips(app, [["a.txt"]])
            assert "Save changes to" not in app.screen.text(), app.screen.text()
            assert paths["a"].read_bytes() == ORIGINAL
            app.send(CLOSE)
            wait_screen(app, "Save changes to a.txt?", "Cancel")
            app.send(ESCAPE)
            active(app, "a.txt")
            strips(app, [["a.txt"]])
            assert paths["a"].read_bytes() == ORIGINAL
            save(app, paths["a"], changed)
            undo(app, paths["a"], ORIGINAL)
            redo(app, paths["a"], changed)
            app.send(CLOSE)
            wait_screen(app, "No open editors")
            assert paths["a"].read_bytes() == changed
        print("PASS: closing one dirty shared membership retains model/history without a dialog; last dirty Cancel preserves work before Save/Undo/Redo and welcome")

        case, config, paths = setup(root, "mru-close")
        with launched(case, config, paths) as app:
            open_file(app, paths["b"])
            open_file(app, paths["c"])
            open_file(app, paths["a"])
            open_file(app, paths["b"])
            app.send(CLOSE)
            active(app, "a.txt", 1)
            strips(app, [["a.txt", "c.txt"]])
            app.send(SPLIT)
            open_file(app, paths["b"])
            app.send(FIRST)
            palette(app, "View: Close Editor Group")
            active(app, "b.txt", 1)
            strips(app, [["a.txt", "b.txt"]])
            app.send(CLOSE)
            active(app, "a.txt", 1)
            app.send(CLOSE)
            wait_screen(app, "No open editors")
            assert all(path.read_bytes() == ORIGINAL for path in paths.values())
        print("PASS: non-adjacent MRU close fallback and original Close Editor Group remove only intended memberships; final close reaches native welcome")

        case, config, paths = setup(root, "session-views")
        with launched(case, config, paths, session=True) as app:
            app.send(RIGHT * 2)
            open_file(app, paths["b"])
            app.send(RIGHT * 4)
            app.send(SPLIT)
            active(app, "b.txt", 5)
            open_file(app, paths["a"])
            app.send(RIGHT)
            open_file(app, paths["c"])
            app.send(RIGHT * 3)
            palette(app, "View: Previous Editor in Group")
            active(app, "a.txt", 2)
            palette(app, "View: Previous Editor in Group")
            active(app, "b.txt", 5)
            app.send(FIRST)
            open_file(app, paths["a"])
            active(app, "a.txt", 3)
            strips(app, [["a.txt", "b.txt"], ["b.txt", "a.txt", "c.txt"]])
        with launched(case, config, paths, restore=True, session=True) as app:
            active(app, "a.txt", 3)
            strips(app, [["a.txt", "b.txt"], ["b.txt", "a.txt", "c.txt"]])
            app.send(SECOND)
            active(app, "b.txt", 5)
            palette(app, "View: Next Editor in Group")
            active(app, "a.txt", 2)
            palette(app, "View: Next Editor in Group")
            active(app, "c.txt", 4)
            app.paste("Z")
            edited = "猫🙂 Zalpha\r\nsecond β line\r\n".encode()
            save(app, paths["c"], edited)
            undo(app, paths["c"], ORIGINAL)
            redo(app, paths["c"], edited)
            app.send(FIRST)
            active(app, "a.txt", 3)
            palette(app, "View: Next Editor in Group")
            active(app, "b.txt", 5)
            assert paths["a"].read_bytes() == paths["b"].read_bytes() == ORIGINAL
        print("PASS: clean session restart restores independent ordered group inventories, active group and inactive historical Unicode carets; saved edits retain Undo/Redo")

        case, config, paths = setup(root, "held-save-close", peer=True)
        with launched(case, config, paths, peer=True) as app:
            wait_screen(app, "group save peer ready")
            acknowledged(app, case, "open", lambda item: item["uri"] == paths["a"].resolve().as_uri())
            open_file(app, paths["b"])
            open_file(app, paths["a"])
            app.paste("PENDING ")
            changed = b"PENDING " + ORIGINAL
            acknowledged(app, case, "change", lambda item: item["text"].encode() == changed)
            app.send(CTRL_S)
            acknowledged(app, case, "entered", lambda item: item["text"].encode() == changed)
            app.send(CLOSE)
            wait_screen(app, "Waiting for pending save before closing")
            app.send(NEXT)
            active(app, "b.cpp", 1)
            # File gate releases the actual callback after focus has positively
            # changed. No filesystem-worker timing or fake receipt is inferred.
            (case / "release-action").touch()
            acknowledged(app, case, "released")
            acknowledged(app, case, "saved", lambda item: item["uri"] == paths["a"].resolve().as_uri() and item["text"].encode() == changed)
            eventually(lambda: app.read() and paths["a"].read_bytes() == changed, timeout=6)
            active(app, "b.cpp", 1)
            strips(app, [["b.cpp"]])
            assert paths["b"].read_bytes() == ORIGINAL
            assert sum(item["kind"] == "entered" for item in records(case)) == 1
            assert not any(item["kind"] == "expired" for item in records(case))
            app.send(CLOSE)
            wait_screen(app, "No open editors")
        print("PASS: positively held native save participant then tab switch saves/closes only the original membership; focused historical target and disk remain intact")


if __name__ == "__main__":
    run()
