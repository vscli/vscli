#!/usr/bin/env python3
"""Deterministic stdio LSP peer for protocol and stale-response tests."""
import json
import sys
import threading
import time

documents = {}
lock = threading.Lock()

def send(message):
    data = json.dumps({"jsonrpc": "2.0", **message}).encode()
    with lock:
        sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        sys.stdout.buffer.flush()

def reply(ident, result, delay=0):
    def run():
        time.sleep(delay)
        send({"id": ident, "result": result})
    threading.Thread(target=run, daemon=True).start()

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    method = message.get("method")
    params = message.get("params", {})
    ident = message.get("id")
    if method == "initialize":
        reply(ident, {"capabilities": {"positionEncoding": "utf-16", "textDocumentSync": 1, "hoverProvider": True, "completionProvider": {}, "definitionProvider": True, "referencesProvider": True, "documentFormattingProvider": True, "renameProvider": True}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        doc = params["textDocument"]
        documents[doc["uri"]] = doc.get("text", params.get("contentChanges", [{}])[0].get("text", ""))
        send({"method": "textDocument/publishDiagnostics", "params": {"uri": doc["uri"], "version": doc["version"], "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "severity": 2, "message": "Fixture diagnostic"}]}})
    elif method == "textDocument/hover":
        reply(ident, {"contents": {"kind": "plaintext", "value": "Fixture hover\nNative LSP works"}})
    elif method == "textDocument/completion":
        reply(ident, [{"label": "answer", "insertText": "answer"}])
    elif method == "textDocument/formatting":
        text = documents[params["textDocument"]["uri"]]
        first = text.splitlines()[0] if text else ""
        reply(ident, [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": len(first.encode('utf-16-le')) // 2}}, "newText": "formatted"}], 0.25)
    elif method in ("textDocument/definition", "textDocument/references"):
        reply(ident, [{"uri": params["textDocument"]["uri"], "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}}])
    elif method == "textDocument/rename":
        reply(ident, {"changes": {params["textDocument"]["uri"]: [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": params["newName"]}]}})
    elif method == "shutdown":
        reply(ident, None)
    elif method == "exit":
        break
    elif ident is not None:
        send({"id": ident, "error": {"code": -32601, "message": "unsupported fixture method"}})
