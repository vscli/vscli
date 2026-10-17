const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const original=v.window.activeTextEditor.document;
 const ordinary='猫🙂 A\r\nlast\r\n',dirty='dirty '+ordinary;
 const initial='猫🙂 generated\r\nint first = 1;\r\n';
 const associatedInput='猫🙂 associated\nint associated = 2;\n';
 const associatedText=document=>associatedInput.replace(/\n/g,document.eol===v.EndOfLine.CRLF?'\r\n':'\n');
 const root=v.Uri.file(v.workspace.rootPath);
 let options,associated,late,closedOptions,eventFailure,changes=0;
 const opened=new Set(),saved=[],lifecycle=[];
 const register=(id,callback)=>context.subscriptions.push(v.commands.registerCommand('untitled.'+id,callback));
 context.subscriptions.push(v.workspace.onDidOpenTextDocument(document=>{opened.add(document);lifecycle.push(['open',document]);}));
 context.subscriptions.push(v.workspace.onDidCloseTextDocument(document=>{
  try{assert.equal(document.isClosed,true);assert.equal(document.isDirty,false);assert(!v.workspace.textDocuments.includes(document));lifecycle.push(['close',document]);}
  catch(error){eventFailure=String(error.message).slice(0,4096);}
 }));
 context.subscriptions.push(v.workspace.onDidChangeTextDocument(event=>{if(event.document===options)changes++;}));
 context.subscriptions.push(v.workspace.onDidSaveTextDocument(document=>{
  try{assert(v.workspace.textDocuments.includes(document));assert.equal(document.uri.scheme,'file');assert.equal(document.isUntitled,false);assert.equal(document.isDirty,false);saved.push(document);lifecycle.push(['save',document]);}
  catch(error){eventFailure=String(error.message).slice(0,4096);}
 }));
 function retained(document,text,untitled){
  assert(v.workspace.textDocuments.includes(document));assert.equal(document.getText(),text);
  assert.equal(document.isUntitled,untitled);assert.equal(document.isDirty,untitled);
 }
 function active(document,text,untitled){retained(document,text,untitled);assert.strictEqual(v.window.activeTextEditor.document,document);}
 async function shown(document){const editor=await v.window.showTextDocument(document,{preview:false});assert.strictEqual(editor.document,document);return editor;}
 function originalIdentity(text){
  assert.strictEqual(v.window.activeTextEditor.document,original);assert(v.workspace.textDocuments.includes(original));
  assert.equal(original.getText(),text);assert.equal(original.isDirty,text===dirty);
 }
 function associatedUri(name){return v.Uri.joinPath(root,name).with({scheme:'untitled'});}
 register('options',async()=>{
  originalIdentity(dirty);
  options=await v.workspace.openTextDocument({language:'cpp',content:initial});
  assert(opened.has(options),'didOpen must publish shared identity before open acknowledgement');
  retained(options,initial,true);assert.equal(options.uri.scheme,'untitled');assert.equal(options.languageId,'cpp');
  assert.strictEqual(v.window.activeTextEditor.document,original);
  assert(v.window.visibleTextEditors.every(editor=>editor.document!==options),'initial open is hidden');
  const plain=await v.workspace.openTextDocument(),empty=await v.workspace.openTextDocument({});
  for(const document of [plain,empty]){assert(opened.has(document));retained(document,'',true);assert.equal(document.uri.scheme,'untitled');}
  assert.notStrictEqual(plain,empty);assert.notStrictEqual(options,plain);assert.notEqual(plain.uri.toString(),empty.uri.toString());
  await shown(options);active(options,initial,true);
  await v.window.showInformationMessage('API untitled options;hidden=true;fresh=true;CRLF=true');
 });
 register('edit',async()=>{
  active(options,initial,true);const version=options.version,before=changes;
  assert.equal(await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'edit ')),true);
  active(options,'edit '+initial,true);assert(options.version>version);assert(changes>before,'change mirror/event precedes edit acknowledgement');
  await v.window.showInformationMessage('API untitled edit=true;event=true');
 });
 register('initial',()=>{active(options,initial,true);return v.window.showInformationMessage('API untitled initial retained;dirty=true');});
 register('undo',async()=>{
  active(options,'edit '+initial,true);await v.commands.executeCommand('undo');active(options,initial,true);
  await v.window.showInformationMessage('API untitled public undo;initial=true;dirty=true');
 });
 async function savedResource(old,text,filename,before,eventsBefore){
  const oldUri=old.uri.toString();
  const next=v.workspace.textDocuments.find(document=>document.uri.toString()===v.Uri.joinPath(root,filename).toString());
  assert(next,'new file document is published before save resolves');assert.notStrictEqual(next,old);
  assert.equal(old.uri.toString(),oldUri);assert.equal(old.uri.scheme,'untitled');
  assert.equal(old.isClosed,true);assert.equal(old.isDirty,false);assert.equal(old.getText(),text);
  assert(!v.workspace.textDocuments.includes(old));active(next,text,false);
  assert.equal(saved.length,before+1);assert.strictEqual(saved[before],next);
  assert.deepEqual(lifecycle.slice(eventsBefore),[['close',old],['open',next],['save',next]],'close/open/save all precede true');
  await assert.rejects(()=>old.save(),'closed former untitled document cannot be saved again');
  return next;
 }
 register('save',async()=>{
  active(options,initial,true);const old=options,before=saved.length,uri=old.uri.toString(),eventsBefore=lifecycle.length;
  const result=await old.save();assert.equal(eventFailure,undefined);
  if(result){
   options=await savedResource(old,initial,'options-saved.cpp',before,eventsBefore);closedOptions=old;
   assert.equal(old.uri.toString(),uri);
   await v.window.showInformationMessage('API untitled saved;transition=true;events=close/open/save');
  }else{
   active(old,initial,true);assert.equal(old.uri.toString(),uri);assert.equal(saved.length,before);assert.equal(lifecycle.length,eventsBefore);
   await v.window.showInformationMessage('API untitled cancelled;dirty=true;event=false');
  }
 });
 register('editSaved',async()=>{
  active(options,initial,false);assert.equal(closedOptions.isClosed,true);
  assert.equal(await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'file ')),true);
  assert.strictEqual(v.window.activeTextEditor.document,options);assert.equal(options.getText(),'file '+initial);assert(options.isDirty);
  assert.equal(closedOptions.getText(),initial);assert.equal(closedOptions.uri.scheme,'untitled');assert.equal(closedOptions.isDirty,false);
  await v.window.showInformationMessage('API new file edited;dirty=true;oldClosed=true');
 });
 register('cleanSaved',()=>{
  active(options,initial,false);assert.equal(closedOptions.isClosed,true);assert.equal(closedOptions.getText(),initial);
  return v.window.showInformationMessage('API new file undo;clean=true;oldClosed=true');
 });
 register('dirtySaved',()=>{
  assert.strictEqual(v.window.activeTextEditor.document,options);assert.equal(options.getText(),'file '+initial);assert(options.isDirty);
  assert.equal(closedOptions.isClosed,true);assert.equal(closedOptions.isDirty,false);assert.equal(closedOptions.getText(),initial);
  return v.window.showInformationMessage('API new file redo;dirty=true;oldClosed=true');
 });
 register('associated',async()=>{
  originalIdentity(dirty);const uri=associatedUri('new.cpp');
  associated=await v.workspace.openTextDocument(uri);assert(opened.has(associated));retained(associated,'',true);
  assert.equal(associated.languageId,'cpp');assert.equal(associated.uri.toString(),uri.toString());
  assert.strictEqual(v.window.activeTextEditor.document,original);const editor=await shown(associated);
  assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),associatedInput)),true);
  active(associated,associatedText(associated),true);
  const again=await v.workspace.openTextDocument(uri);assert.strictEqual(again,associated);active(associated,associatedText(associated),true);
  await v.window.showInformationMessage('API associated untitled;cached=true;cpp=true');
 });
 register('saveAssociated',async()=>{
  active(associated,associatedText(associated),true);const old=associated,before=saved.length,eventsBefore=lifecycle.length,uri=old.uri.toString();
  assert.equal(await old.save(),true);assert.equal(eventFailure,undefined);
  associated=await savedResource(old,associatedText(old),'new.cpp',before,eventsBefore);assert.equal(old.uri.toString(),uri);
  await v.window.showInformationMessage('API associated saved;direct=true;transition=true;events=close/open/save');
 });
 register('existing',async()=>{
  originalIdentity(dirty);const uri=associatedUri('exists.cpp'),before=v.workspace.textDocuments.length;
  let rejected=false;try{await v.workspace.openTextDocument(uri);}catch(error){rejected=true;}
  assert(rejected,'Existing associated target must refuse admission');assert.equal(v.workspace.textDocuments.length,before);
  assert(!v.workspace.textDocuments.some(document=>document.uri.toString()===uri.toString()));originalIdentity(dirty);
  await v.window.showInformationMessage('API associated existing rejected;original=true');
 });
 register('late',async()=>{
  originalIdentity(dirty);late=await v.workspace.openTextDocument(associatedUri('late.cpp'));assert(opened.has(late));retained(late,'',true);
  const editor=await shown(late);assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),associatedInput)),true);
  active(late,associatedText(late),true);await v.window.showInformationMessage('API associated late target ready;dirty=true');
 });
 register('protected',async()=>{
  active(late,associatedText(late),true);const before=saved.length,uri=late.uri.toString(),version=late.version;
  let refused=false;try{refused=(await late.save())===false;}catch(error){refused=true;}
  assert(refused,'Target appeared after open; save must never overwrite it');active(late,associatedText(late),true);
  assert.equal(late.uri.toString(),uri);assert.equal(late.version,version);assert.equal(saved.length,before);
  await v.window.showInformationMessage('API associated late save refused;dirty=true;event=false');
 });
 register('original',async()=>{await shown(original);originalIdentity(dirty);await v.window.showInformationMessage('API untitled original restored;dirty=true;identity=true');});
 register('clean',()=>{originalIdentity(ordinary);return v.window.showInformationMessage('API untitled original clean history retained');});
 register('dirty',()=>{originalIdentity(dirty);return v.window.showInformationMessage('API untitled original dirty history retained');});
};
