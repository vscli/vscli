#!/usr/bin/env python3
"""Deterministic delayed signature help, including canceled out-of-order replies."""
import json
import sys
import threading
import time
lock = threading.Lock()
count = 0

def send(message):
    body = json.dumps({"jsonrpc":"2.0", **message}).encode()
    with lock:
        sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        sys.stdout.buffer.flush()

def reply(ident, number):
    time.sleep(0.18 if number == 1 else 0.04)
    label = f"sum{number}(double left, int count)"
    result = {"activeSignature":0,"activeParameter":1,"signatures":[{
        "label":label,"documentation":"Adds two values.","parameters":[
            {"label":"double left"},{"label":"int count","documentation":"Number of values."}]}]}
    if "--invalid" in sys.argv:
        result["signatures"][0]["parameters"][1]["label"] = [0, 999]
    send({"id":ident,"result":result})

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n": break
        key, value = line.decode().split(":",1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    method, ident = message.get("method"), message.get("id")
    if method == "initialize":
        send({"id":ident,"result":{"capabilities":{"textDocumentSync":1,"signatureHelpProvider":{"triggerCharacters":["(",","]}}}})
    elif method == "textDocument/signatureHelp":
        assert message["params"]["context"] == {"triggerKind":1,"isRetrigger":False}
        count += 1
        threading.Thread(target=reply,args=(ident,count),daemon=True).start()
    elif method == "shutdown": send({"id":ident,"result":None})
    elif method == "exit": break
