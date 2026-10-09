#!/usr/bin/env python3
"""Automatic caret suggestions through actual Unix input, with optional real clangd."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, wait, save
from pty_smoke import CTRL_S, CTRL_Z, eventually


def fixture(root):
    source = root / 'input.rs'
    original = ' 🙂\r\n'
    source.write_bytes(original.encode())
    peer = Path(__file__).resolve().parent / 'fixtures' / 'suggestions_server.py'
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send('ans')
    wait(app, 'Suggestions · Tab accepts')
    app.send(b'\t')
    save(app, source, 'answer 🙂\r\n')
    app.send(CTRL_Z)
    save(app, source, 'ans 🙂\r\n')
    app.send(b'\x1b[32;5u')
    wait(app, 'Suggestions · Tab accepts')
    app.send(b'\x1b')
    eventually(lambda: app.read() and 'Suggestions ·' not in app.screen.text())
    app.finish()
    print('PASS: automatic native popup and Tab, explicit reopen/Escape, CRLF/Unicode save/undo with no executable Node')

    source.write_bytes('an 🙂\r\n'.encode())
    (root / 'requests.jsonl').unlink()
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True)
    wait(app, 'Language server ready')
    app.send(b'\x1b[C' * 2 + b'\x1b[32;5u')
    wait(app, 'Suggestions · Tab accepts')
    (root / 'hold').touch()
    app.send('s')
    wait(app, 'Suggestions · updating')
    eventually(lambda: app.read() and len((root / 'requests.jsonl').read_text().splitlines()) == 2)
    app.send(b'\x1b[B\t')
    save(app, source, 'ans  🙂\r\n')
    (root / 'hold').unlink()
    app.send('猫')
    save(app, source, 'ans 猫 🙂\r\n')
    app.finish()
    print('PASS: continued typing filters pending popup; stale Tab indents, held reply cannot overwrite responsive native input')

    source.write_bytes(b'')
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True)
    wait(app, 'Language server ready')
    burst = 'xz' * 600
    # Preserve the existing four-second raw-input oracle with tooling ready.
    # Save belongs to this same burst; no later key may be needed to wake it.
    app.send(burst.encode() + CTRL_S)
    eventually(lambda: app.read() and source.read_bytes() == burst.encode())
    app.finish()
    print('PASS: ready completion source retains native 1200-key burst/save responsiveness with unchanged 4s oracle')
    resolution_fixture(root / 'resolution')


def resolution_fixture(root):
    root.mkdir()
    source = root / 'input.rs'
    original = '// 🙂\r\nans\r\n'
    source.write_bytes(original.encode())
    for marker in ('resolve', 'snippet', 'resolve_import', 'resolve_hold'):
        (root / marker).touch()
    peer = Path(__file__).resolve().parent / 'fixtures' / 'suggestions_server.py'
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x07')
    wait(app, 'Go to Line')
    app.send(b'2:4\r\x1b[32;5u')
    wait(app, 'Suggestions · Tab accepts')
    app.send(b'\t')
    wait(app, 'Resolving suggestion')
    eventually(lambda: app.read() and (root / 'resolves.jsonl').exists())
    app.send(b'\t' * 8)
    assert source.read_bytes() == original.encode()
    assert len((root / 'resolves.jsonl').read_text().splitlines()) == 1
    (root / 'resolve_hold').unlink()
    eventually(lambda: app.read() and 'answer(value)' in app.screen.text())
    app.send('x')
    save(app, source, 'use thing;\r\n// 🙂\r\nanswer(x)\r\n')
    app.send(CTRL_Z)
    save(app, source, 'use thing;\r\n// 🙂\r\nanswer(value)\r\n')
    app.send(CTRL_Z)
    save(app, source, original)
    app.finish()
    print('PASS: lazy native resolve waits without duplicate work; import+snippet commit preserves CRLF/Unicode and atomic Undo')

    for marker in ('snippet', 'resolve_import'):
        (root / marker).unlink()
    (root / 'resolves.jsonl').unlink()
    source.write_bytes('ans 🙂\r\n'.encode())
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x1b[C' * 3 + b'\x1b[32;5u')
    wait(app, 'Resolved docs')
    wait(app, 'Resolved native declaration')
    app.send(b'\t')
    save(app, source, 'answer 🙂\r\n')
    app.send(CTRL_Z)
    save(app, source, 'ans 🙂\r\n')
    app.finish()
    print('PASS: selected native completion resolves documentation in caret details UI without Node and retains plain completion Undo')

    (root / 'resolve_hold').touch()
    (root / 'resolves.jsonl').unlink()
    source.write_bytes('ans 🙂\r\n'.encode())
    app = Editor(root, '--lsp', sys.executable, '--lsp-arg', peer, '--lsp-arg', root,
                 source, enhanced=True, extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x1b[C' * 3 + b'\x1b[32;5u')
    wait(app, 'Suggestions · Tab accepts')
    app.send(b'\t')
    wait(app, 'Resolving suggestion')
    eventually(lambda: app.read() and (root / 'resolves.jsonl').exists())
    app.send(b'\x1b')
    eventually(lambda: app.read() and 'Suggestions ·' not in app.screen.text())
    app.send('猫')
    save(app, source, 'ans猫 🙂\r\n')
    (root / 'resolve_hold').unlink()
    app.send('z')
    save(app, source, 'ans猫z 🙂\r\n')
    app.finish()
    print('PASS: Escape cancels held native resolution; late import/primary edits cannot overwrite subsequent typing/save')


def real(root):
    source = root / 'input.cpp'
    original = 'int answer; int main(){ return ; }\r\n// 🙂\r\n'
    source.write_bytes(original.encode())
    (root / 'compile_flags.txt').write_text('-std=c++17\n')
    settings = root / 'user.json'
    settings.write_text(json.dumps({'[cpp]': {'vscli.languageServer.program': shutil.which('clangd')}}))
    app = Editor(root, '--settings', settings, source, enhanced=True, auto_lsp=True,
                 extra_env={'PATH': ''})
    wait(app, 'Language server ready')
    app.send(b'\x07')
    wait(app, 'Go to Line')
    app.send(f"1:{original.index('return ') + len('return ') + 1}\r")
    app.send('ans')
    wait(app, 'Suggestions · Tab accepts')
    app.send(b'\t')
    save(app, source, original.replace('return ;', 'return answer;'))
    app.send(CTRL_Z)
    save(app, source, original.replace('return ;', 'return ans;'))
    app.finish()
    print('PASS: installed clangd automatic caret popup + Tab with empty PATH/no Node, exact CRLF/Unicode save and native undo')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-suggest-pty-') as directory:
        try:
            (real if '--real-clangd' in sys.argv else fixture)(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if app.process.poll() is None:
                        os.kill(app.process.pid, signal.SIGTERM)
                        try:
                            app.process.wait(timeout=6)
                        except subprocess.TimeoutExpired:
                            app.process.kill()
                            app.process.wait(timeout=3)
                    app.close_fds()
                    LIVE.remove(app)
                except Exception as error:
                    errors.append(error)
            if errors:
                if failed:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
