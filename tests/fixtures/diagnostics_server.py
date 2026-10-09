#!/usr/bin/env python3
"""Controllable native diagnostic publication peer; versions are never inferred."""
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


def publish(document, options=None):
    options = options or {}
    version = document['version']
    while (root / f'hold-{version}').exists():
        time.sleep(.005)
    params = {
        'uri': options.get('uri', document['uri']),
        'diagnostics': [] if options.get('empty') else [{
            'range': {'start': {'line': 0, 'character': 0},
                      'end': {'line': 0, 'character': 1}},
            'message': f'version-{version}', 'severity': 2,
            'code': 'fixture', 'data': {'version': version},
        }],
    }
    if not options.get('unversioned'):
        params['version'] = options.get('version', version)
    if options.get('message_bytes'):
        params['diagnostics'][0]['message'] = 'é' * (options['message_bytes'] // 2)
    if options.get('data_bytes'):
        params['diagnostics'][0]['data']['payload'] = 'x' * options['data_bytes']
    if options.get('count'):
        params['diagnostics'] *= options['count']
    if options.get('reversed'):
        params['diagnostics'][0]['range']['start'] = {'line': 1, 'character': 0}
    if 'coordinate' in options:
        params['diagnostics'][0]['range']['end']['line'] = options['coordinate']
    if 'severity' in options:
        params['diagnostics'][0]['severity'] = options['severity']
    if options.get('negative'):
        params['diagnostics'][0]['range']['start']['line'] = -1
    if options.get('malformed_diagnostics'):
        params['diagnostics'] = {'invalid': True}
    if options.get('malformed_item'):
        params['diagnostics'] = [{'message': 'missing range'}]
    send({'method': 'textDocument/publishDiagnostics', 'params': params})
    send({'method': 'window/showMessage', 'params': {
        'type': 3, 'message': f'published-{version}'}})


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
            'positionEncoding': 'utf-16', 'textDocumentSync': 1, 'codeActionProvider': True,
        }}})
    elif method in ('textDocument/didOpen', 'textDocument/didChange'):
        document = dict(params['textDocument'])
        documents[document['uri']] = document
        with (root / 'sync.jsonl').open('a') as journal:
            journal.write(json.dumps({'method': method, **document}) + '\n')
        threading.Thread(target=publish, args=(document,), daemon=True).start()
    elif method == 'textDocument/didClose':
        documents.pop(params['textDocument']['uri'], None)
    elif method == 'fixture/publish':
        publish(documents[params['textDocument']['uri']], params.get('publication'))
        send({'id': ident, 'result': None})
    elif method == 'textDocument/codeAction':
        send({'id': ident, 'result': []})
    elif method == 'shutdown':
        send({'id': ident, 'result': None})
    elif method == 'exit':
        break
    elif ident is not None:
        send({'id': ident, 'error': {'code': -32601, 'message': 'unsupported fixture method'}})
