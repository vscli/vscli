"use strict";
const assert=require('node:assert/strict'),v=require('vscode');
exports.activate=context=>{
 const register=(name,callback)=>context.subscriptions.push(v.commands.registerCommand('fixture.configuration.'+name,callback));
 register('roundtrip',async()=>{
  const config=v.workspace.getConfiguration('fixture'),events=[];
  const listener=v.workspace.onDidChangeConfiguration(event=>{if(event.affectsConfiguration('fixture.value'))events.push(v.workspace.getConfiguration('fixture').get('value'));});
  try {
   const before=config.get('value'),value={enabled:false,list:['猫🙂',0,null]};
   assert.equal(await config.update('value',value,v.ConfigurationTarget.Global),undefined);
   assert.deepEqual(config.inspect('value').globalValue,value);assert.deepEqual(events.at(-1),value);
   assert.deepEqual(config.get('value'),before);
   assert.equal(await config.update('value',undefined,true),undefined);
   assert.equal(config.inspect('value').globalValue,undefined);assert.equal(events.at(-1),'default');
   assert.equal(await config.update('value',null,true),undefined);
   assert.equal(config.inspect('value').globalValue,null);assert.equal(events.at(-1),null);
   assert.equal(await config.update('value',['workspace','λ🙂']),undefined);
   assert.deepEqual(config.inspect('value').workspaceValue,['workspace','λ🙂']);assert.deepEqual(events.at(-1),['workspace','λ🙂']);
   assert.equal(events.length,4);
  } finally {listener.dispose();}
  return v.window.showInformationMessage('Configuration roundtrip persisted');
 });
 register('language',async()=>{
  const editor=v.window.activeTextEditor;assert(editor);const scope={uri:editor.document.uri,languageId:'cpp'};
  const config=v.workspace.getConfiguration('fixture',scope);
  assert.equal(config.inspect('value').workspaceLanguageValue,false);
  await config.update('value',{language:'猫🙂'},v.ConfigurationTarget.WorkspaceFolder);
  assert.deepEqual(config.inspect('value').workspaceLanguageValue,{language:'猫🙂'});
  await config.update('value','root',false,false);
  assert.equal(config.inspect('value').workspaceValue,'root');
  assert.deepEqual(config.inspect('value').workspaceLanguageValue,{language:'猫🙂'});
  await config.update('value',undefined,2,true);
  assert.equal(config.inspect('value').workspaceLanguageValue,undefined);
  await v.workspace.getConfiguration('editor',scope).update('tabSize',3,3,true);
  assert.equal(v.workspace.getConfiguration('editor',scope).get('tabSize'),3);
  assert.equal(editor.options.tabSize,3);
  return v.window.showInformationMessage('Language configuration persisted');
 });
 register('clean',async path=>{
  const document=typeof path==='string' ? await v.workspace.openTextDocument(v.Uri.file(path)) : v.window.activeTextEditor.document;
  const before=document.getText(),version=document.version,events=[];
  assert(before.includes('"before"'));assert.equal(document.isDirty,false);
  const listener=v.workspace.onDidChangeTextDocument(event=>{if(event.document===document)events.push(document.getText());});
  try {
   assert.equal(await v.workspace.getConfiguration('fixture').update('value','after-clean',true),undefined);
   assert(document.getText().includes('"after-clean"'));assert(document.version>version);
   assert.equal(document.isDirty,false);assert(events.some(text=>text.includes('"after-clean"')));
   assert.equal(v.workspace.getConfiguration('fixture').inspect('value').globalValue,'after-clean');
  } finally {listener.dispose();}
  return v.window.showInformationMessage('Open configuration refreshed');
 });
 register('race',async()=>{
  await assert.rejects(v.workspace.getConfiguration('fixture').update('value','after-clean',true),/Setting persisted;.*Unsaved settings edits retained/);
  return v.window.showInformationMessage('Late configuration edits preserved');
 });
 register('refused',async()=>{
  await assert.rejects(v.workspace.getConfiguration('fixture').update('value','forbidden',true),/unsaved changes/);
  return v.window.showInformationMessage('Dirty configuration preserved');
 });
};
