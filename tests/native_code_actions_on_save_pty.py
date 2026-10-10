#!/usr/bin/env python3
"""Original terminal Save keys with automatically configured native source actions.

Five isolated Unix PTY journeys use an absolute stdlib Python LSP peer, an empty
PATH, and missing Node. Protocol barriers qualify actual callback settlement;
committed Unicode/CRLF bytes qualify the model and filesystem save receipts.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile
import threading
import time


def fixture_server(root, mode):
    output_lock = threading.Lock()
    trace_lock = threading.Lock()
    documents = {}
    entered = 0
    observed = 0

    def trace(kind, **fields):
        nonlocal observed
        with trace_lock:
            observed += 1
            assert observed <= 512, "Save-action fixture event budget exceeded"
            with (root / "protocol.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps({"kind": kind, **fields}, ensure_ascii=False) + "\n")

    def send(message):
        body = json.dumps({"jsonrpc": "2.0", **message}, ensure_ascii=False).encode()
        assert len(body) <= 2 * 1024 * 1024
        with output_lock:
            sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()

    def replace_line(document, before, after):
        lines = document["text"].splitlines()
        assert lines.count(before) == 1, f"Unexpected action input: {document['text']!r}"
        line = lines.index(before)
        return {"documentChanges": [{"textDocument": {"uri": document["uri"], "version": document["version"]},
                                     "edits": [{"range": {"start": {"line": line, "character": 0},
                                                           "end": {"line": line, "character": len(before.encode('utf-16-le')) // 2}},
                                                "newText": after}]}]}

    def action(kind, edit):
        return {"title": "Fixture " + kind, "kind": kind, "edit": edit}

    def held_reply(ident, document):
        deadline = time.monotonic() + 20
        while not (root / "release-action").exists():
            if time.monotonic() >= deadline:
                send({"id": ident, "error": {"code": -32603, "message": "fixture gate expired"}})
                trace("action-expired", id=ident)
                return
            time.sleep(0.01)
        send({"id": ident, "result": [action("source.fixAll", replace_line(document, "int value=1;", "int value=666;"))]})
        trace("action-replied", id=ident, late=True)
        # Ordered after the result: visible receipt proves actual release was
        # consumed, rather than merely that the server wrote bytes to a pipe.
        send({"method": "window/showMessage", "params": {"type": 3, "message": "late action settled"}})

    while True:
        headers = {}
        header_bytes = 0
        while True:
            line = sys.stdin.buffer.readline(8193)
            if not line:
                return
            header_bytes += len(line)
            assert header_bytes <= 8192
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
                "codeActionProvider": {"resolveProvider": True, "codeActionKinds": ["source.fixAll", "source.organizeImports"]},
                "documentFormattingProvider": True}}})
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            incoming = params["textDocument"]
            text = incoming["text"] if method.endswith("didOpen") else params["contentChanges"][-1]["text"]
            documents[incoming["uri"]] = {"uri": incoming["uri"], "version": incoming["version"], "text": text}
            trace("open" if method.endswith("didOpen") else "change", text=text, version=incoming["version"])
            if method.endswith("didOpen"):
                send({"method": "window/showMessage", "params": {"type": 3, "message": "native save actions ready"}})
        elif method == "textDocument/codeAction":
            entered += 1
            document = dict(documents[params["textDocument"]["uri"]])
            context = params["context"]
            assert context["triggerKind"] == 2
            assert len(context.get("diagnostics", [])) <= 128
            assert context["only"] in (["source.fixAll"], ["source.organizeImports"])
            family = context["only"][0]
            trace("action-entered", id=ident, number=entered, family=family, text=document["text"], version=document["version"])
            if mode in ("held", "cancel") and entered == 1:
                assert family == "source.fixAll"
                threading.Thread(target=held_reply, args=(ident, document), daemon=True).start()
                continue
            result = []
            if not (root / "null-participants").exists() and mode == "direct":
                if family == "source.fixAll":
                    result = [action(family, replace_line(document, "int value=1;", "int value=2;"))]
                else:
                    assert "int value=2;" in document["text"], "Organize discovery did not observe the preceding fix"
                    assert not document["text"].startswith("#include")
                    edit = {"documentChanges": [{"textDocument": {"uri": document["uri"], "version": document["version"]},
                                                  "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                                                             "newText": "#include <vector>\r\n"}]}]}
                    result = [action(family, edit)]
            elif mode == "combined" and family == "source.fixAll":
                result = [action(family, replace_line(document, "int value=1;", "int value=999;"))]
                result[0]["command"] = {"command": "fixture.unavailable", "arguments": []}
            send({"id": ident, "result": result})
            trace("action-replied", id=ident, noop=not result)
        elif method == "textDocument/formatting":
            document = documents[params["textDocument"]["uri"]]
            text = document["text"]
            trace("format-entered", id=ident, text=text)
            result = None
            if mode in ("direct", "combined") and not (root / "null-participants").exists():
                before = "int value=2;" if mode == "direct" else "int value=1;"
                if mode == "direct":
                    assert text.startswith("#include <vector>\r\n"), "Formatter ran before imports"
                change = replace_line(document, before, before.replace("=", " = "))
                result = change["documentChanges"][0]["edits"]
            send({"id": ident, "result": result})
            trace("format-replied", id=ident, noop=result is None)
        elif method in ("workspace/executeCommand", "codeAction/resolve"):
            trace("unexpected-action-followup", method=method)
            send({"id": ident, "error": {"code": -32601, "message": "Unexpected fixture followup"}})
        elif method == "$/cancelRequest":
            trace("cancel", id=params["id"])
        elif method == "textDocument/didSave":
            assert "text" in params, "Committed didSave snapshot missing"
            trace("save", text=params["text"])
        elif method == "shutdown":
            send({"id": ident, "result": None})
        elif method == "exit":
            return


if __name__ == "__main__" and sys.argv[1:2] == ["--server"]:
    fixture_server(Path(sys.argv[2]), sys.argv[3])
    raise SystemExit(0)

from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen
from smart_typing_pty import REDO

CLOSE = b"\x17"  # Original Linux Ctrl+W.
ESCAPE = b"\x1b[27u"  # Enhanced original Escape, without legacy ambiguity.
ORIGINAL = "猫🙂 e\u0301\r\nint value=1;\r\n".encode()
PREFIX = "DIRTY λ🙂 ".encode()
RAW = PREFIX + ORIGINAL
FIXED = RAW.replace(b"int value=1;", b"int value=2;")
ORGANIZED = b"#include <vector>\r\n" + FIXED
FORMATTED = ORGANIZED.replace(b"int value=2;", b"int value = 2;")
COMBINED_FORMATTED = RAW.replace(b"int value=1;", b"int value = 1;")


def records(case):
    trace = case / "protocol.jsonl"
    if not trace.exists():
        return []
    data = trace.read_bytes()
    assert len(data) <= 512 * 16 * 1024, "Fixture trace byte budget exceeded"
    return [json.loads(line) for line in data.splitlines(keepends=True) if line.endswith(b"\n")]


def acknowledged(app, case, kind, predicate=lambda event: True):
    eventually(lambda: app.read() and any(event["kind"] == kind and predicate(event) for event in records(case)), timeout=6)


def persisted(app, source, expected):
    eventually(lambda: app.read() and source.read_bytes() == expected, timeout=6)
    wait_screen(app, "Saved", absent=("main.cpp *",))


def saved(app, source, case, expected):
    before = sum(event["kind"] == "save" for event in records(case))
    app.send(CTRL_S)
    persisted(app, source, expected)

    def receipt():
        app.read()
        saves = [event for event in records(case) if event["kind"] == "save"]
        return len(saves) == before + 1 and saves[-1]["text"].encode() == expected
    eventually(receipt, timeout=6)
    wait_screen(app, "Saved", absent=("main.cpp *",))


def caret(app, row, column):
    wait_screen(app, f"Ln {row}, Col {column}", "1 cursor(s)")


def dirty(app, case, *, autosave=False):
    app.paste(PREFIX.decode())
    wait_screen(app, "DIRTY λ🙂", *(() if autosave else ("main.cpp *",)))
    acknowledged(app, case, "change", lambda event: event["text"].encode() == RAW)
    caret(app, 1, 10)


@contextmanager
def editor(root, name, mode, *, autosave=False):
    case = root / name
    case.mkdir()
    source = case / "main.cpp"
    source.write_bytes(ORIGINAL)
    config = case / "config"
    config.mkdir()
    values = {"breadcrumbs.enabled": False, "editor.quickSuggestions": False,
              "editor.parameterHints.enabled": False, "editor.formatOnSave": True,
              "editor.formatOnSaveMode": "file", "editor.tabSize": 2, "editor.insertSpaces": True,
              "editor.codeActionsOnSave": {"source.fixAll": "always", "source.organizeImports": "always"},
              "files.autoSave": "afterDelay" if autosave else "off", "files.autoSaveDelay": 200,
              "[cpp]": {"vscli.languageServer.program": str(Path(sys.executable).resolve()),
                        "vscli.languageServer.args": [str(Path(__file__).resolve()), "--server", str(case), mode]}}
    (config / "settings.json").write_text(json.dumps(values), encoding="utf-8")
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node", "--no-session",
                 source, enhanced=True, auto_lsp=True, extra_env={"PATH": ""})
    completed = False
    try:
        wait_screen(app, "native save actions ready")
        acknowledged(app, case, "open", lambda event: event["text"].encode() == ORIGINAL)
        yield app, source, case
        assert not any(event["kind"] == "unexpected-action-followup" for event in records(case)), "Unsupported commands were executed"
        app.finish()
        completed = True
    except BaseException:
        print(f"Code-actions-on-save PTY failure ({name}):\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not completed:
            # Release the real ignored-cancel callback before supervising exit.
            (case / "release-action").touch()
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, "Failed save-action workflow leaked terminal mode"
                app.close_fds()
            except Exception as error:
                print(f"Save-action cleanup: {error}", file=sys.stderr)


def run():
    with tempfile.TemporaryDirectory(prefix="vs-act-") as directory:
        root = Path(directory)
        with editor(root, "a", "direct") as (app, source, case):
            dirty(app, case)
            assert source.read_bytes() == ORIGINAL
            saved(app, source, case, FORMATTED)
            caret(app, 2, 10)
            callbacks = [event for event in records(case) if event["kind"] in ("action-entered", "format-entered")]
            assert [event["kind"] for event in callbacks] == ["action-entered", "action-entered", "format-entered"]
            assert [event.get("family") for event in callbacks[:2]] == ["source.fixAll", "source.organizeImports"]
            assert [event["text"].encode() for event in callbacks] == [RAW, FIXED, ORGANIZED]
            assert callbacks[1]["version"] > callbacks[0]["version"], "Fresh family request reused the old synchronized lifetime"
            # No-op participants keep Redo intact while disk receipts expose the
            # exact current model after every independent text-stage Undo.
            (case / "null-participants").touch()
            for expected in (ORGANIZED, FIXED, RAW, ORIGINAL):
                app.send(CTRL_Z)
                saved(app, source, case, expected)
                caret(app, 2 if expected.startswith(b"#include") else 1, 1 if expected == ORIGINAL else 10)
            for expected in (RAW, FIXED, ORGANIZED, FORMATTED):
                app.send(REDO)
                saved(app, source, case, expected)
                caret(app, 2 if expected.startswith(b"#include") else 1, 10)
        print("PASS: automatic native server, fresh fix-all/imports/format stages, exact Unicode/CRLF, four Undo/Redo steps and no-op saves preserve history")

        with editor(root, "b", "combined") as (app, source, case):
            dirty(app, case)
            saved(app, source, case, COMBINED_FORMATTED)
            wait_screen(app, "Saved", "command")
            trace = records(case)
            formats = [event for event in trace if event["kind"] == "format-entered"]
            assert len(formats) == 1 and formats[0]["text"].encode() == RAW
            assert not any(b"999" in event.get("text", "").encode() for event in trace)
            caret(app, 1, 10)
        print("PASS: combined command/edit action is skipped before mutation, formatting sees raw input, and the save notice survives committed acknowledgement")

        with editor(root, "c", "held") as (app, source, case):
            dirty(app, case)
            saved(app, source, case, RAW)
            wait_screen(app, "Saved", "1500 ms")
            first = [event for event in records(case) if event["kind"] == "action-entered"]
            assert len(first) == 1
            acknowledged(app, case, "cancel", lambda event: event["id"] == first[0]["id"])
            app.paste("LATEST ")
            latest = PREFIX + b"LATEST " + ORIGINAL
            acknowledged(app, case, "change", lambda event: event["text"].encode() == latest)
            saved(app, source, case, latest)
            assert sum(event["kind"] == "action-entered" for event in records(case)) == 1
            (case / "release-action").touch()
            wait_screen(app, "late action settled")
            wait_screen(app, "int value=1;", "LATEST", absent=("int value=666;",))
            assert source.read_bytes() == latest
            caret(app, 1, 17)
            saved(app, source, case, latest)
            trace = records(case)
            assert sum(event["kind"] == "action-entered" for event in trace) == 3
            assert all(event["text"].encode() == latest for event in trace if event["kind"] == "action-entered" and event["number"] > 1)
            assert sum(event["kind"] == "format-entered" for event in trace) == 3
        print("PASS: 1500ms deadline saves raw bytes; held actual action rejects overlap, late response is inert, and exact release permits fresh discovery")

        with editor(root, "d", "cancel") as (app, source, case):
            dirty(app, case)
            app.send(CTRL_S)
            acknowledged(app, case, "action-entered")
            app.send(CLOSE)
            wait_screen(app, "Waiting for pending save before closing")
            app.send(ESCAPE)
            wait_screen(app, "Save retired", "main.cpp *", "DIRTY λ🙂")
            assert source.read_bytes() == ORIGINAL
            assert not any(event["kind"] == "save" for event in records(case))
            acknowledged(app, case, "cancel")
            (case / "release-action").touch()
            wait_screen(app, "late action settled")
            wait_screen(app, "main.cpp *", "int value=1;", absent=("int value=666;", "No open editors"))
            assert source.read_bytes() == ORIGINAL
            caret(app, 1, 10)
            saved(app, source, case, RAW)
            assert sum(event["kind"] == "action-entered" for event in records(case)) == 3
            app.send(CLOSE)
            wait_screen(app, "No open editors")
            assert source.read_bytes() == RAW
        print("PASS: original pending Ctrl+W then Escape retires the save and close; exact late release retains the dirty origin until a new explicit save")

        with editor(root, "e", "direct", autosave=True) as (app, source, case):
            dirty(app, case, autosave=True)
            persisted(app, source, RAW)
            acknowledged(app, case, "save", lambda event: event["text"].encode() == RAW)
            trace = records(case)
            assert not any(event["kind"] in ("action-entered", "format-entered") for event in trace)
            caret(app, 1, 10)
        print("PASS: after-delay autosave skips even always-configured source actions and formatting, without Node or executable discovery, and restores terminal mode")


if __name__ == "__main__":
    run()
