"""Controllable actual LSP peer for debouncing and stale completion integrity."""
import json
from pathlib import Path
import sys
import threading
import time

root = Path(sys.argv[1])
documents = {}
lock = threading.Lock()


def send(value):
    data = json.dumps({'jsonrpc': '2.0', **value}).encode()
    with lock:
        sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode() + data)
        sys.stdout.buffer.flush()


def complete(ident, params):
    # A held response keeps its original coordinates even after didChange.
    position = params['position']
    while (root / 'hold').exists():
        time.sleep(.005)
    items = [{'label': label, 'textEdit': {
        'range': {'start': {'line': position['line'], 'character': 0}, 'end': position},
        'newText': label,
    }} for label in ['answer', 'another']]
    if (root / 'overlap').exists():
        for item in items:
            item['additionalTextEdits'] = [{'range': {
                'start': {'line': 0, 'character': 0}, 'end': {'line': 0, 'character': 1},
            }, 'newText': 'invalid'}]
    send({'id': ident, 'result': items})


while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b'\r\n':
            break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method, params, ident = message.get('method'), message.get('params', {}), message.get('id')
    if method == 'initialize':
        send({'id': ident, 'result': {'capabilities': {
            'positionEncoding': 'utf-16', 'textDocumentSync': 1,
            'completionProvider': {'triggerCharacters': ['.']},
        }}})
    elif method == 'textDocument/completion':
        with (root / 'requests.jsonl').open('a') as journal:
            journal.write(json.dumps(params) + '\n')
        threading.Thread(target=complete, args=(ident, params), daemon=True).start()
    elif method in ('textDocument/didOpen', 'textDocument/didChange'):
        documents[params['textDocument']['uri']] = params.get('text', params.get('contentChanges'))
    elif method == 'shutdown':
        send({'id': ident, 'result': None})
    elif method == 'exit':
        break
