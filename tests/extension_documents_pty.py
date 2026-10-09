#!/usr/bin/env python3
"""Real terminal qualification for native extension documents and Mementos."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait, save
from pty_smoke import CTRL_Z, eventually


def fixture(root):
    extension = root / 'extension'
    extension.mkdir()
    manifest = {'publisher': 'fixture', 'name': 'documents', 'version': '1.0.0', 'main': 'extension.cjs',
                'contributes': {'commands': [{'command': 'documents.' + name, 'title': 'Document service ' + name}
                                            for name in ('show', 'edit', 'state', 'crash')],
                                'views': {'explorer': [{'id': 'documents.hiddenTree', 'name': 'Hidden Document Mirror'}]}}}
    (extension / 'package.json').write_text(json.dumps(manifest))
    (extension / 'extension.cjs').write_text(r"""
const vscode=require('vscode'); const assert=require('node:assert/strict');
exports.activate=async context=>{
 let opened;
 vscode.workspace.onDidOpenTextDocument(doc=>{opened=doc; assert.equal(vscode.window.activeTextEditor,undefined);});
 const doc=await vscode.workspace.openTextDocument(vscode.Uri.file(require('node:path').join(vscode.workspace.rootPath,'input.txt')));
 assert.equal(opened,doc); assert.equal(vscode.window.activeTextEditor,undefined); assert.deepEqual(vscode.window.visibleTextEditors,[]);
 assert.equal(doc.getText(),'α😀\r\nsecond');
 let closed=0;context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(value=>{if(value===doc)closed++;}));
 function checkHidden(){assert.equal(closed,0);assert.equal(doc.isClosed,false);assert(vscode.workspace.textDocuments.includes(doc));}
 const output=vscode.window.createOutputChannel('Hidden document output');output.appendLine('Hidden document mirror retained');output.show(true);
 const status=vscode.window.createStatusBarItem('document-mirror');status.text='Document mirror';status.command='documents.state';status.show();
 const leaf={};const tree=vscode.window.createTreeView('documents.hiddenTree',{treeDataProvider:{getChildren(){checkHidden();return [leaf];},getTreeItem(){checkHidden();return {label:'Hidden mirror intact',command:{command:'documents.state'}};}}});
 context.subscriptions.push(output,status,tree);
 await context.globalState.update('launches',context.globalState.get('launches',0)+1);
 const register=(name,call)=>context.subscriptions.push(vscode.commands.registerCommand('documents.'+name,call));
 register('show',async()=>{
  const editor=await vscode.window.showTextDocument(doc,{preview:false,selection:new vscode.Range(0,1,0,3)});
  assert.equal(editor.document,doc); assert.equal(editor,vscode.window.activeTextEditor);
  assert.equal(editor.selection.end.character,3);
  await vscode.window.showInformationMessage('native shown same identity');
 });
 register('edit',async()=>{
  assert(await vscode.window.activeTextEditor.edit(edit=>edit.insert(new vscode.Position(1,0),'dirty ')));
  await vscode.commands.executeCommand('undo'); assert.equal(doc.getText(),'α😀\r\nsecond');
  await vscode.commands.executeCommand('redo'); assert.equal(doc.getText(),'α😀\r\ndirty second');
  await vscode.window.showInformationMessage('native edit undo redo observed');
 });
 register('state',async()=>{
  checkHidden();
  await context.workspaceState.update('marker','local');
  await vscode.window.showInformationMessage('native launches='+context.globalState.get('launches')+' workspace='+context.workspaceState.get('marker'));
 });
 register('crash',()=>process.exit(7));
};
""")
    return extension


def run(root):
    extension = fixture(root)
    file = root / 'input.txt'
    original = 'α😀\r\nsecond'
    file.write_bytes(original.encode())
    config, recovery = root / 'config', root / 'recovery'
    args = ('--config-dir', config, '--recovery-dir', recovery, '--no-session')
    app = Editor(root, *args, '--extension', extension, recovery=True, enhanced=True)
    wait(app, '(4 commands)')
    wait(app, 'No open editors')
    # Activation assertions verify onDidOpen fired before the promise and both
    # active/visible editors remained empty despite the loaded hidden model.
    wait(app, 'Hidden document mirror retained')
    command(app, 'Extensions: Status Items')
    wait(app, 'Extension Status Items')
    app.send(b'\r')
    wait(app, 'native launches=1 workspace=local')
    wait(app, 'No open editors')
    command(app, 'Extensions: Tree Views')
    wait(app, 'Extension Tree Views')
    app.send(b'\r')
    wait(app, 'Hidden mirror intact')
    app.send(b'\r')
    wait(app, 'native launches=1 workspace=local')
    wait(app, 'No open editors')
    command(app, 'Document service show')
    wait(app, 'native shown same identity')
    command(app, 'Document service edit')
    wait(app, 'native edit undo redo observed')
    assert file.read_bytes().decode() == original
    command(app, 'Document service crash')
    wait(app, 'Extension host stopped')
    # SIGTERM must flush the authoritative latest native buffer even after the
    # optional host crashes; file bytes remain unchanged until explicit save.
    os.kill(app.process.pid, signal.SIGTERM)
    # Keep consuming terminal output while interruption flushes recovery and
    # restores the terminal; a full PTY must not stall the editor's exit.
    eventually(lambda: app.read() and app.process.poll() is not None, timeout=4)
    app.process.wait(timeout=1)
    assert app.process.restored, 'Terminal mode leaked after interruption'
    assert file.read_bytes().decode() == original
    app.close_fds()
    LIVE.remove(app)
    recovered = Editor(root, *args, recovery=True, enhanced=True)
    wait(recovered, 'dirty second')
    save(recovered, file, 'α😀\r\ndirty second')
    recovered.send(CTRL_Z)
    save(recovered, file, original)
    recovered.finish()
    print('PASS: welcome hidden open → output/status/tree mirror preservation → shared identity show → delegated undo/redo → host crash → dirty recovery/save/undo')
    restarted = Editor(root, *args, '--extension', extension, recovery=True, enhanced=True)
    wait(restarted, '(4 commands)')
    wait(restarted, 'No open editors')
    command(restarted, 'Document service state')
    wait(restarted, 'native launches=2 workspace=local')
    restarted.finish()
    assert file.read_bytes().decode() == original
    print('PASS: native global/workspace Mementos persist across restart without opening a visible editor')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-documents-pty-') as directory:
        try:
            run(Path(directory))
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in list(LIVE):
                try:
                    if app.process.poll() is None:
                        try:
                            os.kill(app.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            eventually(lambda: app.read() and app.process.poll() is not None, timeout=4)
                            app.process.wait(timeout=1)
                        except (AssertionError, subprocess.TimeoutExpired):
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
