const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const original=v.window.activeTextEditor.document;
 const ordinary='猫🙂 A\r\nlast\r\n',dirty='dirty '+ordinary;
 const uri=v.Uri.parse('preview-test:/sample.cpp');
 const first='// 猫🙂\r\nint first = 1;\rnext\r\n';
 const firstNormalized='// 猫🙂\nint first = 1;\nnext\n';
 const refreshed='// β🙂\r\nint refreshed = 2;\r\n';
 const refreshedNormalized='// β🙂\nint refreshed = 2;\n';
 const latest='// latest 猫🙂\r\nint latest = 3;\r\n';
 const latestNormalized='// latest 猫🙂\nint latest = 3;\n';
 let content=first,document,opened=[],changeWait,heldRelease,heldStarted;
 let waitHeld,held=false,disposed=false,providerCalls=0,callbackFailure;
 const emitter=new v.EventEmitter();
 const registration=v.workspace.registerTextDocumentContentProvider('preview-test',{
  onDidChange:emitter.event,
  provideTextDocumentContent(input,token){
   assert.equal(input.toString(),uri.toString());providerCalls++;
   if(held){
    held=false;return new Promise(resolve=>{
     heldRelease=()=>resolve('// obsolete 猫🙂\r\nint obsolete = 99;\r\n');
     if(heldStarted){heldStarted();heldStarted=undefined;}
    });
   }
   return content;
  }
 });
 context.subscriptions.push(emitter,registration);
 const register=(id,callback)=>context.subscriptions.push(v.commands.registerCommand('virtual.'+id,callback));
 function originalIdentity(text){
  assert.strictEqual(v.window.activeTextEditor.document,original);
  assert(v.workspace.textDocuments.includes(original));assert.equal(original.getText(),text);
  assert.equal(original.isDirty,text===dirty);
 }
 function virtualIdentity(text){
  assert.strictEqual(v.window.activeTextEditor.document,document);
  assert(v.workspace.textDocuments.includes(document));assert.equal(document.uri.toString(),uri.toString());
  assert.equal(document.getText(),text);assert.equal(document.isDirty,false);assert.equal(document.isUntitled,false);
 }
 context.subscriptions.push(v.workspace.onDidOpenTextDocument(d=>{
  if(d.uri.toString()===uri.toString())opened.push(d);
 }));
 context.subscriptions.push(v.workspace.onDidChangeTextDocument(event=>{
  if(event.document!==document)return;
  try{
   assert(v.workspace.textDocuments.includes(document));assert.equal(document.isDirty,false);
   if(changeWait&&document.getText()===changeWait.text){const resolve=changeWait.resolve;changeWait=undefined;resolve();}
  }catch(error){callbackFailure=String(error.message).slice(0,4096);if(changeWait){changeWait.reject(error);changeWait=undefined;}}
 }));
 function changed(text){assert.equal(changeWait,undefined);return new Promise((resolve,reject)=>{changeWait={text,resolve,reject};});}
 register('open',async()=>{
  originalIdentity(dirty);
  document=await v.workspace.openTextDocument(uri);
  assert.equal(document.getText(),firstNormalized);assert.equal(document.isUntitled,false);
  assert.deepEqual(opened,[document],'didOpen publishes the same mirror before open acknowledgement');
  assert(v.workspace.textDocuments.includes(document));assert.strictEqual(v.window.activeTextEditor.document,original);
  const again=await v.workspace.openTextDocument(uri);assert.strictEqual(again,document);assert.equal(providerCalls,1);
  const editor=await v.window.showTextDocument(document,{preview:false});
  assert.strictEqual(editor.document,document);virtualIdentity(firstNormalized);
  await v.window.showInformationMessage('API virtual opened;identity=true;LF=true');
 });
 register('readonly',async()=>{
  const text=firstNormalized,version=document.version,editor=v.window.activeTextEditor;
  virtualIdentity(text);assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),'forbidden ')),false);
  assert.equal(await editor.edit(()=>{}),false);assert.equal(await document.save(),false);
  virtualIdentity(text);assert.equal(document.version,version);
  await v.window.showInformationMessage('API virtual readonly;edit=false;save=false');
 });
 register('refresh',async()=>{
  const version=document.version,identity=document;
  const done=changed(refreshedNormalized);content=refreshed;emitter.fire(uri);await done;
  assert.strictEqual(document,identity);virtualIdentity(refreshedNormalized);assert(document.version>version);
  assert.equal(callbackFailure,undefined);
  await v.window.showInformationMessage('API virtual refreshed;identity=true;event=true');
 });
 register('hold',async()=>{
  virtualIdentity(refreshedNormalized);held=true;
  waitHeld=new Promise(resolve=>{heldStarted=resolve;});emitter.fire(uri);await waitHeld;
  virtualIdentity(refreshedNormalized);
  await v.window.showInformationMessage('API virtual obsolete callback held');
 });
 register('latest',async()=>{
  assert.equal(typeof heldRelease,'function');
  const version=document.version,done=changed(latestNormalized);
  content=latest;emitter.fire(uri);const release=heldRelease;heldRelease=undefined;release();await done;
  virtualIdentity(latestNormalized);assert(document.version>version);assert.equal(callbackFailure,undefined);
  await v.window.showInformationMessage('API virtual latest wins;identity=true');
 });
 register('dispose',async()=>{
  virtualIdentity(latestNormalized);const version=document.version,calls=providerCalls;
  registration.dispose();disposed=true;
  // Already-open cached mirrors remain usable without invoking a disposed provider.
  const again=await v.workspace.openTextDocument(uri);assert.strictEqual(again,document);
  await v.window.showTextDocument(again,{preview:false});virtualIdentity(latestNormalized);
  assert.equal(document.version,version);assert.equal(providerCalls,calls);assert(disposed);
  await v.window.showInformationMessage('API virtual disposed;cached=true;readonly=true');
 });
 register('original',async()=>{
  const editor=await v.window.showTextDocument(original,{preview:false});assert.strictEqual(editor.document,original);
  originalIdentity(dirty);await v.window.showInformationMessage('API original restored;dirty=true;identity=true');
 });
 register('dirty',()=>{originalIdentity(dirty);return v.window.showInformationMessage('API original dirty history retained');});
 register('clean',()=>{originalIdentity(ordinary);return v.window.showInformationMessage('API original clean history retained');});
};
