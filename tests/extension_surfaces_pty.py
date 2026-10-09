#!/usr/bin/env python3
"""Native output/status/lazy-tree interactions with real extension commands."""
import os
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from extension_sessions_pty import Editor, LIVE, command, wait, save
from pty_smoke import CTRL_Z, eventually, wait_screen
from extension_activation_pty import install

FIXTURE = Path(__file__).resolve().parent / 'fixtures' / 'extension-surfaces'


def run(root):
    file = root / 'unicode.txt'
    file.write_bytes('original猫\r\n'.encode())
    app = Editor(root, '--extension', FIXTURE, file, enhanced=True)
    wait(app, '(8 commands)')
    wait(app, 'Native Ready')
    command(app, 'Surface show')
    wait(app, 'Output: Native Fixture Output')
    wait(app, 'LOG 猫🙂')
    command(app, 'Keyboard Inspector')
    wait_screen(app, 'Keyboard Inspector · Esc closes',
                absent=('Output: Native Fixture Output', 'Native Ready'), timeout=8)
    assert 'Output: Native Fixture Output' not in app.screen.text()
    assert 'Native Ready' not in app.screen.text()
    app.send(b'\x13')
    wait(app, 'Mapped command: workbench.action.files.save')
    assert file.read_bytes() == 'original猫\r\n'.encode()
    app.send(b'\x1b')
    wait(app, 'Output: Native Fixture Output')
    wait(app, 'LOG 猫🙂')
    wait(app, 'Native Ready')
    app.paste('MUST NOT EDIT')
    app.send('x\r\x7f')
    app.send(b'\x1b')
    command(app, 'Extensions: Status Items')
    wait(app, 'Extension Status Items')
    wait(app, 'Runs opaque action')
    app.send(b'\r')
    wait(app, 'Surface applied=true')
    save(app, file, 'INSERTED猫original猫\r\n')
    app.send(CTRL_Z)
    save(app, file, 'original猫\r\n')
    command(app, 'Extensions: Tree Views')
    wait(app, 'Extension Tree Views')
    app.send(b'\r')
    wait(app, 'Group')
    app.send(b'\x1b[C')
    wait(app, 'Leaf 猫')
    app.send(b'\x1b[B\r')
    wait(app, 'Surface applied=true')
    save(app, file, 'INSERTED猫original猫\r\n')
    app.send(CTRL_Z)
    save(app, file, 'original猫\r\n')
    command(app, 'Surface refresh')
    wait(app, 'Extension command completed')
    command(app, 'Extensions: Tree Views')
    wait(app, 'Extension Tree Views')
    app.send(b'\r')
    wait(app, 'Group refreshed')
    app.send(b'\x1b')
    command(app, 'Surface crash')
    wait(app, 'Extension host stopped')
    eventually(lambda: app.read() and 'Native Ready' not in app.screen.text())
    save(app, file, 'original猫\r\n')
    app.finish()
    print('PASS: Inspector hides/restores surfaces, read-only output, status/tree opaque edits, refresh, crash, Unicode CRLF save/undo')

    empty = root / 'empty'
    empty.mkdir()
    app = Editor(empty, '--extension', FIXTURE, enhanced=True)
    wait(app, '(8 commands)')
    wait(app, 'No open editors')
    command(app, 'Extensions: Status Items')
    wait(app, 'Extension Status Items')
    app.send(b'\r')
    wait(app, 'No active editor; no document created')
    wait(app, 'No open editors')
    command(app, 'Surface prompt')
    wait(app, 'Surface input')
    app.send(b'\r')
    wait(app, 'Surface input=answer')
    wait(app, 'No open editors')
    app.send(b'\x1b')
    command(app, 'Surface dispose')
    wait(app, 'Extension command completed')
    eventually(lambda: app.read() and 'Native Ready' not in app.screen.text())
    app.finish()
    print('PASS: empty workbench status/output/input and disposal create no phantom document')


