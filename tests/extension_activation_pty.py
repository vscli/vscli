#!/usr/bin/env python3
"""Explicit native grants and lazy dependency activation through a real Unix PTY."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import zipfile
from extension_sessions_pty import Editor, LIVE, command, wait, save, alive
from pty_smoke import BINARY, CTRL_Z, eventually
F5, F6, F7, F8 = (b'\x1b[15~', b'\x1b[17~', b'\x1b[18~', b'\x1b[19~')


def install(root, store, name, source, extra):
    manifest = {'publisher': 'activation', 'name': name, 'version': '1.0.0', 'main': 'main.cjs', **extra}
    archive = root / f'{name}.vsix'
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as writer:
        writer.writestr('extension/package.json', json.dumps(manifest))
        writer.writestr('extension/main.cjs', source)
    subprocess.run([BINARY, '--extensions-dir', str(store), '--install-extension', str(archive)], check=True, capture_output=True)


def bindings(root, a='activation.a'):
    path = root / 'keys.json'
    path.write_text(json.dumps([
        {'key': 'f5', 'command': 'vscli.extensions.enableGlobal', 'args': {'id': a}},
        {'key': 'f6', 'command': 'vscli.extensions.enableGlobal', 'args': {'id': 'activation.b'}},
        {'key': 'f7', 'command': 'vscli.extensions.disableGlobal', 'args': {'id': a}},
        {'key': 'f8', 'command': 'activation.edit', 'args': ['猫', 2]},
    ]))
    return path


def run(root):
    store, config = root / 'store', root / 'config'
    source_a = r"""
const vscode=require('vscode');
exports.activate=context=>{
 require('node:fs').writeFileSync('host.pid',String(process.pid));
 const dependency=vscode.extensions.getExtension('activation.b');
 context.subscriptions.push(vscode.commands.registerCommand('activation.edit',async args=>{
  const editor=vscode.window.activeTextEditor;
  const applied=await editor.edit(edit=>edit.insert(new vscode.Position(0,0),dependency.exports.prefix+JSON.stringify(args)+':'));
  await vscode.window.showInformationMessage('lazy applied='+applied);
 }));
};
"""
    install(root, store, 'a', source_a, {'extensionDependencies': ['activation.b'], 'contributes': {'commands': [{'command': 'activation.edit', 'title': 'Lazy dependency edit'}]}})
    install(root, store, 'b', "exports.activate=()=>({prefix:'shared:'});", {})
    keys = bindings(root)
    file = root / 'native.txt'; file.write_text('original🙂')
    app = Editor(root, '--config-dir', config, '--extensions-dir', store, '--keybindings', keys, file, enhanced=True)
    app.send(F5); wait(app, 'Enabled activation.a;')
    assert not (root / 'host.pid').exists()
    assert json.loads((config / 'extensions-enabled.json').read_text())['extensions'] == {'activation.a': True}
    app.send(F8); wait(app, 'activation.b is disabled')
    assert not (root / 'host.pid').exists()
    app.send(F6); wait(app, 'Enabled activation.b;')
    assert not (root / 'host.pid').exists()
    app.send(F8); wait(app, 'lazy applied=true')
    save(app, file, 'shared:["猫",2]:original🙂')
    leader = int((root / 'host.pid').read_text())
    app.send(CTRL_Z); save(app, file, 'original🙂')
    app.send(F7); wait(app, 'Disabled activation.a;')
    eventually(lambda: app.read() and not alive(leader))
    app.send('N'); save(app, file, 'Noriginal🙂')
    app.finish()
    assert json.loads((config / 'extensions-enabled.json').read_text())['extensions'] == {'activation.a': False, 'activation.b': True}
    app = Editor(root, '--config-dir', config, '--extensions-dir', store, '--keybindings', keys, file, enhanced=True)
    app.send(F8); wait(app, 'Command not implemented')
    save(app, file, 'Noriginal🙂'); app.finish()
    print('PASS: explicit atomic remembered grants, separate dependency consent, zero Node until command, shared Unicode edit/save/undo, disable reaping and restart persistence')

    prompt = root / 'prompt'; prompt.mkdir()
    prompt_store, prompt_config = prompt / 'store', prompt / 'config'
    install(prompt, prompt_store, 'a', r"""
