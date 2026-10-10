#!/usr/bin/env python3
"""Automatic hints, overload shortcuts and completion coexistence through a PTY."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, wait, save
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, eventually, wait_screen


def records(path):
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def native(root):
    source = root / 'input.cpp'
    original = 'sum\r\n// 猫🙂\r\n'
    source.write_bytes(original.encode())
    log = root / 'requests.jsonl'
    peer = Path(__file__).resolve().parent / 'fixtures' / 'signature_hints_server.py'
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer,
                 '--lsp-arg', log, '--lsp-arg', root / 'release', '--lsp-language', 'cpp', source,
                 enhanced=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x1b[C' * 3 + b'(')
    wait_screen(app, 'Parameter Hints 1/2', 'sum1(int left, int right)')
    requests = [row for row in records(log) if row['event'] == 'request']
    assert len(requests) == 1
    assert requests[0]['params']['context'] == {'triggerKind': 2, 'triggerCharacter': '(', 'isRetrigger': False}
    assert requests[0]['params']['position']['character'] == 4
    assert source.read_bytes() == original.encode(), 'Typing must remain unsaved'
    app.send(b'\x1b[B')
    wait_screen(app, 'Parameter Hints 2/2', 'Unicode overload')
    app.send(b'\x1b[1;3A')
    wait_screen(app, 'Parameter Hints 1/2', 'integer overload')
    app.send('1,')
    wait_screen(app, 'Parameter Hints 1/2', 'sum2(int left, int right)', 'right argument')
    requests = [row for row in records(log) if row['event'] == 'request']
    assert len(requests) == 2, 'Overload navigation must be local'
    assert all(row['maxPending'] == 1 for row in requests)
    context = requests[1]['params']['context']
    assert context['triggerKind'] == 2 and context['triggerCharacter'] == ',' and context['isRetrigger']
    assert context['activeSignatureHelp']['activeSignature'] == 0
    save(app, source, 'sum(1,)\r\n// 猫🙂\r\n')
    wait(app, 'Parameter Hints 1/2')
    app.send(b'\x1b')
    eventually(lambda: app.read() and 'Parameter Hints' not in app.screen.text())
    app.send(CTRL_Z)
    save(app, source, 'sum()\r\n// 猫🙂\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: automatic native hints, original overload shortcuts, comma context, local cycling, CRLF/Unicode save/Undo with no Node')


def extension(root):
    root.mkdir()
    source = root / 'input.cpp'
    original = 'sum\r\n// 猫🙂\r\n'
    source.write_bytes(original.encode())
    fixture = Path(__file__).resolve().parent / 'fixtures' / 'signature-hints-extension'
    log = root / 'extension-signatures.jsonl'
    app = Editor(root, '--extension', fixture, source, enhanced=True)
    wait(app, 'Extension ready:')
    app.send(b'\x1b[C' * 3 + b'(')
    wait_screen(app, 'Parameter Hints 1/2', 'extension1(int left, int right)')
    assert len(records(log)) == 1
    app.send(b'\x1b[32;5u')  # original Ctrl+Space
    wait_screen(app, 'Suggestions · Tab accepts', 'choiceOne', 'choiceTwo', 'Parameter Hints 1/2')
    app.send(b'\x1b[32;6u')  # original Ctrl+Shift+Space retains suggestions
    wait_screen(app, 'Suggestions · Tab accepts', 'Parameter Hints 1/2', 'extension2(int left, int right)')
    repeated = records(log)
    assert len(repeated) == 2
    assert repeated[1]['triggerKind'] == 1 and repeated[1]['isRetrigger'] and repeated[1]['originalObject']
    app.send(b'\x1b[B')
    wait_screen(app, 'Suggestions · Tab accepts', 'Parameter Hints 1/2')
    app.send(b'\x1b[1;3B')  # original Alt+Down: hints even while suggestions are open
    wait_screen(app, 'Suggestions · Tab accepts', 'Parameter Hints 2/2', 'Unicode overload')
    assert len(records(log)) == 2, 'Both widget navigation commands must avoid extra signature callbacks'
    app.send(b'\x1b')  # completion Escape wins and keeps parameter hints
    wait_screen(app, 'Parameter Hints 2/2', absent=('Suggestions ·',))
    app.send(',')
    wait_screen(app, 'Parameter Hints 1/2', 'extension3(int left, int right)', 'right argument')
    trace = records(log)
    assert len(trace) == 3
    assert trace[0]['triggerKind'] == 2 and trace[0]['triggerCharacter'] == '(' and not trace[0]['isRetrigger']
    assert trace[2]['triggerKind'] == 2 and trace[2]['triggerCharacter'] == ',' and trace[2]['isRetrigger']
    assert trace[2]['originalObject'] and trace[2]['activeSignature'] == 1
    assert source.read_bytes() == original.encode()
    save(app, source, 'sum(,)\r\n// 猫🙂\r\n')
    app.send(b'\x1b')
    eventually(lambda: app.read() and 'Parameter Hints' not in app.screen.text())
    app.send(CTRL_Z)
    save(app, source, 'sum()\r\n// 猫🙂\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: optional signature original-object retrigger, independent completion/hint shortcuts, bounded callbacks, unsaved CRLF/Unicode Undo')


def real(root):
    source = root / 'input.cpp'
    original = 'int sum(int left, int right);\r\nint main(){ return ; }\r\n// 猫🙂\r\n'
    source.write_bytes(original.encode())
    (root / 'compile_flags.txt').write_text('-std=c++17\n')
    settings = root / 'user.json'
    settings.write_text(json.dumps({'[cpp]': {'vscli.languageServer.program': shutil.which('clangd')}}))
    app = Editor(root, '--settings', settings, source, enhanced=True, auto_lsp=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x07')
    wait(app, 'Go to Line')
    app.send('2:20\r')
    app.send('sum(')
    wait_screen(app, 'Parameter Hints', 'sum', 'int left', 'int right')
    save(app, source, original.replace('return ;', 'return sum();'))
    wait_screen(app, 'Parameter Hints', 'sum')
    app.send('1,')
    wait_screen(app, 'Parameter Hints', 'int right')
    edited = original.replace('return ;', 'return sum(1,);')
    save(app, source, edited)
    app.send(b'\x1b')
    app.send(CTRL_Z)
    save(app, source, original.replace('return ;', 'return sum();'))
    app.finish()
    print('PASS: actual installed clangd automatic hints with no CLI LSP or executable Node, native pairs, CRLF/Unicode save and Undo')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-signature-pty-') as directory:
        try:
            root = Path(directory)
            if '--real-clangd' in sys.argv:
                real(root)
            else:
                native(root)
                extension(root / 'extension')
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in list(LIVE):
                try:
                    stop_surface_process(app)
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