def lazy(root):
    root.mkdir()
    store, config = root / 'store', root / 'config'
    install(root, store, 'b', r"""
const v=require('vscode');
const leaf={};v.window.createTreeView('lazy.tree',{treeDataProvider:{
 getChildren:()=>[leaf],getTreeItem:()=>({label:'Lazy native leaf',command:{command:'lazy.edit'}})
}});
exports.activate=ctx=>{
 ctx.subscriptions.push(v.commands.registerCommand('lazy.ready',()=>v.window.showInformationMessage('new owner ready')));
 ctx.subscriptions.push(v.commands.registerCommand('lazy.edit',async()=>{
  const ok=await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'LAZY:'));
  await v.window.showInformationMessage('lazy tree applied='+ok);
 }));
};
""", {'contributes': {'commands': [{'command': 'lazy.ready', 'title': 'Lazy Ready'}],
                         'views': {'explorer': [{'id': 'lazy.tree', 'name': 'Lazy Native Tree'}]}}})
    install(root, store, 'c', r"""
const v=require('vscode');exports.activate=async()=>{
 const out=v.window.createOutputChannel('Failed owner log');out.appendLine('must disappear');
 const status=v.window.createStatusBarItem('failed');status.text='FAILED OWNER';status.show();
 await v.window.showInputBox({title:'Fail new surface owner'});throw new Error('intentional surface failure');
};
""", {'contributes': {'commands': [{'command': 'lazy.fail', 'title': 'Lazy Fail'}]}})
    keys = root / 'keys.json'
    keys.write_text(json.dumps([
        {'key': 'f5', 'command': 'vscli.extensions.enableGlobal', 'args': {'id': 'activation.b'}},
        {'key': 'f6', 'command': 'vscli.extensions.enableGlobal', 'args': {'id': 'activation.c'}},
        {'key': 'f7', 'command': 'lazy.ready'}, {'key': 'f8', 'command': 'lazy.fail'},
    ]))
    file = root / 'original.txt'; file.write_bytes('original猫\r\n'.encode())
    app = Editor(root, '--config-dir', config, '--extensions-dir', store, '--keybindings', keys,
                 '--extension', FIXTURE, file, enhanced=True)
    wait(app, 'Native Ready')
    app.send(b'\x1b[15~'); wait(app, 'Enabled activation.b;')
    app.send(b'\x1b[18~'); wait(app, 'new owner ready')
    command(app, 'Extensions: Tree Views'); wait(app, 'Lazy Native Tree')
    app.send(b'\r'); wait(app, 'Lazy native leaf')
    app.send(b'\r'); wait(app, 'lazy tree applied=true')
    save(app, file, 'LAZY:original猫\r\n')
    app.send(CTRL_Z); save(app, file, 'original猫\r\n')
    app.send(b'\x1b[17~'); wait(app, 'Enabled activation.c;')
    app.send(b'\x1b[19~'); wait(app, 'Fail new surface owner')
    app.send(b'\r'); wait(app, 'Extension activation.c failed to activate; restart explicitly to retry')
    eventually(lambda: app.read() and 'FAILED OWNER' not in app.screen.text())
    wait(app, 'Native Ready')
    command(app, 'Surface show'); wait(app, 'LOG 猫🙂')
    app.send(b'\x1b'); save(app, file, 'original猫\r\n'); app.finish()
    print('PASS: newly admitted module-level tree, opaque native edit/undo, failed owner cleanup and older surface retention')


def interrupted_output(root):
    root.mkdir()
    file = root / 'preserved.txt'
    original = 'native 猫🙂\r\n'.encode()
    file.write_bytes(original)
    app = Editor(root, '--extension', FIXTURE, file, enhanced=True)
    wait(app, 'Native Ready')
    command(app, 'Surface show')
    # Interrupt without consuming the resulting output-panel redraw first.
    stop_surface_process(app)
    assert app.process.restored, 'Surface interruption leaked terminal mode'
    assert file.read_bytes() == original
    app.close_fds()
    LIVE.remove(app)
    print('PASS: interrupted surface output drains within exit deadline and restores terminal/native bytes')


def stop_surface_process(app):
    if app.process.poll() is not None:
        return
    deadline = time.monotonic() + 3
    try:
        os.kill(app.process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        # Exit/terminal restoration may be blocked behind a full PTY output queue.
        # Drain within the existing SIGTERM deadline before waiting on the supervisor.
        eventually(lambda: app.read() and app.process.poll() is not None,
                   timeout=max(0, deadline - time.monotonic()))
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError('Surface cleanup exceeded its SIGTERM deadline')
        app.process.wait(timeout=remaining)
    except (AssertionError, subprocess.TimeoutExpired):
        app.process.kill()
        app.process.wait(timeout=3)


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-surfaces-pty-') as directory:
        try:
            run(Path(directory))
            lazy(Path(directory) / 'lazy')
            interrupted_output(Path(directory) / 'interrupt')
        finally:
            original_failure = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    stop_surface_process(app)
                except Exception as error:
                    errors.append(error)
                finally:
                    app.close_fds()
            if errors:
                if original_failure:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
