'use strict';
const v=require('vscode'),assert=require('node:assert/strict'),path=require('node:path'),fs=require('node:fs');
exports.activate=context=>{
 const document=v.window.activeTextEditor.document,original='猫🙂 original\r\n',dirty='dirty '+original;
 let watcher,seen=0,atDispose,target,phase='created';
 function identity(text=dirty){assert.strictEqual(v.window.activeTextEditor.document,document);assert(v.workspace.textDocuments.includes(document));assert.equal(document.getText(),text);assert.equal(document.isDirty,text===dirty);}
 function register(id,callback){context.subscriptions.push(v.commands.registerCommand('watch.'+id,callback));}
 register('arm',()=>{
  identity();watcher=v.workspace.createFileSystemWatcher('**/watch-*.rs');context.subscriptions.push(watcher);
  for(const [name,event] of [['created',watcher.onDidCreate],['changed',watcher.onDidChange],['deleted',watcher.onDidDelete]])
   context.subscriptions.push(event(uri=>{
    identity();assert.equal(uri.scheme,'file');assert.equal(path.dirname(uri.fsPath),v.workspace.rootPath);assert(++seen<=512);
    if(!target&&name==='created')target=uri.toString();
    // Keep one actual created resource's receipt visible; callbacks for all
    // other paths still run identity checks and contribute to disposal evidence.
    if(uri.toString()!==target||name!==phase)return;
    if(name==='changed'&&fs.readFileSync(uri.fsPath,'utf8')!=='🙂 changed\r\n')return;
    if(name==='deleted'&&fs.existsSync(uri.fsPath))return;
    return v.window.showInformationMessage('API '+name+': '+path.basename(uri.fsPath));
   }));
  return v.window.showInformationMessage('API watching files');
 });
 register('change',()=>{identity();assert(target);phase='changed';return v.window.showInformationMessage('API waiting changed');});
 register('delete',()=>{identity();assert(target);phase='deleted';return v.window.showInformationMessage('API waiting deleted');});
 register('dispose',()=>{identity();watcher.dispose();atDispose=seen;return v.window.showInformationMessage('API watcher disposed');});
 register('verify',()=>{identity();assert.equal(seen,atDispose);return v.window.showInformationMessage('API disposed watcher stayed silent');});
 register('clean',()=>{identity(original);return v.window.showInformationMessage('API watcher retained native Undo');});
};
