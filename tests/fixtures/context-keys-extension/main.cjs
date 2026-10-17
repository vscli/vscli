const v=require('vscode'),assert=require('node:assert/strict');
const moduleContext=v.commands.executeCommand('setContext','fixture.module',true);
exports.activate=async context=>{
 await moduleContext;
 for(const [key,value]of Object.entries({'fixture.enabled':true,'fixture.zero':0,'fixture.null':null,'editorTextFocus':false}))await v.commands.executeCommand('setContext',key,value);
 await v.commands.executeCommand('setContext','fixture.uris',[v.Uri.file('/tmp/context λ')]);
 context.subscriptions.push(v.commands.registerCommand('fixture.context.edit',async()=>{
  require('node:fs').appendFileSync(require('node:path').join(v.workspace.rootPath,'context-entered'),'edit\n');
  assert.equal(await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'context ')),true);
  assert.equal(await v.commands.executeCommand('setContext','fixture.enabled',false),undefined);
  await v.window.showInformationMessage('Context edit applied and shortcut disabled');
 }),v.commands.registerCommand('fixture.context.enable',async()=>{
  await v.commands.executeCommand('setContext','fixture.enabled',true);
  await v.commands.executeCommand('setContext','fixture.removed',true);
  await v.commands.executeCommand('setContext','fixture.removed');
  await v.window.showInformationMessage('Context shortcut enabled');
 }),v.commands.registerCommand('fixture.context.invalid',async()=>{
  await assert.rejects(v.commands.executeCommand('setContext','fixture.enabled','x'.repeat(65536)));
  await v.window.showInformationMessage('Invalid context refused');
 }));
};
