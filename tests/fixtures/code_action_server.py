#!/usr/bin/env python3
"""Deterministic actions/resolve/executeCommand/applyEdit protocol peer."""
import json
import sys
import threading
import time

lock = threading.Lock()
documents = {}
commands = {}
completed_commands = {}
unsolicited_sent = False

def send(message):
    data = json.dumps({"jsonrpc": "2.0", **message}).encode()
    with lock:
        sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        sys.stdout.buffer.flush()

def delayed(message, delay=0.04):
    def run():
        time.sleep(delay)
        send(message)
    threading.Thread(target=run, daemon=True).start()

def edit(uri, version, span, text):
    return {"documentChanges": [{"textDocument": {"uri": uri, "version": version}, "edits": [{"range": span, "newText": text}]}]}

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
    method, ident, params = message.get("method"), message.get("id"), message.get("params", {})
    if method == "initialize":
        send({"id": ident, "result": {"capabilities": {"textDocumentSync": 1, "codeActionProvider": {"resolveProvider": True}, "executeCommandProvider": {"commands": ["fixture.action"]}}}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        doc = params["textDocument"]
        documents[doc["uri"]] = {"version": doc["version"], "text": doc.get("text", params.get("contentChanges", [{}])[0].get("text", ""))}
        send({"method": "textDocument/publishDiagnostics", "params": {"uri": doc["uri"], "version": doc["version"], "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}, "message": "Fix fixture", "code": "fixture-code", "data": {"token": 42}}]}})
        if "--unsolicited" in sys.argv and not unsolicited_sent:
            unsolicited_sent = True
            commands["unsolicited"] = None
            delayed({"id": "unsolicited", "method": "workspace/applyEdit", "params": {"edit": edit(doc["uri"], doc["version"], {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}, "uninvited")}}, 0.1)
    elif method == "textDocument/didClose":
        documents.pop(params["textDocument"]["uri"], None)
    elif method == "textDocument/codeAction":
        uri = params["textDocument"]["uri"]
        version, span = documents[uri]["version"], params["range"]
        data = {"uri": uri, "version": version, "span": span}
        valid = edit(uri, version, span, "fixed")
        invalid = json.loads(json.dumps(valid))
        invalid["documentChanges"].append({"textDocument": {"uri": uri.rsplit("/", 1)[0] + "/other.rs", "version": None}, "edits": [{"range": {"start": {"line": 999, "character": 0}, "end": {"line": 999, "character": 1}}, "newText": "bad"}]})
        resource = json.loads(json.dumps(valid))
        resource["documentChanges"].append({"kind": "delete", "uri": uri})
        annotated = json.loads(json.dumps(valid))
        annotated["documentChanges"][0]["edits"][0]["annotationId"] = "confirmation"
        multi = {"changes": {u: [{"range": span, "newText": "both"}] for u in documents}}
        items = [
            {"title": "Fix selected text", "kind": "quickfix", "isPreferred": True, "edit": valid},
            {"title": "Resolve refactor", "kind": "refactor.rewrite", "data": data},
            {"title": "Execute command", "command": {"title": "Apply", "command": "fixture.action", "arguments": [data]}},
            {"title": "Disabled fix", "disabled": {"reason": "Fixture disabled"}, "edit": valid},
            {"title": "Atomic invalid target", "edit": invalid},
            {"title": "Stale version", "edit": edit(uri, version - 1, span, "bad")},
            {"title": "Resource operation", "edit": resource},
            {"title": "Annotated edit", "edit": annotated},
            {"title": "Multiple dirty buffers", "edit": multi},
            {"title": "Legacy command", "command": "fixture.action", "arguments": [data]},
            {"title": "Combined edit and command", "edit": valid, "command": {"command": "fixture.action", "arguments": [data]}},
            {"title": "Unknown edit form", "edit": {"changes": {uri: [{"range": span, "newText": "bad", "insertTextFormat": 2}]}}},
            {"title": "Overlapping edits", "edit": {"changes": {uri: [{"range": span, "newText": "bad"}, {"range": span, "newText": "also bad"}]}}},
        ]
        for variant in ("changes", "null", "old"):
            items.append({"title": f"Late completed command {variant}", "command": {"command": f"fixture.late.{variant}", "arguments": [data]}})
        if params["context"].get("only") == ["refactor"]:
            items = [items[1]]
        if params["context"].get("diagnostics") and params["context"]["diagnostics"][0].get("data") != {"token": 42}:
            send({"id": ident, "error": {"code": -32602, "message": "Diagnostic data was lost"}})
        else:
            delayed({"id": ident, "result": items})
    elif method == "codeAction/resolve":
        data = params["data"]
        delayed({"id": ident, "result": {**params, "edit": edit(data["uri"], data["version"], data["span"], "resolved")}}, 0.15)
    elif method == "workspace/executeCommand":
        data = params["arguments"][0]
        command = params["command"]
        if command.startswith("fixture.late.") and command not in completed_commands:
            completed_commands[command] = data
            send({"id": ident, "result": None})
            send({"method": "window/showMessage", "params": {"type": 3, "message": "Fixture first command completed"}})
            continue
        callback = f"apply-{ident}"
        commands[callback] = ident
        workspace_edit = edit(data["uri"], data["version"], data["span"], "commanded")
        if command.startswith("fixture.late."):
            old = completed_commands[command]
            workspace_edit = edit(old["uri"], old["version"], old["span"], "late")
            if command.endswith(".null"):
                workspace_edit["documentChanges"][0]["textDocument"]["version"] = None
            elif command.endswith(".changes"):
                workspace_edit = {"changes": {old["uri"]: workspace_edit["documentChanges"][0]["edits"]}}
        delayed({"id": callback, "method": "workspace/applyEdit", "params": {"edit": workspace_edit}}, 0.15)
    elif method is None and ident in commands:
        original = commands.pop(ident)
        if original is not None:
            send({"id": original, "result": None})
        send({"method": "window/showMessage", "params": {"type": 3, "message": "Fixture command applied" if message.get("result", {}).get("applied") else "Fixture command rejected"}})
    elif method == "shutdown":
        send({"id": ident, "result": None})
    elif method == "exit":
        break
