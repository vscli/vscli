#!/usr/bin/env python3
"""Native extension Quick Pick/Input Box behavior through the real Unix terminal."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command, wait, save, alive
from pty_smoke import CTRL_Z, eventually


def fixture(root):
    folder = root / 'extension'
    folder.mkdir()
    manifest = {'publisher': 'fixture', 'name': 'prompts', 'version': '1.0.0', 'main': 'extension.cjs',
                'contributes': {'commands': [{'command': 'prompts.' + name, 'title': 'Prompt ' + name}
                                             for name in ('edit', 'cancel', 'queue', 'crash', 'unsupported')],
                                'keybindings': [{'key': 'f6', 'command': 'prompts.edit'}]}}
    (folder / 'package.json').write_text(json.dumps(manifest))
    (folder / 'extension.cjs').write_text(r"""
const vscode = require('vscode');
exports.activate = context => {
 require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath,'host.pid'), String(process.pid));
 const register = (id, call) => context.subscriptions.push(vscode.commands.registerCommand('prompts.'+id,call));
 register('edit', async()=> {
   const chosen = await vscode.window.showQuickPick([{label:'One',description:'first',detail:'first detail'}, {label:'Two 猫',description:'second',detail:'find me',data:'picked'}], {title:'Choose native item',placeHolder:'Filter items',matchOnDetail:true});
   if (!chosen) return vscode.window.showInformationMessage('pick canceled');
   const value = await vscode.window.showInputBox({title:'Type native input',prompt:'Enter Unicode',placeHolder:'Input hint',value:'default'});
   if(value === undefined) return vscode.window.showInformationMessage('input canceled');
   const editor=vscode.window.activeTextEditor;
   const applied=await editor.edit(edit=>edit.insert(new vscode.Position(0,0), chosen.data+value));
   await vscode.window.showInformationMessage('prompt applied='+applied);
 });
 register('cancel', async()=> {
   const result=await vscode.window.showInputBox({title:'Cancel input'});
   await vscode.window.showInformationMessage('cancel result='+result);
 });
 register('queue', async()=> {
   const values=await Promise.all([vscode.window.showInputBox({title:'First queued'}),vscode.window.showInputBox({title:'Second queued'})]);
   await vscode.window.showInformationMessage('queued='+JSON.stringify(values));
 });
 register('unsupported', async()=> {
   try {await vscode.window.showInputBox({title:'Must not appear',password:true});}
   catch(error){ await vscode.window.showInformationMessage('password rejected'); }
 });
 register('crash', async()=> {
   await vscode.window.showInputBox({title:'Crash pending'});
 });
};
""")
    return folder


def run(root):
    extension = fixture(root)
    file = root / 'shared.txt'
    file.write_text('original')
    app = Editor(root, '--extension', extension, file, enhanced=True)
    wait(app, '(5 commands)')
    app.send(b'\x1b[17~')  # F6 avoids searching the native palette.
    wait(app, 'Choose native item')
    wait(app, 'Two 猫')
    wait(app, 'second')
    wait(app, 'find me')
    app.paste('find me')
    app.send(b'\r')
    wait(app, 'Type native input')
    wait(app, 'Enter Unicode')
    wait(app, 'Input hint')
    app.paste('x' * 5000)
    wait(app, 'Extension prompt text exceeds 4 KiB')
    app.paste('🙂')
    app.send(b'\r')
    wait(app, 'prompt applied=true')
    save(app, file, 'picked🙂original')
    app.send(CTRL_Z)
    save(app, file, 'original')
    command(app, 'Prompt cancel')
    wait(app, 'Cancel input')
    app.send(b'\x1b')
    wait(app, 'cancel result=undefined')
    command(app, 'Prompt unsupported')
    wait(app, 'password rejected')
    assert 'Must not appear' not in app.screen.text()
    print('PASS: native Quick Pick item details/filtering, Unicode Input Box, atomic edit/save/undo, cancellation and unsupported password rejection')

    # Two awaited requests share one FIFO native prompt slot.
    app.send(b'\x1bOP')
    wait(app, 'Command Palette')
    app.send('Prompt queue\r')
    wait(app, 'First queued')
    app.send(b'\x1b')
    wait(app, 'Second queued')
    app.paste('done')
    app.send(b'\r')
    wait(app, 'queued=[null,"done"]')
    command(app, 'Prompt crash')
    wait(app, 'Crash pending')
    child = int((root / 'host.pid').read_text())
    # Trigger the crash only after the pending prompt is observable. A fixed
    # timer can terminate Node before the terminal renders on a busy runner.
    assert os.getpgid(child) == child
    os.kill(child, signal.SIGKILL)
    wait(app, 'Extension host stopped')
    eventually(lambda: not alive(child))
    assert 'Crash pending' not in app.screen.text()
    app.send('!')
    save(app, file, '!original')
    app.send(CTRL_Z)
    save(app, file, 'original')
    command(app, 'Extensions: Restart Selected Session')
    wait(app, '(5 commands)')
    command(app, 'Prompt cancel')
    wait(app, 'Cancel input')
    app.send(b'\x1b')
    wait(app, 'cancel result=undefined')
    command(app, 'Extensions: Stop Host')
    wait(app, 'Extension host stopped')
    child = int((root / 'host.pid').read_text())
    eventually(lambda: not alive(child))
    app.finish()
    print('PASS: FIFO prompts, pending-prompt crash cleanup, responsive native edit/save, explicit restart and stop without stale UI')

    startup = root / 'startup'
    startup.mkdir()
    (startup / 'package.json').write_text(json.dumps({'publisher': 'fixture', 'name': 'activation', 'version': '1.0.0', 'main': 'extension.cjs', 'contributes': {'commands': [{'command': 'startup.answer', 'title': 'Startup answer'}]}}))
    (startup / 'extension.cjs').write_text(r"""
