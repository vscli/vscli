const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const originalEditor=v.window.activeTextEditor,original=originalEditor.document;
 const initial='猫🙂 A\r\nlast\r\n',dirty='dirty '+initial;
 const root=v.Uri.file(v.workspace.rootPath),lifecycle=[];
 let selectionEvents=0,pending,movement=false,eventFailure,options;
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('editorState.'+id,fn));
 const points=selections=>selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 context.subscriptions.push(v.workspace.onDidCloseTextDocument(document=>lifecycle.push({kind:'close',document,language:document.languageId,closed:document.isClosed,dirty:document.isDirty})));
 context.subscriptions.push(v.workspace.onDidOpenTextDocument(document=>lifecycle.push({kind:'open',document,language:document.languageId,closed:document.isClosed,dirty:document.isDirty})));
 context.subscriptions.push(v.workspace.onDidSaveTextDocument(document=>lifecycle.push({kind:'save',document,language:document.languageId,closed:document.isClosed,dirty:document.isDirty})));
 context.subscriptions.push(v.window.onDidChangeTextEditorSelection(event=>{
  selectionEvents++;
  if(pending&&event.textEditor===pending.editor){
   const waiter=pending;pending=undefined;
   try{
    assert.strictEqual(event.textEditor.document,waiter.editor.document);
    assert.deepEqual(points(event.selections),waiter.expected);
    assert.deepEqual(points(waiter.editor.selections),waiter.expected,'authoritative getter matches native event');
    waiter.resolve(event);
   }catch(error){waiter.reject(error);}
  }
  if(movement&&event.textEditor===originalEditor){
   movement=false;
   try{
    assert.deepEqual(points(event.selections),[[1,4,1,4]]);
    originalState(dirty);assert.equal(eventFailure,undefined);
    void v.window.showInformationMessage('API native movement event;identity=true');
   }catch(error){eventFailure=String(error.message).slice(0,4096);void v.window.showErrorMessage('Editor state movement failed: '+eventFailure);}
  }
 }));
 function originalState(text){
  assert.strictEqual(v.window.activeTextEditor.document,original);assert(v.workspace.textDocuments.includes(original));
  assert.equal(original.getText(),text);assert.equal(original.isDirty,text===dirty);
 }
 function assign(editor,selections,expected,single=false){
  assert.equal(pending,undefined);
  let resolve,reject;const done=new Promise((yes,no)=>{resolve=yes;reject=no;});
  pending={editor,expected,resolve,reject};const before=selectionEvents;
  try{
   if(single)editor.selection=selections[0];else editor.selections=selections;
   assert.deepEqual(points(editor.selections),selections.length?points(selections):[[0,0,0,0]],'setter getter is synchronous');
   assert.equal(selectionEvents,before,'setter does not invent synchronous selection events');
  }catch(error){pending=undefined;throw error;}
  return done;
 }
 async function switchLanguage(document,language){
  const old=document.languageId,version=document.version,text=document.getText(),isDirty=document.isDirty,start=lifecycle.length;
  const result=await v.languages.setTextDocumentLanguage(document,language);
  assert.strictEqual(result,document);assert.equal(document.version,version);assert.equal(document.getText(),text);assert.equal(document.isDirty,isDirty);
  const events=lifecycle.slice(start);
  if(old===language){assert.deepEqual(events,[]);return;}
  assert.equal(events.length,2);assert.deepEqual(events.map(e=>[e.kind,e.language,e.closed,e.dirty]),[['close',old,false,isDirty],['open',language,false,isDirty]]);
  for(const event of events)assert.strictEqual(event.document,document);
  assert.equal(document.languageId,language);assert(v.workspace.textDocuments.includes(document));
 }
 register('language',async()=>{
  originalState(dirty);assert.equal(original.languageId,'cpp');
  await switchLanguage(original,'python');await switchLanguage(original,'python');
  const before=lifecycle.length;
  await assert.rejects(()=>v.languages.setTextDocumentLanguage(original,'fixture-not-registered'));
  assert.equal(original.languageId,'python');assert.equal(lifecycle.length,before);
  await switchLanguage(original,'fixture-notes');await switchLanguage(original,'python');
  assert.strictEqual(v.window.activeTextEditor,originalEditor);originalState(dirty);
  await v.window.showInformationMessage('API language lifecycle;identity=true;registered=true;quiet=true');
 });
 register('selections',async()=>{
  originalState(dirty);
  await assign(originalEditor,[new v.Selection(0,9,0,6),new v.Selection(1,0,1,4)],[[0,9,0,6],[1,0,1,4]]);
  const before=points(originalEditor.selections);
  assert.throws(()=>{originalEditor.selections=[{}];});
  assert.deepEqual(points(originalEditor.selections),before);
  assert.throws(()=>{originalEditor.selections=Array.from({length:10001},()=>new v.Selection(0,0,0,0));});
  assert.deepEqual(points(originalEditor.selections),before);originalState(dirty);
  await v.window.showInformationMessage('API selections;reverse=true;multiple=true;invalid=refused');
 });
 register('clamp',async()=>{
  originalState(dirty);
  await assign(originalEditor,[new v.Selection(999,999,0,999)],[[2,0,0,11]],true);
  await assign(originalEditor,[new v.Selection(0,8,0,8)],[[0,7,0,7]],true);
  await assign(originalEditor,[],[[0,0,0,0]]);originalState(dirty);
  await v.window.showInformationMessage('API selections clamped;UTF16=true;empty=origin');
 });
 register('inactive',async()=>{
  originalState(dirty);
  const other=await v.workspace.openTextDocument(v.Uri.joinPath(root,'b.txt'));
  const otherEditor=await v.window.showTextDocument(other,{preview:false,viewColumn:-2}),before=points(otherEditor.selections),version=other.version;
  await assign(originalEditor,[new v.Selection(1,4,1,1)],[[1,4,1,1]],true);
  assert.strictEqual(v.window.activeTextEditor,otherEditor);assert.strictEqual(otherEditor.document,other);
  assert.deepEqual(points(otherEditor.selections),before);assert.equal(other.version,version);assert.equal(other.getText(),'other\r\n');assert.equal(other.isDirty,false);
  assert.equal(original.getText(),dirty);assert.equal(original.isDirty,true);
  await v.window.showInformationMessage('API inactive selections;focus=other;identity=true');
 });
 register('armMovement',async()=>{
  assert.deepEqual(points(originalEditor.selections),[[1,4,1,1]]);
  const editor=await v.window.showTextDocument(original,{preview:false,viewColumn:1});assert.strictEqual(editor,originalEditor);originalState(dirty);
  movement=true;await v.window.showInformationMessage('API native movement armed');
 });
 register('original',async()=>{const editor=await v.window.showTextDocument(original,{preview:false,viewColumn:1});assert.strictEqual(editor,originalEditor);originalState(dirty);await v.window.showInformationMessage('API editor state original;identity=true;dirty=true');});
 register('clean',()=>{originalState(initial);return v.window.showInformationMessage('API editor state original undo;clean=true');});
 register('dirty',()=>{originalState(dirty);return v.window.showInformationMessage('API editor state original redo;dirty=true');});
 register('prepareSave',async()=>{
  originalState(dirty);options=await v.workspace.openTextDocument({language:'cpp',content:initial});
  await v.window.showTextDocument(options,{preview:false,viewColumn:-2});await switchLanguage(options,'python');
  assert.equal(options.isUntitled,true);assert.equal(options.isDirty,true);
  await v.window.showInformationMessage('API language save prepared;python=true;CRLF=true');
 });
 register('save',async()=>{
  const old=options,start=lifecycle.length;assert.strictEqual(v.window.activeTextEditor.document,old);
  assert.equal(await old.save(),true);
  const next=v.workspace.textDocuments.find(d=>d.uri.toString()===v.Uri.joinPath(root,'state-saved.cpp').toString());
  assert(next);assert.notStrictEqual(next,old);assert.equal(old.isClosed,true);assert.equal(old.isDirty,false);assert.equal(old.uri.scheme,'untitled');assert.equal(old.getText(),initial);
  assert.strictEqual(v.window.activeTextEditor.document,next);assert.equal(next.languageId,'python');assert.equal(next.getText(),initial);assert.equal(next.isDirty,false);
  const events=lifecycle.slice(start);assert.deepEqual(events.map(e=>[e.kind,e.language,e.closed,e.dirty]),[['close','python',true,false],['open','python',false,false],['save','python',false,false]]);
  assert.strictEqual(events[0].document,old);assert.strictEqual(events[1].document,next);assert.strictEqual(events[2].document,next);options=next;
  await v.window.showInformationMessage('API language saved;python=true;events=close/open/save');
 });
 register('cleanSaved',()=>{
  assert.strictEqual(v.window.activeTextEditor.document,options);assert.equal(options.languageId,'python');assert.equal(options.getText(),initial);assert.equal(options.isDirty,false);
  return v.window.showInformationMessage('API saved language undo;python=true;clean=true');
 });
 register('editSaved',async()=>{
  assert.equal(options.languageId,'python');assert.equal(await v.window.activeTextEditor.edit(edit=>edit.insert(new v.Position(0,0),'file ')),true);
  assert.equal(options.getText(),'file '+initial);assert.equal(options.isDirty,true);assert.equal(options.languageId,'python');
  await v.window.showInformationMessage('API saved language edited;python=true;dirty=true');
 });
};
