#!/usr/bin/env python3
"""Native-only terminal regression for a latest intent behind canceled actual work."""
import json
from pathlib import Path
import sys
import tempfile
import time

from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_S, CTRL_Z, Editor, eventually, wait_screen

ORIGINAL = 'left right\r\n// 猫🙂\r\n'
EXPECTED = 'left RIGHT 猫🙂\r\nnext\r\n// 猫🙂\r\n'
QUICK_FIX = b'\x1b[46;5u'  # Original Linux Ctrl+. through enhanced delivery.
ESCAPE = b'\x1b[27u'
HOME, RIGHT, SHIFT_RIGHT = b'\x1b[H', b'\x1b[C', b'\x1b[1;2C'

# The peer deliberately ignores advisory cancellation. Its first actual callback
# remains occupied until this test releases a filesystem gate; the intermediate
# queued caret request must never reach it, and the final selected range must.
PEER = r'''
import json
from pathlib import Path
import sys
import threading
import time

root = Path(sys.argv[1])
lock = threading.RLock()
documents = {}
pending = 0
requests = 0

def record(kind, **values):
    with lock, (root / 'peer.jsonl').open('a', encoding='utf-8') as output:
        output.write(json.dumps({'kind': kind, **values}) + '\n')

def send(message):
    data = json.dumps({'jsonrpc': '2.0', **message}).encode()
    with lock:
        sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode() + data)
        sys.stdout.buffer.flush()

def action(ident, params, version, ordinal):
    global pending
    if ordinal == 1:
        deadline = time.monotonic() + 10
        while not (root / 'release-first').exists():
            if time.monotonic() >= deadline:
                record('gate-timeout', id=ident)
                with lock:
                    pending -= 1
                send({'id': ident, 'error': {'code': -32000, 'message': 'Held peer gate expired'}})
                return
            time.sleep(.005)
    title = 'Canceled earlier action' if ordinal == 1 else 'Latest queued fix'
    replacement = 'STALE' if ordinal == 1 else 'RIGHT 猫🙂\nnext'
    row = {'title': title, 'kind': 'quickfix', 'edit': {'documentChanges': [{
        'textDocument': {'uri': params['textDocument']['uri'], 'version': version},
        'edits': [{'range': params['range'], 'newText': replacement}],
    }]}}
    with lock:
        pending -= 1
        record('response', id=ident, ordinal=ordinal, pending=pending)
        send({'id': ident, 'result': [row]})

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b'\r\n':
            break
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method, ident, params = message.get('method'), message.get('id'), message.get('params', {})
    if method == 'initialize':
        record('initialize')
        send({'id': ident, 'result': {'capabilities': {'textDocumentSync': 1, 'codeActionProvider': True}}})
    elif method in ('textDocument/didOpen', 'textDocument/didChange'):
        document = params['textDocument']
        documents[document['uri']] = document['version']
    elif method == 'textDocument/codeAction':
        with lock:
            requests += 1
            pending += 1
            ordinal = requests
            record('request', id=ident, ordinal=ordinal, pending=pending, range=params['range'])
        threading.Thread(target=action, args=(ident, params, documents[params['textDocument']['uri']], ordinal), daemon=True).start()
    elif method == '$/cancelRequest':
        record('cancel', id=params['id'], pending=pending)
    elif method == 'shutdown':
        send({'id': ident, 'result': None})
    elif method == 'exit':
        break
'''


def trace(root):
    file = root / 'peer.jsonl'
    if not file.exists():
        return []
    # A partial final line is a concurrent append, not an observation yet.
    return [json.loads(line) for line in file.read_text().splitlines(keepends=True)
            if line.endswith('\n')]


def requests(root):
    return [entry for entry in trace(root) if entry['kind'] == 'request']


def save(app, source, expected):
    app.send(CTRL_S)
    eventually(lambda: app.read() and source.read_bytes() == expected.encode())


def run(root):
    source, peer = root / 'main.cpp', root / 'held_peer.py'
    source.write_bytes(ORIGINAL.encode())
    peer.write_text(PEER)
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer,
                 '--lsp-arg', root, '--lsp-language', 'cpp', '--no-session',
                 source, enhanced=True, extra_env={'PATH': ''})
    try:
        wait_screen(app, 'Language server ready', 'main.cpp', timeout=8)
        app.send(HOME + SHIFT_RIGHT * 4 + QUICK_FIX)
        eventually(lambda: app.read() and len(requests(root)) == 1, timeout=8)
        first = requests(root)[0]
        assert first['range'] == {'start': {'line': 0, 'character': 0},
                                 'end': {'line': 0, 'character': 4}}, first
        app.send(ESCAPE)
        eventually(lambda: app.read() and any(entry['kind'] == 'cancel'
                                             and entry['id'] == first['id'] for entry in trace(root)), timeout=8)
        # Thirty intermediate invocations use different selections; with the
        # first and final request this exercises 32 user intents behind one
        # actual occupied callback. None may reach the peer before its gate.
        app.send(b''.join(HOME + RIGHT * (index % 10) + SHIFT_RIGHT + QUICK_FIX + ESCAPE
                          for index in range(30)))
        # Each consumed cancellation targets the same held actual request. This
        # barrier prevents the filesystem gate from overtaking batched input.
        eventually(lambda: app.read() and sum(entry['kind'] == 'cancel'
                    and entry['id'] == first['id'] for entry in trace(root)) >= 31, timeout=8)
        assert len(requests(root)) == 1, 'Intermediate intent overlapped actual held work'
        app.send(HOME + RIGHT * 5 + SHIFT_RIGHT * 5 + QUICK_FIX)
        deadline = time.monotonic() + .25
        while time.monotonic() < deadline:
            app.read()
            assert len(requests(root)) == 1, 'Queued intents admitted overlapping actual peer work'
            assert 'Code Actions ·' not in app.screen.text(), 'Canceled held work opened a picker'
            assert source.read_bytes() == ORIGINAL.encode(), 'Request/cancellation saved native text'
            time.sleep(.005)
        (root / 'release-first').touch()
        wait_screen(app, 'Code Actions', 'Latest queued fix', absent=('Canceled earlier action',), timeout=8)
        observed = requests(root)
        assert len(observed) == 2, f'Latest intent did not replace the intermediate queue: {trace(root)}'
        assert observed[1]['range'] == {'start': {'line': 0, 'character': 5},
                                        'end': {'line': 0, 'character': 10}}, observed[1]
        assert max(entry['pending'] for entry in observed) == 1, 'More than one actual callback occupied the peer'
        assert not any(entry['kind'] == 'gate-timeout' for entry in trace(root))
        app.send(b'\r')
        assert source.read_bytes() == ORIGINAL.encode(), 'Accepting an action saved without explicit Save'
        save(app, source, EXPECTED)
        app.send(CTRL_Z)
        save(app, source, ORIGINAL)
        assert len(requests(root)) == 2
        app.finish()
        assert app.process.restored, 'Terminal mode leaked after native-only action workflow'
        print('PASS: empty-PATH native-only Ctrl+. coalesces 32 user intents behind one actual callback, ignores canceled earlier response, applies latest selection and preserves Unicode/CRLF save/Undo and terminal restoration')
    except BaseException:
        stop_surface_process(app)
        assert app.process.restored, 'Terminal mode leaked after regression failure'
        app.close_fds()
        raise


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-code-actions-queue-pty-') as directory:
        run(Path(directory))
