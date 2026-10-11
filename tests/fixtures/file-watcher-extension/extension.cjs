'use strict';
const v=require('vscode'),assert=require('node:assert/strict'),path=require('node:path');
exports.activate=context=>{
 const document=v.window.activeTextEditor.document,original='猫🙂 original\r\n',dirty='dirty '+original;
 let watcher,seen=0,atDispose;
 function identity(text=dirty){assert.strictEqual(v.window.activeTextEditor.document,document);assert(v.workspace.textDocuments.includes(document));assert.equal(document.getText(),text);assert.equal(document.isDirty,text===dirty);}
 function register(id,callback){context.subscriptions.push(v.commands.registerCommand('watch.'+id,callback));}
 register('arm',()=>{
  identity();watcher=v.workspace.createFileSystemWatcher('**/watch-*.rs');context.subscriptions.push(watcher);
  for(const [name,event] of [['created',watcher.onDidCreate],['changed',watcher.onDidChange],['deleted',watcher.onDidDelete]])
   context.subscriptions.push(event(uri=>{identity();assert.equal(uri.scheme,'file');assert.equal(path.dirname(uri.fsPath),v.workspace.rootPath);seen++;return v.window.showInformationMessage('API '+name+': '+path.basename(uri.fsPath));}));
  return v.window.showInformationMessage('API watching files');
 });
 register('dispose',()=>{identity();watcher.dispose();atDispose=seen;return v.window.showInformationMessage('API watcher disposed');});
 register('verify',()=>{identity();assert.equal(seen,atDispose);return v.window.showInformationMessage('API disposed watcher stayed silent');});
 register('clean',()=>{identity(original);return v.window.showInformationMessage('API watcher retained native Undo');});
};