const vscode=require('vscode'); exports.activate=context=>{
 require('node:fs').writeFileSync('prompt-host.pid',String(process.pid));
 context.subscriptions.push(vscode.commands.registerCommand('activation.edit',async()=>{
  const value=vscode.extensions.getExtension('activation.b').exports.value;
  if(value===undefined) return vscode.window.showInformationMessage('dependency input canceled; no edit');
  const applied=await vscode.window.activeTextEditor.edit(edit=>edit.insert(new vscode.Position(0,0),value));
  await vscode.window.showInformationMessage('dependency input edit='+applied);
 }));
};
""", {'extensionDependencies': ['activation.b'], 'activationEvents': ['onCommand:activation.edit']})
    install(prompt, prompt_store, 'b', "const vscode=require('vscode');exports.activate=async()=>({value:await vscode.window.showInputBox({title:'Lazy dependency input'})});", {})
    prompt_keys = bindings(prompt); prompt_file = prompt / 'native.txt'; prompt_file.write_text('original')
    app = Editor(prompt, '--config-dir', prompt_config, '--extensions-dir', prompt_store, '--keybindings', prompt_keys, prompt_file, enhanced=True)
    app.send(F5); wait(app, 'Enabled activation.a;'); app.send(F6); wait(app, 'Enabled activation.b;')
    app.send(F8); wait(app, 'Lazy dependency input'); app.send(b'\x1b')
    wait(app, 'dependency input canceled; no edit'); save(app, prompt_file, 'original')
    command(app, 'Extensions: Restart Selected Session'); wait(app, 'Lazy dependency input')
    app.paste('猫'); app.send(b'\r'); wait(app, '(1 commands)')
    app.send(F8); wait(app, 'dependency input edit=true'); save(app, prompt_file, '猫original')
    app.send(CTRL_Z); save(app, prompt_file, 'original'); app.finish()
    print('PASS: non-contributed onCommand activates selected dependency, native activation Input Box cancel/accept, explicit restart and exact edit/undo')

    held = root / 'held'; held.mkdir()
    held_store, held_config = held / 'store', held / 'config'
    install(held, held_store, 'a', r"""
const vscode=require('vscode'),fs=require('node:fs');
exports.activate=async context=>{
 fs.writeFileSync('held-host.pid',String(process.pid));
 await new Promise(resolve=>{const timer=setInterval(()=>{if(fs.existsSync('release')){clearInterval(timer);resolve();}},5);});
 context.subscriptions.push(vscode.commands.registerCommand('activation.edit',async()=>{
  const applied=await vscode.window.activeTextEditor.edit(edit=>edit.insert(new vscode.Position(0,0),'extension:'));
  await vscode.window.showInformationMessage('held context edit='+applied);
 }));
};
""", {'activationEvents': ['onCommand:activation.edit']})
    held_keys = bindings(held); held_file = held / 'native.txt'; held_file.write_text('original🙂')
    app = Editor(held, '--config-dir', held_config, '--extensions-dir', held_store, '--keybindings', held_keys, held_file, enhanced=True)
    app.send(F5); wait(app, 'Enabled activation.a;'); app.send(F8)
    eventually(lambda: app.read() and (held / 'held-host.pid').exists())
    app.send('N'); wait(app, 'Pending extension command canceled')
    save(app, held_file, 'Noriginal🙂')
    (held / 'release').write_text('yes'); wait(app, '(1 commands)')
    save(app, held_file, 'Noriginal🙂')
    app.send(F8); wait(app, 'held context edit=true'); save(app, held_file, 'extension:Noriginal🙂')
    app.send(CTRL_Z); save(app, held_file, 'Noriginal🙂'); app.finish()
    print('PASS: native input/save remains responsive during held activation, deferred command is canceled, later explicit edit/undo preserves exact bytes')

    picker = root / 'picker'; picker.mkdir()
    picker_store, picker_config = picker / 'store', picker / 'config'
    install(picker, picker_store, 'a', r"""
