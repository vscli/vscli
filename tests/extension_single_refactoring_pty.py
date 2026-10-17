#!/usr/bin/env python3
"""Single-resource refactoring never triggers multi-resource refactoring autosave."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, eventually, wait_screen

ORIGINAL = '猫🙂 A\r\nlast\r\n'
PEER = 'β🙂 B\r\n'
DIRTY = 'dirty ' + ORIGINAL
EDITED = 'rename ' + DIRTY


def fixture(root, policy):
    extension = root / 'extension'
    extension.mkdir()
    commands = [('apply', 'Single refactoring apply'), ('undo', 'Single refactoring verify Undo'),
                ('redo', 'Single refactoring verify Redo'), ('save', 'Single refactoring save')]
    (extension / 'package.json').write_text(json.dumps({
        'publisher': 'fixture', 'name': 'single-refactoring', 'version': '1.0.0', 'main': 'extension.cjs',
        'contributes': {'commands': [{'command': 'single.' + name, 'title': title} for name, title in commands]},
    }), encoding='utf-8')
    (extension / 'extension.cjs').write_text(r"""
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const source=v.window.activeTextEditor.document,dirty='dirty 猫🙂 A\r\nlast\r\n',edited='rename '+dirty;
 let peer,changes=0,saves=0,applying=false;
 const register=(name,fn)=>context.subscriptions.push(v.commands.registerCommand('single.'+name,fn));
 function same(text){assert.strictEqual(v.window.activeTextEditor.document,source);assert(v.workspace.textDocuments.includes(source));assert(v.workspace.textDocuments.includes(peer));assert.equal(source.getText(),text);assert.equal(peer.getText(),'β🙂 B\r\n');assert.equal(peer.isDirty,false);}
 context.subscriptions.push(v.workspace.onDidChangeTextDocument(event=>{if(event.document===source){changes++;if(applying){same(edited);assert(source.isDirty);}}}));
 context.subscriptions.push(v.workspace.onDidSaveTextDocument(doc=>{assert.strictEqual(doc,source);assert.equal(doc.isDirty,false);saves++;}));
 register('apply',async()=>{
  peer=await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'b.txt'));same(dirty);
  assert(v.window.visibleTextEditors.every(editor=>editor.document!==peer));
  const before=source.version;changes=0;applying=true;
  const edit=new v.WorkspaceEdit();edit.insert(source.uri,new v.Position(0,0),'rename ');
  assert.equal(await v.workspace.applyEdit(edit,{isRefactoring:true}),true);
  applying=false;same(edited);assert.equal(changes,1);assert.equal(saves,0);assert(source.version>before);assert(source.isDirty);
  return v.window.showInformationMessage('API single refactoring;applied=true;changes=1;saves=0;hidden=true');
 });
 register('undo',()=>{same(dirty);assert(source.isDirty);assert.equal(saves,0);return v.window.showInformationMessage('API single refactoring Undo;dirtyOriginal=true;saves=0');});
 register('redo',()=>{same(edited);assert(source.isDirty);assert.equal(saves,0);return v.window.showInformationMessage('API single refactoring Redo;identity=true;saves=0');});
 register('save',async()=>{same(edited);assert.equal(await source.save(),true);same(edited);assert.equal(saves,1);assert.equal(source.isDirty,false);return v.window.showInformationMessage('API single refactoring explicit Save;didSave=1;identity=true');});
};
""", encoding='utf-8')
    config = root / 'config'
    config.mkdir()
    settings = {'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
                'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {}}
    if policy is not None:
        settings['files.refactoring.autoSave'] = policy
    (config / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
    node = shutil.which('node')
    assert node, 'Single-refactoring API qualification requires optional Node'
    return extension, config, str(Path(node).resolve())


def run(root, policy):
    extension, config, node = fixture(root, policy)
    source, peer = root / 'a.txt', root / 'b.txt'
    source.write_bytes(ORIGINAL.encode())
    peer.write_bytes(PEER.encode())
    app = Editor(root, '--extension', extension, '--extension-node', node,
                 '--config-dir', config, '--no-session', source, enhanced=True, extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.txt')
    app.paste('dirty ')
    command(app, 'Single refactoring apply')
    wait_screen(app, 'API single refactoring;applied=true;changes=1;saves=0;hidden=true')
    assert source.read_bytes() == ORIGINAL.encode()
    assert peer.read_bytes() == PEER.encode()
    app.send(CTRL_Z)
    command(app, 'Single refactoring verify Undo')
    wait_screen(app, 'API single refactoring Undo;dirtyOriginal=true;saves=0')
    app.send(b'\x1b[122;6u')
    command(app, 'Single refactoring verify Redo')
    wait_screen(app, 'API single refactoring Redo;identity=true;saves=0')
    assert source.read_bytes() == ORIGINAL.encode()
    assert peer.read_bytes() == PEER.encode()
    command(app, 'Single refactoring save')
    wait_screen(app, 'API single refactoring explicit Save;didSave=1;identity=true')
    eventually(lambda: source.read_bytes() == EDITED.encode())
    app.finish()
    assert source.read_bytes() == EDITED.encode()
    assert peer.read_bytes() == PEER.encode()
    print(f'PASS: single-resource refactoring policy={policy} stays unsaved with atomic mirror receipt, dirty Unicode CRLF UndoRedo and hidden peer preserved; only explicit Save writes native bytes')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-single-refactor-') as directory:
        try:
            for name, policy in [('default', None), ('enabled', True), ('disabled', False)]:
                root = Path(directory, name)
                root.mkdir()
                run(root.resolve(), policy)
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Single-refactoring interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
