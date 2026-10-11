#!/usr/bin/env python3
"""Real selected workspace.applyEdit: atomic mirrors, grouped history, real saves.
Fixture JavaScript never reads/writes document files. Native operations own every
edit and persistence; explicit refactoring autosave=false permits metadata=true.
"""
import json
from pathlib import Path
import shutil
import sys
import tempfile

from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_Z, eventually, wait_screen

A = '猫🙂 A\r\nlast\r\n'
B = 'β🙂 B\r\n'
EDITED_A = 'edit ' + A
EDITED_B = 'cohort ' + B


def fixture(root):
    extension = root / 'extension'
    extension.mkdir()
    operations = ('apply', 'undo', 'redo', 'save')
    titles = ('Workspace cohort apply', 'Workspace cohort verify undo',
              'Workspace cohort verify redo', 'Workspace cohort save both')
    (extension / 'package.json').write_text(json.dumps({
        'publisher': 'fixture', 'name': 'workspace-cohort', 'version': '1.0.0',
        'main': 'extension.cjs',
        'contributes': {'commands': [
            {'command': 'cohort.' + operation, 'title': title}
            for operation, title in zip(operations, titles)]},
    }), encoding='utf-8')
    (extension / 'extension.cjs').write_text(r"""
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=c=>{
 const a=v.window.activeTextEditor.document;
 const originalA='猫🙂 A\r\nlast\r\n',originalB='β🙂 B\r\n';
 const editedA='edit '+originalA,editedB='cohort '+originalB;
 let b,applying=false,changesA=0,changesB=0,saved=0,mirrorFailure;
 const register=(name,callback)=>c.subscriptions.push(v.commands.registerCommand('cohort.'+name,callback));
 function identity(){
  assert(v.workspace.textDocuments.includes(a));assert(v.workspace.textDocuments.includes(b));
  assert.strictEqual(v.window.activeTextEditor.document,a);
 }
 function text(first,second){identity();assert.equal(a.getText(),first);assert.equal(b.getText(),second);}
 c.subscriptions.push(v.workspace.onDidChangeTextDocument(event=>{
  if(event.document===a)changesA++;
  else if(event.document===b)changesB++;
  else return;
  if(applying){
   // ALL target mirrors must already be final even in the first callback,
   // before the applyEdit promise can resolve true.
   try{text(editedA,editedB);assert(a.isDirty);assert(b.isDirty);}
   catch(error){if(mirrorFailure===undefined)mirrorFailure=String(error.message).slice(0,4096);}
  }
 }));
 c.subscriptions.push(v.workspace.onDidSaveTextDocument(document=>{
  assert(document===a || document===b);identity();assert.equal(document.isDirty,false);saved++;
 }));
 register('apply',async()=>{
  assert.equal(a.getText(),originalA);
  b=await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'b.txt'));
  text(originalA,originalB);
  assert(v.window.visibleTextEditors.every(editor=>editor.document!==b),'openTextDocument must retain B without an editor');
  assert.equal(v.workspace.getConfiguration('files').get('refactoring.autoSave'),false);
  const versionA=a.version,versionB=b.version;changesA=0;changesB=0;applying=true;
  const edit=new v.WorkspaceEdit();
  edit.insert(a.uri,new v.Position(0,0),'edit ');
  edit.insert(b.uri,new v.Position(0,0),'cohort ');
  const result=await v.workspace.applyEdit(edit,{isRefactoring:true});
  assert.equal(result,true);text(editedA,editedB);
  assert.equal(mirrorFailure,undefined,'atomic target mirrors failed inside a pre-ack change callback');
  assert.equal(changesA,1);assert.equal(changesB,1);
  assert(a.version>versionA);assert(b.version>versionB);assert(a.isDirty);assert(b.isDirty);
  applying=false;
  await v.window.showInformationMessage('API apply true;events=2;hidden=true');
 });
 register('undo',()=>{
  text(originalA,originalB);assert.equal(a.isDirty,false);assert.equal(b.isDirty,false);
  return v.window.showInformationMessage('API grouped undo;identity=true');
 });
 register('redo',()=>{
  text(editedA,editedB);assert(a.isDirty);assert(b.isDirty);
  return v.window.showInformationMessage('API grouped redo;identity=true');
 });
 register('save',async()=>{
  text(editedA,editedB);const before=saved;
  assert.equal(await a.save(),true);assert.equal(saved,before+1);identity();
  assert.equal(a.isDirty,false);assert(b.isDirty);
  assert.equal(await b.save(),true);assert.equal(saved,before+2);text(editedA,editedB);
  assert.equal(a.isDirty,false);assert.equal(b.isDirty,false);
  await v.window.showInformationMessage('API saved A+B;events=2;identity=true');
 });
};
""", encoding='utf-8')
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'files.refactoring.autoSave': False,
        'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual optional-host ApplyEdit gateway qualification requires Node')
    return extension, config, str(Path(node).resolve())


def unchanged(a, b):
    assert a.read_bytes() == A.encode('utf-8'), 'Workspace edit/Undo/Redo unexpectedly persisted A'
    assert b.read_bytes() == B.encode('utf-8'), 'Workspace edit/Undo/Redo unexpectedly persisted hidden B'


def run(root):
    extension, config, node = fixture(root)
    a, b = root / 'a.txt', root / 'b.txt'
    a.write_bytes(A.encode('utf-8'))
    b.write_bytes(B.encode('utf-8'))
    app = Editor(root, '--extension', extension, '--extension-node', node,
                 '--config-dir', config, '--no-session', a, enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.txt')
    command(app, 'Workspace cohort apply')
    wait_screen(app, 'API apply true;events=2;hidden=true', 'a.txt')
    unchanged(a, b)
    app.send(CTRL_Z)
    command(app, 'Workspace cohort verify undo')
    wait_screen(app, 'API grouped undo;identity=true')
    unchanged(a, b)
    # Original enhanced Ctrl+Shift+Z performs the whole native grouped Redo.
    app.send(b'\x1b[122;6u')
    command(app, 'Workspace cohort verify redo')
    wait_screen(app, 'API grouped redo;identity=true')
    unchanged(a, b)
    command(app, 'Workspace cohort save both')
    wait_screen(app, 'API saved A+B;events=2;identity=true')

    def persisted():
        app.read()
        return (a.read_bytes() == EDITED_A.encode('utf-8')
                and b.read_bytes() == EDITED_B.encode('utf-8'))

    eventually(persisted, timeout=8)
    app.finish()
    print('PASS: public refactoring ApplyEdit publishes both mirrors/events before true, '
          'retains hidden identity and Unicode CRLF disks, groups original Undo/Redo, '
          'then both actual public saves publish didSave before true')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-workspace-apply-api-') as directory:
        try:
            run(Path(directory).resolve())
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'ApplyEdit interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error:
                    errors.append(error)
            if errors and not failed:
                raise errors[0]