const vscode=require('vscode'); exports.activate=context=>{
 require('node:fs').writeFileSync('picker-host.pid',String(process.pid));
 context.subscriptions.push(vscode.commands.registerCommand('activation.edit',async()=>{
  const applied=await vscode.window.activeTextEditor.edit(edit=>edit.insert(new vscode.Position(0,0),'extension:'));
  await vscode.window.showInformationMessage('picker edit='+applied);
 }));
};
""", {'contributes': {
        'commands': [{'command': 'activation.edit', 'title': 'Picker edit'}],
        'keybindings': [{'key': 'f9', 'command': 'activation.edit'}],
    }})
    picker_file = picker / 'native.txt'; picker_file.write_bytes('original🙂\r\n'.encode())
    original = picker_file.read_bytes()
    app = Editor(picker, '--config-dir', picker_config, '--extensions-dir', picker_store, picker_file, enhanced=True)
    command(app, 'Extensions: Show Installed Extensions'); wait(app, 'e/d global')
    app.send('e'); wait(app, 'Enabled activation.a;')
    assert 'enabled; waiting for a supported event' in app.screen.text()
    assert not (picker / 'picker-host.pid').exists()
    app.send('D'); wait(app, 'Disabled activation.a;')
    workspace_files = list((picker_config / 'extension-workspaces').glob('*.json'))
    assert len(workspace_files) == 1
    workspace_grants = workspace_files[0]
    assert json.loads(workspace_grants.read_text())['extensions'] == {'activation.a': False}
    assert json.loads((picker_config / 'extensions-enabled.json').read_text())['extensions'] == {'activation.a': True}
    app.send('E'); wait(app, 'Enabled activation.a;')
    app.send('d'); wait(app, 'Disabled activation.a;')
    assert json.loads((picker_config / 'extensions-enabled.json').read_text())['extensions'] == {'activation.a': False}
    assert json.loads(workspace_grants.read_text())['extensions'] == {'activation.a': True}
    assert 'enabled; waiting for a supported event' in app.screen.text()
    assert not (picker / 'picker-host.pid').exists()
    assert picker_file.read_bytes() == original
    app.send(b'\x1b'); app.send(b'\x1b[20~'); wait(app, 'picker edit=true')
    leader = int((picker / 'picker-host.pid').read_text())
    save(app, picker_file, 'extension:original🙂\r\n')
    app.send(CTRL_Z); save(app, picker_file, 'original🙂\r\n')
    command(app, 'Extensions: Show Installed Extensions'); wait(app, 'e/d global')
    app.send('D'); wait(app, 'Disabled activation.a;')
    eventually(lambda: app.read() and not alive(leader))
    app.send(b'\x1b'); app.send('N'); save(app, picker_file, 'Noriginal🙂\r\n')
    app.finish()
    app = Editor(picker, '--config-dir', picker_config, '--extensions-dir', picker_store, picker_file, enhanced=True)
    command(app, 'Extensions: Show Installed Extensions'); wait(app, 'e/d global')
    assert 'disabled' in app.screen.text()
    app.send(b'\x1b'); save(app, picker_file, 'Noriginal🙂\r\n'); app.finish()
    print('PASS: installed picker global/workspace enable-disable keys, effective overrides, lazy original F9, native CRLF Unicode save/undo, disable cleanup and remembered restart')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-activation-pty-') as directory:
        root = Path(directory)
        try:
            run(root)
        finally:
            original_failure = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if app.process.poll() is None:
                        try:
                            os.kill(app.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            app.process.wait(timeout=3)
                        except (AssertionError, subprocess.TimeoutExpired):
                            app.process.kill(); app.process.wait(timeout=3)
                    app.close_fds()
                except Exception as error:
                    errors.append(error)
            for path in root.rglob('*.pid'):
                try:
                    pid = int(path.read_text())
                    if os.getpgid(pid) == pid:
                        os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if errors:
                if original_failure:
                    print(f'Additional cleanup errors: {errors}', file=sys.stderr)
                else:
                    raise errors[0]
