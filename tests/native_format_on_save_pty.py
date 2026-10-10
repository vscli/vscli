#!/usr/bin/env python3
"""Native format-before-save through original terminal keys and real LSP frames.

The script also serves as its own deterministic stdlib-only formatter process.
No JavaScript runtime or executable lookup on PATH is needed. Every wait has a
positive protocol, terminal, or committed-byte witness; no failed target retries.
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
    records = 0
    count = 0
    documents = {}

    def trace(kind, **fields):
        nonlocal records
        with trace_lock:
            records += 1
            if records > 256:
                raise AssertionError("Formatter fixture event budget exceeded")
            with (root / "protocol.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps({"kind": kind, **fields}, ensure_ascii=False) + "\n")

    def send(message):
        body = json.dumps({"jsonrpc": "2.0", **message}, ensure_ascii=False).encode()
        assert len(body) <= 16 * 1024 * 1024
        with output_lock:
            sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()

    def edits(text):
        line = text.splitlines()[1]
        assert line == "int value=1;", f"Unexpected formatter fixture input: {text!r}"
        return [{"range": {"start": {"line": 1, "character": 0},
                           "end": {"line": 1, "character": len(line.encode('utf-16-le')) // 2}},
                 "newText": "int value = 1;"}]

    def held_reply(ident, text):
        deadline = time.monotonic() + 20
        while not (root / "release-format").exists():
            if time.monotonic() >= deadline:
                send({"id": ident, "error": {"code": -32603, "message": "fixture gate expired"}})
                trace("format-expired", id=ident)
                return
            time.sleep(0.01)
        send({"id": ident, "result": edits(text)})
        trace("format-replied", id=ident, late=True)
        # This notification is ordered AFTER the exact formatter result. Its
        # visible receipt proves the editor consumed that late settlement.
        send({"method": "window/showMessage", "params": {"type": 3, "message": "late formatter settled"}})

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
                "documentFormattingProvider": True}}})
        elif method == "textDocument/didOpen":
            document = params["textDocument"]
            documents[document["uri"]] = document["text"]
            trace("open", text=document["text"])
            send({"method": "window/showMessage", "params": {"type": 3, "message": "native formatter ready"}})
        elif method == "textDocument/didChange":
            text = params["contentChanges"][-1]["text"]
            documents[params["textDocument"]["uri"]] = text
            trace("change", text=text)
        elif method == "textDocument/formatting":
            count += 1
            text = documents[params["textDocument"]["uri"]]
            trace("format-entered", id=ident, number=count, text=text)
            if mode == "failure":
                send({"id": ident, "error": {"code": -32603, "message": "fixture failure"}})
                trace("format-replied", id=ident, error=True)
            elif mode == "held" and count == 1:
                threading.Thread(target=held_reply, args=(ident, text), daemon=True).start()
            else:
                result = None if mode == "held" or (root / "null-format").exists() else edits(text)
                send({"id": ident, "result": result})
                trace("format-replied", id=ident, noop=result is None)
        elif method == "$/cancelRequest":
            trace("cancel", id=params["id"])
        elif method == "textDocument/didSave":
            assert "text" in params, "Negotiated committed save text is missing"
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

ORIGINAL = "猫🙂 e\u0301\r\nint value=1;\r\n".encode()
PREFIX = "DIRTY λ🙂 ".encode()
RAW = PREFIX + ORIGINAL
FORMATTED = RAW.replace(b"int value=1;", b"int value = 1;")


def records(case):
    trace = case / "protocol.jsonl"
    if not trace.exists():
        return []
    # Ignore only an incomplete final append while its writer is active. A
    # complete malformed protocol observation remains an immediate failure.
    lines = trace.read_bytes().splitlines(keepends=True)
    return [json.loads(line) for line in lines if line.endswith(b"\n")]


def acknowledged(app, case, kind, predicate=lambda event: True):
    eventually(lambda: app.read() and any(event["kind"] == kind and predicate(event)
                                         for event in records(case)), timeout=6)


def persisted(app, source, expected):
    eventually(lambda: app.read() and source.read_bytes() == expected, timeout=6)
    wait_screen(app, "Saved", absent=("main.cpp *",))


def saved(app, source, case, expected):
    before = sum(event["kind"] == "save" for event in records(case))
    app.send(CTRL_S)
    persisted(app, source, expected)
    def current_receipt():
        app.read()
        saves = [event for event in records(case) if event["kind"] == "save"]
        return len(saves) == before + 1 and saves[-1]["text"].encode() == expected
    eventually(current_receipt, timeout=6)
    wait_screen(app, "Saved", absent=("main.cpp *",))


@contextmanager
def editor(root, name, mode="edit", *, autosave=False, native_server=True):
    case = root / name
    case.mkdir()
    source = case / "main.cpp"
    source.write_bytes(ORIGINAL)
    config = case / "config"
    config.mkdir()
    settings = {"vscli.languageServer.enabled": False, "breadcrumbs.enabled": False,
                "editor.quickSuggestions": False, "editor.parameterHints.enabled": False,
                "editor.formatOnSave": True, "editor.formatOnSaveMode": "file",
                "files.autoSave": "afterDelay" if autosave else "off", "files.autoSaveDelay": 200}
    (config / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    options = ["--lsp", sys.executable, "--lsp-language", "cpp", "--lsp-arg", str(Path(__file__).resolve()),
               "--lsp-arg=--server", "--lsp-arg", str(case), "--lsp-arg", mode] if native_server else ["--no-lsp"]
    app = Editor(case, "--config-dir", config, "--extension-node", case / "missing-node",
                 "--no-session", *options, source, enhanced=True, extra_env={"PATH": ""})
    completed = False
    try:
        if native_server:
            wait_screen(app, "native formatter ready")
            acknowledged(app, case, "open", lambda event: event["text"].encode() == ORIGINAL)
        yield app, source, case
        app.finish()
        completed = True
    except BaseException:
        print(f"Format-on-save PTY failure ({name}):\n{app.screen.text()}", file=sys.stderr)
        raise
    finally:
        if not completed:
            try:
                app.process.kill()
                app.process.wait(timeout=3)
                assert app.process.restored, "Failed formatter workflow leaked terminal mode"
                app.close_fds()
            except Exception as error:
                print(f"Format-on-save cleanup: {error}", file=sys.stderr)


def dirty(app, case, *, native_server=True, requires_dirty=True):
    app.paste(PREFIX.decode())
    markers = ("DIRTY λ🙂", "main.cpp *") if requires_dirty else ("DIRTY λ🙂",)
    wait_screen(app, *markers)
    if native_server:
        acknowledged(app, case, "change", lambda event: event["text"].encode() == RAW)


def run():
    with tempfile.TemporaryDirectory(prefix="vs-fmt-") as directory:
        root = Path(directory)
        with editor(root, "a") as (app, source, case):
            dirty(app, case)
            assert source.read_bytes() == ORIGINAL
            saved(app, source, case, FORMATTED)
            assert sum(event["kind"] == "format-entered" for event in records(case)) == 1
            app.send(CTRL_Z)
            wait_screen(app, "int value=1;", "main.cpp *", absent=("int value = 1;",))
            assert source.read_bytes() == FORMATTED
            (case / "null-format").touch()
            saved(app, source, case, RAW)
            app.send(REDO)
            wait_screen(app, "int value = 1;", "DIRTY λ🙂")
            saved(app, source, case, FORMATTED)
            trace = records(case)
            assert sum(event["kind"] == "format-entered" for event in trace) == 3
            assert sum(event["kind"] == "format-replied" and event.get("noop", False) for event in trace) == 2
        print("PASS: original Ctrl+S formats exact Unicode/CRLF bytes; separate Undo/Redo and null formatting preserve history and committed LSP text")

        # Exercise a macOS-shaped long workspace path at the existing width;
        # participant error notices must survive the successful save receipt.
        long_root = root / ("workspace-" + "x" * 100)
        long_root.mkdir()
        with editor(long_root, "b", mode="failure") as (app, source, case):
            dirty(app, case)
            saved(app, source, case, RAW)
            wait_screen(app, "Saved", "Formatting skipped", "fixture failure")
            assert sum(event["kind"] == "format-entered" for event in records(case)) == 1
        print("PASS: actual formatter error preserves raw save bytes and a notice after its committed didSave acknowledgement")

        with editor(root, "c", mode="held") as (app, source, case):
            dirty(app, case)
            saved(app, source, case, RAW)
            wait_screen(app, "Saved", "Formatting skipped: 1500 ms")
            acknowledged(app, case, "cancel")
            app.paste("LATEST ")
            latest = PREFIX + b"LATEST " + ORIGINAL
            saved(app, source, case, latest)
            assert sum(event["kind"] == "format-entered" for event in records(case)) == 1
            (case / "release-format").touch()
            wait_screen(app, "late formatter settled")
            wait_screen(app, "int value=1;", "LATEST", absent=("int value = 1;",))
            assert source.read_bytes() == latest
            saved(app, source, case, latest)
            assert sum(event["kind"] == "format-entered" for event in records(case)) == 2
        print("PASS: deadline saves raw bytes, occupied callback blocks overlap, and exact late settlement cannot edit newer text before safe capacity reuse")

        with editor(root, "d", autosave=True) as (app, source, case):
            dirty(app, case, requires_dirty=False)
            persisted(app, source, RAW)
            acknowledged(app, case, "save", lambda event: event["text"].encode() == RAW)
            assert not any(event["kind"] == "format-entered" for event in records(case))
        print("PASS: actual after-delay save commits exact bytes with zero formatter callbacks")

        with editor(root, "e", native_server=False) as (app, source, case):
            dirty(app, case, native_server=False)
            app.send(CTRL_S)
            persisted(app, source, RAW)
            wait_screen(app, "Saved", "Formatting skipped: no native language server")
            assert not (case / "protocol.jsonl").exists()
        print("PASS: no-LSP/no-Node editing and save remain usable, with honest formatting fallback and restored terminal modes")


if __name__ == "__main__":
    run()
