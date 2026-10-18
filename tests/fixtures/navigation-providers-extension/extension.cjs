'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const source=v.window.activeTextEditor.document,root=v.Uri.file(v.workspace.rootPath);
 const original='猫🙂 source\r\nnext\r\n',dirty='dirty '+original,oneText='α🙂 targetA\r\n';
 const kinds=['Definition','Reference','Implementation','TypeDefinition','Declaration'];
 const callbacks=[],registrations=[];
 let targetOne,virtual,holding=false,held=[],heldFinished=0,heldCancelled=false,heldCancellation,holdDocument,capacityPromises=[],capacityEntered=0,capacityReleases=[],isolation=false;
 const uri=name=>v.Uri.joinPath(root,name),key=value=>value.targetUri?value.targetUri.toString():value.uri.toString();
 const point=s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character];
 const selections=editor=>editor.selections.map(point);
 const info=text=>v.window.showInformationMessage(text);
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('navigationProviders.'+id,fn));
 function retained(){assert.equal(source.getText(),dirty);assert(source.isDirty);assert(v.workspace.textDocuments.includes(source));}
 function target(doc){
  if(doc===virtual)return{uri:virtual.uri,range:new v.Range(0,0,0,virtual.lineAt(0).text.length),selection:new v.Range(0,0,0,1),other:new v.Range(0,1,0,3)};
  const start=targetOne&&targetOne.getText().startsWith('dirty ')?7:1;
  return{uri:uri('one.cpp'),range:new v.Range(0,0,0,targetOne?targetOne.lineAt(0).text.length:'α🙂 targetA'.length),selection:new v.Range(0,start,0,start+2),other:new v.Range(0,start-1,0,start)};
 }
 function rows(kind,role,doc){
  const main=target(doc),origin=doc===source?new v.Range(0,6,0,9):new v.Range(0,0,0,3);
  const link=(resource,range,selection)=>({targetUri:resource,targetRange:range,targetSelectionRange:selection,originSelectionRange:origin});
  if(kind==='Reference'){
   const primary=new v.Location(main.uri,main.selection);
   return role===0?[primary]:role===1?[new v.Location(uri('two.cpp'),new v.Range(0,1,0,3)),primary]:[new v.Location(uri('three.cpp'),new v.Range(0,1,0,3))];
  }
  return role===0?[link(main.uri,main.range,main.selection)]:role===1?[new v.Location(uri('two.cpp'),new v.Range(0,1,0,3)),link(main.uri,main.range,main.other)]:[link(uri('three.cpp'),new v.Range(0,0,0,'γ🙂 targetC'.length),new v.Range(0,1,0,3))];
 }
 function invoke(kind,role,receiver,provider,doc,position,token,referenceContext){
  assert.strictEqual(receiver,provider);assert(v.workspace.textDocuments.includes(doc));assert(position instanceof v.Position);assert.equal(token.isCancellationRequested,false);
  if(kind==='Reference')assert.equal(referenceContext.includeDeclaration,true);
  assert(callbacks.length<256,'bounded callback fixture evidence');callbacks.push({kind,role,uri:doc.uri.toString(),language:doc.languageId,version:doc.version,line:position.line,character:position.character});
  if(doc===holdDocument){
   assert.equal(role,2);capacityEntered++;
   return new Promise(resolve=>{capacityReleases.push(()=>resolve(rows(kind,role,doc)));if(capacityEntered===8)void info('API navigation capacity;actualCallbacks=8;held=true');});
  }
  if(holding&&kind==='Definition'&&role===0){
   holding=false;let release,resolveCancellation;const promise=new Promise(resolve=>{release=resolve;});heldCancellation=new Promise(resolve=>{resolveCancellation=resolve;});held.push(()=>release(rows(kind,role,doc)));
   token.onCancellationRequested(()=>{heldCancelled=true;resolveCancellation();void info('API navigation held;cancelled=true;actualPending=true');});
   void info('API navigation held;entered=true;source='+doc.version);
   return promise.then(value=>{heldFinished++;return value;});
  }
  if(isolation&&role===1)throw new Error('intentional isolated navigation provider failure');
  if(isolation&&role===2)return null;
  return rows(kind,role,doc);
 }
 // All five families retain the same exact three-provider source order. The
 // newest cpp provider must win the duplicate URI+FULL-range entry.
 for(const [role,selector]of [[1,'cpp'],[2,'*'],[0,'cpp']]){
  for(const kind of kinds){
   const provider={role};
   if(kind==='Reference')provider.provideReferences=function(d,p,c,t){return invoke(kind,role,this,provider,d,p,t,c);};
   else provider['provide'+kind]=function(d,p,t){return invoke(kind,role,this,provider,d,p,t);};
   const registration=v.languages['register'+kind+'Provider'](selector,provider);registrations.push({kind,role,registration});context.subscriptions.push(registration);
  }
 }
 function verifyRows(kind,results,doc){
  const mainUri=doc===virtual?virtual.uri.toString():uri('one.cpp').toString();
  const expected=[mainUri,uri('two.cpp').toString(),uri('three.cpp').toString()].sort();
  assert.deepEqual(results.map(key),expected);assert.equal(results.length,3,'whole-range duplicate removed');
  const main=results.find(value=>key(value)===mainUri),two=results.find(value=>key(value)===uri('two.cpp').toString());
  assert(two instanceof v.Location);assert.deepEqual([two.range.start.line,two.range.start.character,two.range.end.line,two.range.end.character],[0,1,0,3]);
  if(kind==='Reference'){
   assert(results.every(value=>value instanceof v.Location));const start=doc===virtual?0:targetOne?7:1;
   assert.deepEqual([main.range.start.character,main.range.end.character],[start,doc===virtual?1:start+2]);
  }else{
   assert(main.targetUri instanceof v.Uri);assert(main.targetRange instanceof v.Range);assert(main.targetSelectionRange instanceof v.Range);assert(main.originSelectionRange instanceof v.Range);
   assert.deepEqual([main.targetRange.start.line,main.targetRange.start.character,main.targetRange.end.line,main.targetRange.end.character],[0,0,0,doc===virtual?virtual.lineAt(0).text.length:targetOne?targetOne.lineAt(0).text.length:'α🙂 targetA'.length]);
   const start=doc===virtual?0:targetOne?7:1;
   assert.deepEqual([main.targetSelectionRange.start.character,main.targetSelectionRange.end.character],[start,doc===virtual?1:start+2]);
   assert.deepEqual([main.originSelectionRange.start.character,main.originSelectionRange.end.character],doc===source?[6,9]:[0,3]);
   const third=results.find(value=>key(value)===uri('three.cpp').toString());
   assert(third.targetUri instanceof v.Uri);assert.deepEqual([third.targetRange.start.character,third.targetRange.end.character],[0,'γ🙂 targetC'.length]);
   assert.deepEqual([third.targetSelectionRange.start.character,third.targetSelectionRange.end.character],[1,3]);
  }
 }
 register('source',async()=>{
  retained();const editor=await v.window.showTextDocument(source,{preview:false,viewColumn:1});assert.strictEqual(editor.document,source);
  await info('API navigation source;identity=true;dirty=true');
 });
 register('prepareTarget',async()=>{
  retained();targetOne=await v.workspace.openTextDocument(uri('one.cpp'));
  const editor=await v.window.showTextDocument(targetOne,{preview:false,viewColumn:v.ViewColumn.Beside});
  assert.equal(await editor.edit(edit=>edit.insert(new v.Position(0,0),'dirty ')),true);assert.equal(targetOne.getText(),'dirty '+oneText);assert(targetOne.isDirty);
  const shared=await v.window.showTextDocument(targetOne,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.notStrictEqual(shared,editor);assert.strictEqual(shared.document,targetOne);assert.strictEqual(v.window.activeTextEditor,editor);
  await v.window.showTextDocument(source,{preview:false,viewColumn:1});retained();
  await info('API navigation target;retained/shared=true;dirty=true');
 });
 register('target',()=>{assert.strictEqual(v.window.activeTextEditor.document,targetOne);assert.equal(targetOne.getText(),'dirty '+oneText);assert.deepEqual(selections(v.window.activeTextEditor),[[0,7,0,7]]);retained();return info('API navigation reveal;selectionStart=7;collapsed=true;identity=true');});
 register('targetUndo',()=>{assert.strictEqual(v.window.activeTextEditor.document,targetOne);assert.equal(targetOne.getText(),oneText);retained();return info('API navigation target Undo;exactSource=true');});
 register('targetRedo',()=>{assert.strictEqual(v.window.activeTextEditor.document,targetOne);assert.equal(targetOne.getText(),'dirty '+oneText);retained();return info('API navigation target Redo;exactSource=true');});
 register('queries',async()=>{
  retained();const other=await v.workspace.openTextDocument(uri('two.cpp')),editor=await v.window.showTextDocument(other,{preview:false,viewColumn:v.ViewColumn.Beside});
  const before=selections(editor),version=source.version;
  for(const kind of kinds){const begin=callbacks.length,results=await v.commands.executeCommand('vscode.execute'+kind+'Provider',source.uri,new v.Position(0,6));verifyRows(kind,results,source);assert.deepEqual(callbacks.slice(begin).map(row=>row.role),[0,1,2]);}
  assert.strictEqual(v.window.activeTextEditor,editor);assert.deepEqual(selections(editor),before);assert.equal(source.version,version);retained();
  isolation=true;try{const result=await v.commands.executeCommand('vscode.executeDefinitionProvider',source.uri,new v.Position(0,6));assert.equal(result.length,1);assert.equal(key(result[0]),uri('one.cpp').toString());}finally{isolation=false;}
  const content=v.workspace.registerTextDocumentContentProvider('navigation-test',{provideTextDocumentContent(){return '猫🙂 virtual\r\n';}});context.subscriptions.push(content);
  virtual=await v.workspace.openTextDocument(v.Uri.parse('navigation-test:/virtual.cpp'));assert.equal(virtual.languageId,'cpp');
  for(const kind of kinds){const results=await v.commands.executeCommand('vscode.execute'+kind+'Provider',virtual.uri,new v.Position(0,0));verifyRows(kind,results,virtual);}
  await v.languages.setTextDocumentLanguage(source,'python');
  const begin=callbacks.length,result=await v.commands.executeCommand('vscode.executeDefinitionProvider',source.uri,new v.Position(0,6));assert.equal(result.length,1);assert.equal(key(result[0]),uri('three.cpp').toString());assert.deepEqual(callbacks.slice(begin).map(row=>row.role),[2]);
  await v.languages.setTextDocumentLanguage(source,'cpp');assert.strictEqual(v.window.activeTextEditor,editor);assert.deepEqual(selections(editor),before);retained();
  await info('API nav queries;five/fullLinks/sort/dedup=true;inactive/virtual/language/isolation=true');
 });
 register('armHeld',()=>{retained();holding=true;heldFinished=0;heldCancelled=false;return info('API navigation held armed');});
 register('releaseHeld',async()=>{
  assert.equal(held.length,1);await heldCancellation;assert.equal(heldCancelled,true);held.splice(0).forEach(release=>release());
  // Event-loop completion barrier: actual callback microtasks and their framed
  // release receipts precede this observation; no timer/result polling.
  await new Promise(resolve=>setImmediate(resolve));assert.equal(heldFinished,1);
  await info('API navigation held released;actualCallback=true');
 });
 register('languageABA',async()=>{retained();await v.languages.setTextDocumentLanguage(source,'python');await v.languages.setTextDocumentLanguage(source,'cpp');retained();return info('API navigation language ABA;source=unchanged');});
 register('disposeHeld',async()=>{
  const item=registrations.find(item=>item.kind==='Definition'&&item.role===0);item.registration.dispose();
  await heldCancellation;assert.equal(heldCancelled,true);
  // A real native round-trip follows the registry/cancellation frames. It
  // retains the same source and never invokes another navigation target.
  const editor=await v.window.showTextDocument(source,{preview:false,viewColumn:1});assert.strictEqual(editor.document,source);retained();
  return info('API navigation registration disposed;source=unchanged');
 });
 register('capacity',async()=>{
  retained();holdDocument=await v.workspace.openTextDocument(uri('held.hold'));await v.languages.setTextDocumentLanguage(holdDocument,'nav-hold');
  capacityEntered=0;capacityPromises=[];
  for(let n=0;n<8;n++)capacityPromises.push(v.commands.executeCommand('vscode.executeDefinitionProvider',holdDocument.uri,new v.Position(0,0)));
  // The callback itself publishes the eighth-entered public witness.
  for(const promise of capacityPromises)promise.catch(()=>{});
 });
 register('capacityOverflow',async()=>{assert.equal(capacityEntered,8);await assert.rejects(()=>v.commands.executeCommand('vscode.executeDefinitionProvider',holdDocument.uri,new v.Position(0,0)));assert.equal(capacityEntered,8);retained();return info('API navigation capacity;ninth=refused;actualStill8=true');});
 register('capacityRelease',async()=>{
  assert.equal(capacityReleases.length,8);capacityReleases.splice(0).forEach(release=>release());
  const results=await Promise.all(capacityPromises);assert(results.every(result=>result.length===1&&key(result[0])===uri('three.cpp').toString()));
  holdDocument=undefined;retained();const fresh=await v.commands.executeCommand('vscode.executeDefinitionProvider',source.uri,new v.Position(0,6));verifyRows('Definition',fresh,source);
  await info('API navigation capacity;originals/fresh=true;actualReleased=true');
 });
 register('heldRedo',()=>{assert.equal(source.getText(),'dirty X'+original);assert(source.isDirty);return info('API navigation held Redo;intentionalEdit=true;lateReply=false');});
 register('originalClean',()=>{assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),original);assert.equal(source.isDirty,false);return info('API navigation original Undo;clean=true');});
 register('originalDirty',()=>{assert.strictEqual(v.window.activeTextEditor.document,source);retained();return info('API navigation original Redo;dirty=true');});
};