const vscode=require('vscode');
require('node:fs').writeFileSync(require('node:path').join(vscode.workspace.rootPath,'startup.pid'),String(process.pid));
const answer=vscode.window.showInputBox({title:'Activation input',prompt:'Module evaluation input',value:'initial'});
exports.activate=async context=>{
 const value=await answer;
 context.subscriptions.push(vscode.commands.registerCommand('startup.answer',()=>vscode.window.showInformationMessage('startup answer='+value)));
};
""")
    app = Editor(root, '--extension', startup, file, enhanced=True)
    wait(app, 'Activation input')
    wait(app, 'Module evaluation input')
    app.paste('answer🙂')
    app.send(b'\r')
    wait(app, '(1 commands)')
    command(app, 'Startup answer')
    wait(app, 'startup answer=answer🙂')
    command(app, 'Extensions: Restart Selected Session')
    wait(app, 'Activation input')
    app.send(b'\x1b')
    wait(app, '(1 commands)')
    command(app, 'Startup answer')
    wait(app, 'startup answer=undefined')
    save(app, file, 'original')
    app.finish()
    print('PASS: module-evaluation Input Box accepts or cancels before activation completes, with native text unchanged')



if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-prompts-pty-') as directory:
        root = Path(directory)
        try:
            run(root)
        finally:
            original_failure = sys.exc_info()[0] is not None
            cleanup_errors = []
            for app in LIVE:
                try:
                    if app.process.poll() is None:
                        try:
                            os.kill(app.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            app.process.wait(timeout=3)
                        except (AssertionError, subprocess.TimeoutExpired):
                            app.process.kill()
                            app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    cleanup_errors.append(error)
            for path in root.glob('*.pid'):
                pid = int(path.read_text())
                try:
                    if os.getpgid(pid) == pid:
                        os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if cleanup_errors:
                if original_failure:
                    print(f'Additional PTY cleanup errors: {cleanup_errors}', file=sys.stderr)
                else:
                    raise cleanup_errors[0]
