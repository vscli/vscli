#!/usr/bin/env python3
"""Native Explorer operations publish real public file events with shared dirty data intact."""
import json
from pathlib import Path
import shutil
import tempfile
from extension_sessions_pty import command
from pty_smoke import Editor, CTRL_A, CTRL_S, CTRL_Z, eventually, wait_screen


def run():
    with tempfile.TemporaryDirectory(prefix='vscli-file-events-pty-') as directory:
        base=Path(directory).resolve();root=base/'workspace';root.mkdir()
        extension=base/'extension';extension.mkdir();log=base/'events.jsonl'
        source=root/'a.cpp';moved=root/'b.cpp';created=root/'created.cpp'
        original='original 猫🙂\r\n';source.write_bytes(original.encode())
        (extension/'package.json').write_text(json.dumps({'publisher':'fixture','name':'file-events','version':'1.0.0','main':'main.cjs','engines':{'vscode':'*'},'activationEvents':['*']}))
        (extension/'main.cjs').write_text("""
const vscode=require('vscode'),fs=require('node:fs'),assert=require('node:assert/strict');
exports.activate=context=>{
 const original=vscode.workspace.textDocuments[0];
 const record=(kind,event)=>{
  assert.ok(Object.isFrozen(event));
  if(kind==='rename')assert.equal(original.isClosed,true);
  fs.appendFileSync(LOG,JSON.stringify({kind,files:event.files.map(file=>file.oldUri?{old:file.oldUri.fsPath,new:file.newUri.fsPath}:file.fsPath)})+'\\n');
 };
 context.subscriptions.push(vscode.workspace.onDidCreateFiles(event=>record('create',event)),vscode.workspace.onDidRenameFiles(event=>record('rename',event)),vscode.workspace.onDidDeleteFiles(event=>record('delete',event)));
};
""".replace('LOG',json.dumps(str(log))))
        node=shutil.which('node');assert node,'File operation extension qualification requires Node'
        trash=base/'data';trash.mkdir()
        app=Editor(root,'--extension',extension,'--extension-node',str(Path(node).resolve()),'--no-session',source,enhanced=True,extra_env={'XDG_DATA_HOME':str(trash),'PATH':''})
        completed=False
        def records():return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        try:
            wait_screen(app,'Extension ready:','a.cpp')
            app.paste('dirty λ ');wait_screen(app,'a.cpp *','dirty λ original')
            command(app,'Explorer: New File');app.send(CTRL_A);app.paste(str(created));app.send(b'\r')
            eventually(lambda:app.read() and len(records())==1)
            assert records()[0]=={'kind':'create','files':[str(created)]}
            command(app,'Explorer: Rename Selected Item');app.send(CTRL_A);app.paste(str(moved));app.send(b'\r')
            eventually(lambda:app.read() and len(records())==2)
            assert records()[1]=={'kind':'rename','files':[{'old':str(source),'new':str(moved)}]}
            assert moved.read_bytes()==original.encode()
            command(app,'Explorer: Move Selected Item to Trash');wait_screen(app,'Trash');app.send(b'y')
            eventually(lambda:app.read() and len(records())==3)
            assert records()[2]=={'kind':'delete','files':[str(moved)]};assert not moved.exists()
            assert created.exists()
            app.finish(discard=True);completed=True
        finally:
            if not completed:app.finish(discard=True)
    print('PASS: real native Explorer create/rename/trash → public file events, URI order, dirty bytes, isolated trash and terminal restoration')


if __name__=='__main__':run()
