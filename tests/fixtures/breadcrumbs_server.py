#!/usr/bin/env python3
"""Named synthetic hierarchical document-symbol peer; advisory cancel stays observable."""
import argparse
import json
from pathlib import Path
import sys
import threading
import time

arguments = argparse.ArgumentParser()
arguments.add_argument('log', type=Path)
arguments.add_argument('gate', type=Path)
arguments.add_argument('--hold-first', action='store_true')
arguments.add_argument('--flat', action='store_true')
options = arguments.parse_args()
cases = json.loads((Path(__file__).resolve().parents[1] / 'vscode-reference' / 'breadcrumbs-cases.json').read_text())
fixture = next(case for case in cases if case['shape'] == ('flat' if options.flat else 'hierarchical'))
lock = threading.RLock()
documents = {}
ordinal = 0
pending = 0


def record(event, **values):
    with lock, options.log.open('a', encoding='utf-8') as output:
        output.write(json.dumps({'event': event, **values}, ensure_ascii=False) + '\n')


def send(value):
    data = json.dumps({'jsonrpc': '2.0', **value}, ensure_ascii=False).encode()
    with lock:
        sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode() + data)
        sys.stdout.buffer.flush()


def respond(ident, params, version, index):
    global pending
    if options.hold_first and index == 1:
        deadline = time.monotonic() + 30
        while not options.gate.exists():
            if time.monotonic() >= deadline:
                record('gate-timeout', id=ident)
                with lock:
                    pending -= 1
                send({'id': ident, 'error': {'code': -32000, 'message': 'Named fixture gate expired'}})
                return
            time.sleep(.005)
    symbols = json.loads(json.dumps(fixture['symbols']))
    if options.flat:
        for symbol in symbols:
            symbol['location']['uri'] = params['textDocument']['uri']
    with lock:
        pending -= 1
        record('response', id=ident, ordinal=index, version=version, pending=pending)
        send({'id': ident, 'result': symbols})


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
    method, ident, params = message.get('method'), message.get('id'), message.get('params', {})
    if method == 'initialize':
        record('initialize')
        send({'id': ident, 'result': {'capabilities': {'textDocumentSync': 1, 'documentSymbolProvider': True}}})
    elif method in ('textDocument/didOpen', 'textDocument/didChange'):
        document = params['textDocument']
        text = document.get('text')
        if text is None:
            text = params['contentChanges'][-1]['text']
        documents[document['uri']] = {'version': document['version'], 'text': text}
        record('sync', version=document['version'])
    elif method == 'textDocument/documentSymbol':
        with lock:
            ordinal += 1
            pending += 1
            index = ordinal
            version = documents[params['textDocument']['uri']]['version']
            record('request', id=ident, ordinal=index, version=version, pending=pending)
        threading.Thread(target=respond, args=(ident, params, version, index), daemon=True).start()
    elif method == '$/cancelRequest':
        record('cancel', id=params['id'], pending=pending)
    elif method == 'shutdown':
        send({'id': ident, 'result': None})
    elif method == 'exit':
        break
    elif ident is not None:
        send({'id': ident, 'result': None})
