#!/usr/bin/env python3
"""Deterministic DAP peer: reversed variable replies, rejected step, disconnect."""
import json
import pathlib
import sys

sequence = 0
program = None
old_variables = None
line_number = 2

def send(message):
    global sequence
    sequence += 1
    data = json.dumps({"seq": sequence, **message}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()

def reply(request, body=None, success=True):
    send({"type": "response", "request_seq": request["seq"],
          "command": request["command"], "success": success,
          "body": body or {}, "message": "fixture rejection" if not success else ""})

def event(name, body=None):
    send({"type": "event", "event": name, "body": body or {}})

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
    request = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    command = request["command"]
    args = request.get("arguments", {})
    if command == "initialize":
        reply(request, {"supportsConfigurationDoneRequest": True})
    elif command == "launch":
        program = args["program"]
        launch = request
        event("initialized")
    elif command == "setBreakpoints":
        reply(request, {"breakpoints": [{**b, "verified": True} for b in args["breakpoints"]]})
    elif command == "setExceptionBreakpoints":
        reply(request)
    elif command == "configurationDone":
        reply(request)
        reply(launch)
        event("stopped", {"threadId": 1, "reason": "breakpoint"})
    elif command == "stackTrace":
        reply(request, {"stackFrames": [{"id": 1, "name": "main", "line": line_number,
              "column": 1, "source": {"path": program}}]})
    elif command == "scopes":
        reply(request, {"scopes": [{"name": "Locals", "variablesReference": 10}]})
    elif command == "variables":
        reference = args["variablesReference"]
        body = {"variables": [{"name": f"value{reference}", "value": "42", "variablesReference": 0}]}
        if reference == 11:
            old_variables = (request, body)
        else:
            reply(request, body)
            if old_variables:
                reply(*old_variables)
                old_variables = None
    elif command == "stepIn":
        reply(request, success=False)
    elif command in ("next", "stepOut"):
        line_number += 1
        reply(request)
        event("continued", {"threadId": 1})
        event("stopped", {"threadId": 1, "reason": "step"})
    elif command == "continue":
        reply(request)
        event("output", {"output": "fixture completed\n"})
        event("terminated")
    elif command == "evaluate":
        reply(request, {"result": "42", "variablesReference": 0})
    elif command == "disconnect":
        if program:
            pathlib.Path(program + ".disconnected").write_text("clean shutdown")
        reply(request)
        sys.exit(0)
    else:
        reply(request, success=False)
