#!/usr/bin/env python3
"""Framed signature peer with captured-context replies and explicit release gate."""
import json
from pathlib import Path
import sys
import threading
import time

log = Path(sys.argv[1])
gate = Path(sys.argv[2])
hold_first = '--hold-first' in sys.argv
lock = threading.Lock()
count = 0
pending = 0
max_pending = 0
documents = {}


def record(value):
    with lock:
        with log.open('a', encoding='utf-8') as target:
            target.write(json.dumps(value) + '\n')


def send(value):
    body = json.dumps({'jsonrpc': '2.0', **value}).encode()
    with lock:
        sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode() + body)
        sys.stdout.buffer.flush()


def reply(ident, number, params, text):
    global pending
    if number == 1 and hold_first:
        deadline = time.monotonic() + 10
        while not gate.exists():
            if time.monotonic() > deadline:
                send({'id': ident, 'error': {'code': -32603, 'message': 'fixture gate timed out'}})
                record({'event': 'gate-timeout', 'id': ident})
                with lock:
                    pending -= 1
                return
            time.sleep(0.002)
    position = params['position']
    line = text.splitlines()[position['line']]
    prefix = line.encode('utf-16-le')[:position['character'] * 2].decode('utf-16-le')
    parameter = 1 if ',' in prefix else 0
    result = {'activeSignature': 0, 'activeParameter': parameter, 'signatures': [
        {'label': f'sum{number}(int left, int right)', 'documentation': 'integer overload', 'parameters': [{'label': 'int left'}, {'label': 'int right', 'documentation': 'right argument'}]},
        {'label': f'sum{number}(猫🙂 left, double right)', 'documentation': 'Unicode overload', 'parameters': [{'label': [4 + len(str(number)), 12 + len(str(number))]}, {'label': 'double right'}]},
    ]}
    # Decrement before the wire reply: a subsequent request is admitted only
    # after this matching reply arrives, proving actual-work occupancy <= 1.
    with lock:
        pending -= 1
    record({'event': 'response', 'id': ident, 'number': number})
    send({'id': ident, 'result': result})


while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            raise SystemExit
        if line == b'\r\n':
            break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method, ident, params = message.get('method'), message.get('id'), message.get('params', {})
    if method == 'initialize':
        send({'id': ident, 'result': {'capabilities': {'textDocumentSync': 1, 'signatureHelpProvider': {'triggerCharacters': ['(', ','], 'retriggerCharacters': [')', ';']}}}})
    elif method == 'textDocument/didOpen':
        documents[params['textDocument']['uri']] = params['textDocument']['text']
    elif method == 'textDocument/didChange':
        documents[params['textDocument']['uri']] = params['contentChanges'][-1]['text']
    elif method == 'textDocument/signatureHelp':
        count += 1
        with lock:
            pending += 1
            max_pending = max(max_pending, pending)
        record({'event': 'request', 'id': ident, 'number': count, 'params': params, 'pending': pending, 'maxPending': max_pending})
        text = documents[params['textDocument']['uri']]
        threading.Thread(target=reply, args=(ident, count, params, text), daemon=True).start()
    elif method == '$/cancelRequest':
        record({'event': 'cancel', 'id': params['id']})
    elif method == 'shutdown':
        send({'id': ident, 'result': None})
    elif method == 'exit':
        break
