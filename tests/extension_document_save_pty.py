#!/usr/bin/env python3
"""Actual public document.save -> native worker receipts; optional Node isolated.
Every edit/persist is native; fixture JavaScript observes mirrors and calls
save(), never writes document files.
"""
import json
from pathlib import Path
import shutil
import sys
import tempfile

from extension_sessions_pty import Editor, LIVE, command
from extension_surfaces_pty import stop_surface_process
from pty_smoke import CTRL_A, CTRL_Z, eventually, wait_screen

A = '猫🙂 A\r\nlast\r\n'
B = 'β🙂 B\r\n'


def fixture(root):
    extension = root / 'extension'
    extension.mkdir()
    operations = ('inactive', 'active', 'other', 'closed', 'clean', 'verify')
    titles = ('Document save inactive', 'Document save active', 'Document open other', 'Document save closed original', 'Document repeated clean save', 'Document verify clean redo')
    (extension / 'package.json').write_text(json.dumps({
        'publisher': 'fixture', 'name': 'save-receipt', 'version': '1.0.0', 'main': 'extension.cjs',
        'contributes': {'commands': [{'command': f'save.{op}', 'title': title}
            for op, title in zip(operations, titles)]},
    }), encoding='utf-8')
    (extension / 'extension.cjs').write_text(r"""
const v=require('vscode'), assert=require('node:assert/strict');
exports.activate=c=>{
 const first=v.workspace.textDocuments.find(d=>d.fileName.endsWith('a.txt'));
 let events=0;
 c.subscriptions.push(v.workspace.onDidSaveTextDocument(d=>{
  assert(v.workspace.textDocuments.includes(d));events++;
 }));
 const register=(name,callback)=>c.subscriptions.push(v.commands.registerCommand('save.'+name,callback));
 async function save(d,label){
  const before=events,original=d.uri.toString(); const identity=d;
  try{
   const result=await d.save();assert.equal(typeof result,'boolean');assert.strictEqual(identity,d);
   if(result){
    assert(events>before,'didSave precedes true');
    if(d.isClosed){
     assert(original.startsWith('untitled:'));assert.equal(d.uri.toString(),original);assert.equal(d.isDirty,false);
     const saved=v.workspace.textDocuments.find(current=>current.uri.scheme==='file'&&current.getText()===d.getText());
     assert(saved);assert.notStrictEqual(saved,d);assert.strictEqual(v.window.activeTextEditor.document,saved);
    }else assert(v.workspace.textDocuments.includes(d));
   }else {assert.equal(events,before);assert.equal(d.uri.toString(),original);}
   await v.window.showInformationMessage('API '+label+'='+result+';events='+events+';dirty='+d.isDirty);
  }catch(error){
   if(label==='closed'){assert(d.isClosed);await v.window.showInformationMessage('API closed rejected');return;}
   assert(d.isDirty);await v.window.showInformationMessage('API rejected;dirty=true');
  }
 }
 register('inactive',()=>{assert(first);assert.notStrictEqual(v.window.activeTextEditor.document,first);return save(first,'inactive');});
 register('active',()=>save(v.window.activeTextEditor.document,'active'));
 register('closed',()=>save(first,'closed'));
 register('clean',async()=>{
  const d=v.window.activeTextEditor.document,editor=v.window.activeTextEditor;
  assert.equal(d.getText(),'猫🙂 A\r\nlast\r\n');assert.equal(d.isDirty,false);
  const version=d.version,uri=d.uri.toString(),selection=editor.selection;
  for(let i=0;i<2;i++){
   const before=events,generation=d._snapshot.savedGeneration||0;
   assert.equal(await d.save(),true);assert.equal(events,before+1);
   assert.strictEqual(v.window.activeTextEditor.document,d);assert.equal(d.getText(),'猫🙂 A\r\nlast\r\n');
   assert.equal(d.version,version);assert.equal(d.uri.toString(),uri);assert.equal(d.isDirty,false);
   assert.equal(d._snapshot.savedGeneration,generation+1);
   assert.deepEqual(v.window.activeTextEditor.selection,selection);
  }
  await v.window.showInformationMessage('API clean twice;events='+events+';identity=true');
 });
 register('verify',()=>{
  const d=v.window.activeTextEditor.document;assert.strictEqual(d,first);
  assert.equal(d.getText(),'seed 猫🙂 A\r\nlast\r\n');assert(d.isDirty);
  return v.window.showInformationMessage('API clean saves retained real Redo');
 });
 register('other',async()=>{
  const d=await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath),'b.txt'));
  const editor=await v.window.showTextDocument(d,{preview:false});
  assert.strictEqual(editor.document,d);assert.strictEqual(v.window.activeTextEditor.document,d);
  await v.window.showInformationMessage('API opened other shared document');
 });
};
""", encoding='utf-8')
    config = root / 'config'
    config.mkdir()
    (config / 'settings.json').write_text(json.dumps({
        'vscli.languageServer.enabled': False, 'breadcrumbs.enabled': False,
        'files.autoSave': 'off', 'editor.formatOnSave': False, 'editor.codeActionsOnSave': {},
    }), encoding='utf-8')
    node = shutil.which('node')
    if not node:
        raise AssertionError('Actual optional-host gateway qualification requires Node')
    return extension, config, str(Path(node).resolve())


def editor(root, fixture_paths, file):
    extension, config, node = fixture_paths
    app = Editor(root, '--extension', extension, '--extension-node', node,
                 '--config-dir', config, '--no-session', file, enhanced=True,
                 extra_env={'PATH': ''})
    wait_screen(app, 'Extension ready:', 'a.txt')
    return app


def persisted(app, path, expected):
    def ready():
        app.read()
        return path.exists() and path.read_bytes() == expected.encode('utf-8')
    eventually(ready, timeout=8)


