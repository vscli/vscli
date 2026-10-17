#!/usr/bin/env python3
"""Optional-host filesystem writes preserve native dirty models and recovery."""
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait
from pty_smoke import eventually


def fixture(root):
    extension = root / 'extension'
    extension.mkdir()
    manifest = {'publisher': 'fixture', 'name': 'filesystem', 'version': '1.0.0', 'main': 'extension.cjs',
                'contributes': {'commands': [{'command': 'filesystem.write', 'title': 'Filesystem preserve dirty model'}]}}
    (extension / 'package.json').write_text(json.dumps(manifest))
    (extension / 'extension.cjs').write_text(r"""
const vscode=require('vscode'); const assert=require('node:assert/strict');
exports.activate=context=>{
 context.subscriptions.push(vscode.commands.registerCommand('filesystem.write',async()=>{
  const doc=await vscode.workspace.openTextDocument(vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri,'input.txt'));
  const editor=await vscode.window.showTextDocument(doc,{preview:false});
  assert.equal(vscode.workspace.getWorkspaceFolder(doc.uri),vscode.workspace.workspaceFolders[0]);
  assert.equal(vscode.workspace.asRelativePath(doc.uri),'input.txt');
  assert.equal(vscode.workspace.name,vscode.workspace.workspaceFolders[0].name);
  assert.equal(vscode.workspace.workspaceFile,undefined);
  assert(await editor.edit(edit=>edit.insert(new vscode.Position(1,0),'dirty ')));
  let saves=0;context.subscriptions.push(vscode.workspace.onDidSaveTextDocument(()=>saves++));
  await vscode.workspace.fs.writeFile(doc.uri,new TextEncoder().encode('external\r\ndisk'));
  await new Promise(resolve=>setTimeout(resolve,300));
  assert.equal(doc.getText(),'α😀\r\ndirty second');assert.equal(doc.isDirty,true);assert.equal(saves,0);
  assert.equal(new TextDecoder().decode(await vscode.workspace.fs.readFile(doc.uri)),'external\r\ndisk');
  await vscode.commands.executeCommand('undo');assert.equal(doc.getText(),'α😀\r\nsecond');
  await vscode.commands.executeCommand('redo');assert.equal(doc.getText(),'α😀\r\ndirty second');
  await vscode.window.showInformationMessage('filesystem disk receipt; native dirty undo redo retained');
 }));
};
""")
    return extension


def run(root):
    extension = fixture(root)
    file = root / 'input.txt'
    file.write_bytes('α😀\r\nsecond'.encode())
    args = ('--config-dir', root / 'config', '--recovery-dir', root / 'recovery', '--no-session')
    app = Editor(root, *args, '--extension', extension, recovery=True, enhanced=True)
    wait(app, '(1 commands)')
    command(app, 'Filesystem preserve dirty model')
    wait(app, 'filesystem disk receipt; native dirty undo redo retained')
    assert file.read_bytes() == b'external\r\ndisk'
    os.kill(app.process.pid, signal.SIGTERM)
    eventually(lambda: app.read() and app.process.poll() is not None, timeout=4)
    app.process.wait(timeout=1)
    assert app.process.restored
    app.close_fds()
    LIVE.remove(app)
    recovered = Editor(root, *args, recovery=True, enhanced=True)
    wait(recovered, 'dirty second')
    assert file.read_bytes() == b'external\r\ndisk'
    recovered.finish(discard=True)
    print('PASS: filesystem disk receipt → native dirty model/undo/redo retained → host-independent recovery; disk bytes unchanged')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-filesystem-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in list(LIVE):
                try:
                    if app.process.poll() is None:
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
