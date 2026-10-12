#!/usr/bin/env python3
"""Actual framed LSP rename capability/prepare modes; no JavaScript runtime."""
import json
from pathlib import Path
import sys

mode, transcript = sys.argv[1], Path(sys.argv[2])
RANGE = {'start': {'line': 0, 'character': 10}, 'end': {'line': 0, 'character': 16}}
records = 0


def send(value):
    data = json.dumps(value, ensure_ascii=False).encode()
    sys.stdout.buffer.write(b'Content-Length: ' + str(len(data)).encode() + b'\r\n\r\n' + data)
    sys.stdout.buffer.flush()


def reply(ident, result):
    send({'jsonrpc': '2.0', 'id': ident, 'result': result})


while True:
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line in (b'\r\n', b'\n'):
            break
        if line.lower().startswith(b'content-length:'):
            length = int(line.split(b':', 1)[1])
    if length is None:
        continue
    assert 0 <= length <= 4 * 1024 * 1024
    raw = sys.stdin.buffer.read(length)
    if len(raw) != length:
        sys.exit(1)
    message = json.loads(raw)
    method, ident, params = message.get('method'), message.get('id'), message.get('params', {})
    records += 1
    assert records <= 128, 'Bounded native rename fixture transcript'
    with transcript.open('a', encoding='utf-8') as stream:
        stream.write(json.dumps({'method': method, 'params': params}, ensure_ascii=False) + '\n')
    if method == 'initialize':
        capability = params['capabilities']['textDocument']['rename']
        assert capability['prepareSupport'] is True
        assert capability['prepareSupportDefaultBehavior'] == 1
        reply(ident, {'capabilities': {'positionEncoding': 'utf-16', 'textDocumentSync': 1,
                                     'renameProvider': True if mode == 'no-prepare' else {'prepareProvider': True}}})
    elif method == 'textDocument/prepareRename':
        assert mode != 'no-prepare', 'Unadvertised preparation must not be called'
        if mode == 'range':
            reply(ident, RANGE)
        elif mode == 'placeholder':
            reply(ident, {'range': RANGE, 'placeholder': 'Original source label'})
        elif mode == 'default':
            reply(ident, {'defaultBehavior': True})
        elif mode == 'null':
            reply(ident, None)
        elif mode == 'error':
            send({'jsonrpc': '2.0', 'id': ident, 'error': {'code': -32602, 'message': 'rename rejected by fixture'}})
        else:
            raise AssertionError(mode)
    elif method == 'textDocument/rename':
        assert mode not in ('null', 'error')
        reply(ident, {'changes': {params['textDocument']['uri']: [{'range': RANGE, 'newText': params['newName']}]}})
    elif method == 'shutdown':
        reply(ident, None)
    elif method == 'exit':
        break
    elif ident is not None:
        send({'jsonrpc': '2.0', 'id': ident, 'error': {'code': -32601, 'message': 'Unsupported rename fixture method'}})