def repeated_clean(root):
    prepared = fixture(root)
    a, b = root / 'a.txt', root / 'b.txt'
    a.write_bytes(A.encode()); b.write_bytes(B.encode())
    app = editor(root, prepared, a)
    # Seed a real Undo/Redo history before clean persistence. Native Undo
    # restores the exact clean revision, leaving a genuine Redo entry.
    app.paste('seed ')
    app.send(CTRL_Z)
    wait_screen(app, 'a.txt', absent=('a.txt *',))
    command(app, 'Document repeated clean save')
    wait_screen(app, 'API clean twice;events=2;identity=true')
    assert a.read_bytes() == A.encode()
    assert b.read_bytes() == B.encode()
    # Enhanced original Ctrl+Shift+Z must still restore the seed edit.
    app.send(b'\x1b[122;6u')
    command(app, 'Document verify clean redo')
    wait_screen(app, 'API clean saves retained real Redo')
    assert a.read_bytes() == A.encode(), 'Redo unexpectedly persisted text'
    command(app, 'Document save active')
    wait_screen(app, 'API active=true;events=3;dirty=false')
    persisted(app, a, 'seed ' + A)
    app.send(CTRL_Z)
    command(app, 'Document save active')
    wait_screen(app, 'API active=true;events=4;dirty=false')
    persisted(app, a, A)
    app.finish()
    print('PASS: two clean public saves each advance real save generation/didSave, preserve shared identity/caret/CRLF bytes and original Undo/Redo')


def inactive(root):
    prepared = fixture(root)
    a, b = root / 'a.txt', root / 'b.txt'
    a.write_bytes(A.encode()); b.write_bytes(B.encode())
    app = editor(root, prepared, a)
    app.paste('dirty ')
    command(app, 'Document open other')
    wait_screen(app, 'API opened other shared document', 'b.txt  |')
    app.paste('b-dirty ')
    command(app, 'Document save inactive')
    wait_screen(app, 'API inactive=true;events=1;dirty=false', 'b.txt')
    persisted(app, a, 'dirty ' + A)
    assert b.read_bytes() == B.encode(), 'Inactive save retargeted dirty B'
    app.send(CTRL_Z)
    command(app, 'Document save active')
    wait_screen(app, 'API active=true;events=2;dirty=false', 'b.txt')
    persisted(app, b, B)
    app.finish()
    print('PASS: public inactive save preserves shared source identity, dirty other editor/Undo, Unicode CRLF bytes and didSave-before-true')


def untitled(root):
    prepared = fixture(root)
    a, b = root / 'a.txt', root / 'b.txt'
    a.write_bytes(A.encode()); b.write_bytes(B.encode())
    app = editor(root, prepared, a)
    command(app, 'File: New Text File')
    wait_screen(app, 'Untitled')
    app.paste('猫🙂 new')
    command(app, 'Document save active')
    wait_screen(app, 'Save As (existing files are protected)')
    app.send(b'\x1b')
    wait_screen(app, 'API active=false;events=0;dirty=true', 'Untitled')
    assert a.read_bytes() == A.encode()
    assert not (root / 'untitled.txt').exists()
    command(app, 'Document save active')
    wait_screen(app, 'Save As (existing files are protected)')
    target = root / 'saved-new.txt'
    app.send(CTRL_A);app.paste(str(target));app.send(b'\r')
    wait_screen(app, 'API active=true;events=1;dirty=false', 'saved-new.txt')
    persisted(app, target, '猫🙂 new')
    app.send(CTRL_Z)
    command(app, 'Document save active')
    wait_screen(app, 'API active=true;events=2;dirty=false')
    persisted(app, target, '')
    app.finish()
    print('PASS: public untitled Save As native Cancel=false and acceptance=true preserve identity, didSave, Unicode text and original Undo')


def external_and_closed(root):
    prepared = fixture(root)
    a, b = root / 'a.txt', root / 'b.txt'
    a.write_bytes(A.encode()); b.write_bytes(B.encode())
    app = editor(root, prepared, a)
    app.paste('dirty ')
    foreign = 'foreign 外🙂\r\n'
    a.write_bytes(foreign.encode())
    command(app, 'Document save active')
    wait_screen(app, 'API rejected;dirty=true')
    assert a.read_bytes() == foreign.encode()
    # Save As owns native persistence and restores the original edit history.
    app.send(b'\x1b[115;6u')
    wait_screen(app, 'Save As (existing files are protected)')
    target = root / 'preserved.txt'
    app.send(CTRL_A);app.paste(str(target));app.send(b'\r')
    persisted(app, target, 'dirty ' + A)
    wait_screen(app, 'Saved', 'preserved.txt')
    app.send(b'\x17')
    wait_screen(app, 'No open editors')
    command(app, 'Document save closed original')
    wait_screen(app, 'API closed rejected')
    assert a.read_bytes() == foreign.encode()
    assert target.read_bytes() == ('dirty ' + A).encode()
    app.finish()
    print('PASS: public save conflict rejects without foreign overwrite; native Save As preserves dirty work, closed save rejects with no fake acknowledgement')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='vscli-document-save-api-') as directory:
        root = Path(directory).resolve()
        try:
            for name, run in [('clean', repeated_clean), ('inactive', inactive), ('untitled', untitled), ('external', external_and_closed)]:
                child = root / name;child.mkdir();run(child)
        finally:
            failed = sys.exc_info()[0] is not None
            errors = []
            for app in LIVE[:]:
                try:
                    if failed:
                        stop_surface_process(app)
                        assert app.process.restored, 'Document-save interruption leaked terminal mode'
                        app.close_fds()
                        LIVE.remove(app)
                    else:
                        app.finish()
                except Exception as error: errors.append(error)
            if errors and not failed: raise errors[0]
