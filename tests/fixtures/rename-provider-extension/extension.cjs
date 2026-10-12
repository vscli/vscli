'use strict';
const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const source=v.window.activeTextEditor.document,root=v.Uri.file(v.workspace.rootPath);
 const original='猫🙂 source\r\nnext\r\n',targetOriginal='α🙂 target\r\n',hiddenOriginal='γ🙂 source\r\n',unopenedOriginal='β🙂 source\r\n';
 const pos=new v.Position(0,11),word=new v.Range(0,10,0,16),records=[],registrations=[];
 let mode='single',target,hidden,unopened,lastProvided,holdPrepare=false,holdEdit=false,held,capacity=[],capacityReleases=[],saves=0;
 const uri=name=>v.Uri.joinPath(root,name),info=text=>v.window.showInformationMessage(text),points=editor=>editor.selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 const register=(name,fn)=>context.subscriptions.push(v.commands.registerCommand('renameFixture.'+name,fn));
 context.subscriptions.push(v.workspace.onDidSaveTextDocument(()=>{saves++;}));
 function retained(){assert.equal(source.getText(),'dirty '+original);assert(source.isDirty);assert(v.workspace.textDocuments.includes(source));}
 function record(role,phase,document,position,token,newName){assert.strictEqual(document,source);assert(position instanceof v.Position);assert.equal(token.isCancellationRequested,false);assert(records.length<256,'bounded callback evidence');records.push({role,phase,newName});}
 function hold(phase,token,result){let release,cancel;const cancellation=new Promise(resolve=>{cancel=resolve;});const promise=new Promise(resolve=>{release=resolve;});assert.equal(held,undefined);held={phase,cancellation,release:()=>release(result),cancelled:false,finished:false};token.onCancellationRequested(()=>{held.cancelled=true;cancel();void info('API rename held '+phase+';cancelled=true;actualPending=true');});void info('API rename held '+phase+';entered=true;actualPending=true');return promise.then(value=>{held.finished=true;return value;});}
 function editValue(newName){
  const edit=new v.WorkspaceEdit();edit.replace(source.uri,word,newName);
  if(mode==='native'){edit.replace(target.uri,new v.Range(0,10,0,16),newName);edit.replace(hidden.uri,new v.Range(0,10,0,16),newName);edit.replace(uri('unopened.cpp'),new v.Range(0,4,0,10),newName);}
  return edit;
 }
 function richValue(newName){
  const edit=new v.WorkspaceEdit(),metadata={label:'original annotation',needsConfirmation:true,description:'query-only'};
  edit.replace(source.uri,word,newName,metadata);
  edit.createFile(uri('created.cpp'),{overwrite:false,contents:new Uint8Array([65,10])},metadata);
  const snippet=v.SnippetTextEdit.insert(new v.Position(1,0),new v.SnippetString('${1:猫}-$1'));snippet.keepWhitespace=true;
  edit.set(source.uri,[[snippet,metadata]]);
  edit.renameFile(uri('old.cpp'),uri('new.cpp'),{overwrite:false},metadata);
  edit.replace(uri('hidden.cpp'),new v.Range(0,0,0,1),'H',metadata);
  edit.deleteFile(uri('deleted.cpp'),{recursive:false,ignoreIfNotExists:true},metadata);
  return edit;
 }
 function install(){
  for(const role of [2,1,0]){
   const provider={provideRenameEdits(document,position,newName,token){assert.strictEqual(this,provider);record(role,'edit',document,position,token,newName);
    if(mode==='fallthrough'&&role===0)return null;if(mode==='fallthrough'&&role===1)throw new Error('typed edit rejection');
    const value=mode==='brand'?{}:mode==='empty'||mode==='capacity'?new v.WorkspaceEdit():mode==='full'?richValue(newName):editValue(newName);lastProvided=value;
    if(role===0&&holdEdit){holdEdit=false;return hold('edit',token,value);}return value;
   }};
   if(role!==2)provider.prepareRename=function(document,position,token){assert.strictEqual(this,provider);record(role,'prepare',document,position,token);
    if(mode==='reject')throw role===0?'first reason':new Error('second reason');if(mode==='generic'&&role===0)throw {opaque:true};
    if(role===0&&['index','noprepare','emptyplaceholder','defaultBehavior','badline'].includes(mode)){
     if(mode==='emptyplaceholder')return{range:word,placeholder:''};if(mode==='defaultBehavior')return{defaultBehavior:true};if(mode==='badline')return new v.Range(1,0,1,4);return null;
    }
    if(role===1)return mode==='index'?{range:word,placeholder:'prepared by second'}:null;
    const result=mode==='lineonly'?new v.Range(0,0,0,1):word;
    if(mode==='capacity')return new Promise(resolve=>{capacityReleases.push(()=>resolve(result));if(capacityReleases.length===8)void info('API rename capacity;actualCallbacks=8;held=true');});
    if(holdPrepare){holdPrepare=false;return hold('prepare',token,result);}return result;
   };
   const registration=v.languages.registerRenameProvider(role===0?'cpp':'*',provider);registrations.push({role,registration});context.subscriptions.push(registration);
  }
 }
 install();
 async function prepare(){return v.commands.executeCommand('vscode.prepareRename',source.uri,pos);}
 async function execute(name){return v.commands.executeCommand('vscode.executeDocumentRenameProvider',source.uri,pos,name);}
 const callOrder=begin=>records.slice(begin).map(row=>row.phase+row.role);
 register('queries',async()=>{
  retained();const inactivePeer=await v.workspace.openTextDocument(uri('target.cpp'));
  const active=await v.window.showTextDocument(inactivePeer,{preview:false,viewColumn:v.ViewColumn.Beside});
  const before=points(active),version=source.version,peerVersion=inactivePeer.version;
  mode='single';let begin=records.length,location=await prepare();assert(location.range instanceof v.Range);assert.equal(location.placeholder,'source');assert.deepEqual([location.range.start.character,location.range.end.character],[10,16]);
  const emptyName=await execute('');assert(emptyName instanceof v.WorkspaceEdit);assert.notStrictEqual(emptyName,lastProvided);assert.equal(emptyName.get(source.uri)[0].newText,'');assert.deepEqual(callOrder(begin),['prepare0','prepare0','edit0']);assert.equal(records.at(-1).newName,'');
  mode='index';begin=records.length;location=await prepare();assert.equal(location.placeholder,'prepared by second');await execute('index');assert.deepEqual(callOrder(begin),['prepare0','prepare1','prepare0','prepare1','edit1']);
  mode='fallthrough';begin=records.length;await execute('fallback');assert.deepEqual(callOrder(begin),['prepare0','edit0','edit1','edit2']);
  for(const next of ['noprepare','emptyplaceholder','defaultBehavior','badline']){mode=next;begin=records.length;location=await prepare();assert.equal(location.placeholder,'source');await execute('word');assert.deepEqual(callOrder(begin),['prepare0','prepare1','prepare0','prepare1','edit0']);}
  mode='lineonly';location=await prepare();assert.equal(location.placeholder,'d');assert.deepEqual([location.range.start.character,location.range.end.character],[0,1]);
  mode='reject';await assert.rejects(prepare(),/first reason\nsecond reason/);await assert.rejects(execute('reject'),/first reason\nsecond reason/);
  mode='generic';await assert.rejects(prepare());
  for(const next of ['brand','empty']){mode=next;begin=records.length;const value=await execute('empty');assert(value instanceof v.WorkspaceEdit);assert.equal(value.size,0);assert.deepEqual(callOrder(begin),['prepare0','edit0']);}
  mode='noprepare';const getText=source.getText,getWord=source.getWordRangeAtPosition;source.getText=()=> 'forged word';source.getWordRangeAtPosition=()=>new v.Range(0,0,0,1);
  try{location=await prepare();assert.equal(location.placeholder,'source','fallback must use private captured text');assert.deepEqual([location.range.start.character,location.range.end.character],[10,16]);}finally{source.getText=getText;source.getWordRangeAtPosition=getWord;}
  mode='full';const full=await execute('full');assert(full instanceof v.WorkspaceEdit);assert.notStrictEqual(full,lastProvided);assert.deepEqual(full.entries().map(([resource])=>resource.toString()),[source.uri.toString(),uri('hidden.cpp').toString()]);assert.equal(full.get(source.uri).length,1);assert.equal(full.get(source.uri)[0].newText,'full');assert.equal(full.get(source.uri)[0].newEol,undefined);assert.equal(full.get(source.uri)[0].metadata,undefined);assert.equal(await v.workspace.applyEdit(full,{isRefactoring:false}),false,'whole unsupported mixed edit must refuse');
  for(const item of registrations)item.registration.dispose();location=await prepare();assert.equal(location.placeholder,'source');const none=await execute('no providers');assert(none instanceof v.WorkspaceEdit);assert.equal(none.size,0);const noWord=await v.commands.executeCommand('vscode.prepareRename',source.uri,new v.Position(2,0));assert.equal(noWord.placeholder,'');assert(noWord.range.isEmpty);
  assert.strictEqual(v.window.activeTextEditor,active);assert.deepEqual(points(active),before);assert.equal(source.version,version);assert.equal(inactivePeer.version,peerVersion);assert.equal(inactivePeer.getText(),targetOriginal);retained();assert.equal(saves,0);await info('API rename queries;prepare/fallback/index/empty/fullValue=true;noMutation=true');
 });
 register('prepareTargets',async()=>{
  retained();target=await v.workspace.openTextDocument(uri('target.cpp'));hidden=await v.workspace.openTextDocument(uri('hidden.cpp'));
  const editor=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside});assert.equal(await editor.edit(builder=>builder.insert(new v.Position(0,0),'dirty ')),true);
  const shared=await v.window.showTextDocument(target,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});assert.notStrictEqual(shared,editor);assert.strictEqual(shared.document,target);
  const edit=new v.WorkspaceEdit();edit.insert(hidden.uri,new v.Position(0,0),'dirty ');assert.equal(await v.workspace.applyEdit(edit,{isRefactoring:false}),true);assert(hidden.isDirty);
  await v.window.showTextDocument(source,{preview:false,viewColumn:1});mode='native';retained();assert.equal(target.getText(),'dirty '+targetOriginal);assert.equal(hidden.getText(),'dirty '+hiddenOriginal);await info('API rename targets;shared/hiddenDirty=true;unopenedUntouched=true');
 });
 register('source',async()=>{const editor=await v.window.showTextDocument(source,{preview:false,viewColumn:1});assert.strictEqual(editor.document,source);await info('API rename source;identity=true');});
 async function verify(renamed,label){
  const expected=renamed?'renamed':'source';assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),'dirty 猫🙂 '+expected+'\r\nnext\r\n');assert(source.isDirty);
  assert.equal(target.getText(),'dirty α🙂 '+(renamed?'renamed':'target')+'\r\n');assert.equal(hidden.getText(),'dirty γ🙂 '+expected+'\r\n');
  const value=await v.workspace.openTextDocument(uri('unopened.cpp'));if(unopened)assert.strictEqual(value,unopened);else unopened=value;assert.equal(unopened.getText(),'β🙂 '+expected+'\r\n');assert.equal(unopened.isDirty,renamed);assert(target.isDirty);assert(hidden.isDirty);assert.equal(saves,0);await info('API rename '+label+';grouped/source/shared/hidden/unopened=true;didSave=0');
 }
 register('verifyApplied',()=>verify(true,'applied'));register('verifyUndo',()=>verify(false,'Undo'));register('verifyRedo',()=>verify(true,'Redo'));
 register('armPrepare',()=>{retained();mode='single';holdPrepare=true;return info('API rename held prepare armed');});
 register('armEdit',()=>{retained();mode='single';holdEdit=true;return info('API rename held edit armed');});
 register('release',async()=>{assert(held);await held.cancellation;assert(held.cancelled);held.release();await new Promise(resolve=>setImmediate(resolve));assert(held.finished);const phase=held.phase;held=undefined;return info('API rename held '+phase+';cancelled/actualSettled=true');});
 register('dispose',async()=>{registrations.find(item=>item.role===0).registration.dispose();assert(held);await held.cancellation;assert(held.cancelled);return info('API rename provider disposed;source=retained');});
 register('heldRedo',()=>{assert.equal(source.getText(),'dirty X'+original);assert(source.isDirty);return info('API rename held Redo;intentionalEdit/lateReply=false');});
 register('sourceClean',()=>{assert.equal(source.getText(),original);assert.equal(source.isDirty,false);return info('API rename source Undo;exact=true');});
 register('sourceDirty',()=>{retained();return info('API rename source Redo;exact=true');});
 register('capacity',()=>{retained();mode='capacity';for(let i=0;i<8;i++){const promise=execute('capacity');promise.catch(()=>{});capacity.push(promise);}});
 register('capacityOverflow',async()=>{assert.equal(capacityReleases.length,8);await assert.rejects(execute('ninth'));assert.equal(capacityReleases.length,8);return info('API rename capacity;ninth=refused;actualStill8=true');});
 register('capacityRelease',async()=>{capacityReleases.splice(0).forEach(release=>release());const values=await Promise.all(capacity);assert.equal(values.length,8);assert(values.every(value=>value instanceof v.WorkspaceEdit&&value.size===0));mode='single';const fresh=await execute('fresh');assert.equal(fresh.get(source.uri)[0].newText,'fresh');retained();assert.equal(saves,0);return info('API rename capacity;originals/fresh=true;actualReleased=true');});
 register('aliasApply',async()=>{retained();const edit=new v.WorkspaceEdit();edit.replace(uri('source-alias.cpp'),word,'aliased');assert.equal(await v.workspace.applyEdit(edit),true);assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),'dirty 猫🙂 aliased\r\nnext\r\n');assert(source.isDirty);assert.equal(saves,0);return info('API rename alias;originalDirtyIdentity/mirrorBeforeAck=true');});
 register('verifySingle',()=>{assert.strictEqual(v.window.activeTextEditor.document,source);assert.equal(source.getText(),'dirty 猫🙂 renamed\r\nnext\r\n');assert(source.isDirty);assert.equal(saves,0);return info('API rename single;identity/dirty=true;defaultPolicy/didSave=0');});
};
