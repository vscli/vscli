const v=require('vscode'),assert=require('node:assert/strict');
exports.activate=context=>{
 const first=v.window.activeTextEditor,original=first.document,ordinary='猫🙂 A\r\nlast\r\n',dirty='dirty '+ordinary;
 const root=v.Uri.file(v.workspace.rootPath),visibleEvents=[],columnEvents=[];
 let shared,otherEditor,other,pending,movement=false,closeVisible,closeColumns,emptyVisible;
 const register=(id,fn)=>context.subscriptions.push(v.commands.registerCommand('editorInventory.'+id,fn));
 const points=selections=>selections.map(s=>[s.anchor.line,s.anchor.character,s.active.line,s.active.character]);
 context.subscriptions.push(v.window.onDidChangeVisibleTextEditors(editors=>visibleEvents.push(editors.slice())));
 context.subscriptions.push(v.window.onDidChangeTextEditorViewColumn(event=>columnEvents.push({editor:event.textEditor,column:event.viewColumn})));
 context.subscriptions.push(v.window.onDidChangeTextEditorSelection(event=>{
  if(movement&&event.textEditor===first){
   movement=false;
   try{assert.deepEqual(points(event.selections),[[1,4,1,4]]);assert.strictEqual(v.window.activeTextEditor,first);visible([first,otherEditor]);state(dirty);void v.window.showInformationMessage('API inventory native movement;pane=first;event=true');}
   catch(error){void v.window.showErrorMessage('Editor inventory movement failed: '+String(error.message).slice(0,4096));}
  }
  if(!pending||event.textEditor!==pending.editor)return;
  const waiter=pending;pending=undefined;
  try{assert.deepEqual(points(event.selections),waiter.expected);assert.deepEqual(points(event.textEditor.selections),waiter.expected);waiter.resolve();}
  catch(error){waiter.reject(error);}
 }));
 function visible(expected){
  const current=v.window.visibleTextEditors;assert.deepEqual(current,expected);
  for(const editor of current)assert(v.workspace.textDocuments.includes(editor.document));
 }
 function published(before,expected){
  assert(visibleEvents.length>before,'real visible-list event precedes show acknowledgement');
  assert.deepEqual(visibleEvents.at(-1),expected);visible(expected);
 }
 function state(text){assert.equal(original.getText(),text);assert.equal(original.isDirty,text===dirty);assert(v.workspace.textDocuments.includes(original));}
 function assign(editor,selection){
  assert.equal(pending,undefined);let resolve,reject;const done=new Promise((yes,no)=>{resolve=yes;reject=no;});
  pending={editor,expected:points([selection]),resolve,reject};
  try{editor.selection=selection;assert.deepEqual(points(editor.selections),points([selection]));}
  catch(error){pending=undefined;throw error;}
  return done;
 }
 register('shared',async()=>{
  state(dirty);visible([first]);assert.equal(first.viewColumn,1);
  const before=visibleEvents.length,originalPoints=points(first.selections);
  shared=await v.window.showTextDocument(original,{preview:false,viewColumn:v.ViewColumn.Beside,preserveFocus:true});
  assert.notStrictEqual(shared,first);assert.strictEqual(shared.document,original);assert.strictEqual(v.window.activeTextEditor,first);
  assert.equal(shared.viewColumn,2);assert.equal(first.viewColumn,1);published(before,[first,shared]);
  assert.deepEqual(points(first.selections),originalPoints);
  await assign(shared,new v.Selection(1,4,1,1));
  assert.deepEqual(points(first.selections),originalPoints);assert.strictEqual(v.window.activeTextEditor,first);visible([first,shared]);state(dirty);
  await v.window.showInformationMessage('API inventory shared;editors=distinct;document=shared;focus=first');
 });
 register('focusLanguage',async()=>{
  visible([first,shared]);const before=visibleEvents.length;
  await v.languages.setTextDocumentLanguage(original,'python');
  assert.equal(first.document.languageId,'python');assert.strictEqual(shared.document,original);
  assert.strictEqual(await v.window.showTextDocument(original,{preview:false,viewColumn:2}),shared);assert.strictEqual(v.window.activeTextEditor,shared);
  assert.strictEqual(await v.window.showTextDocument(original,{preview:false,viewColumn:1}),first);assert.strictEqual(v.window.activeTextEditor,first);
  assert.equal(visibleEvents.length,before,'language-only and focus changes must not invent visible-list events');
  visible([first,shared]);state(dirty);
  await v.window.showInformationMessage('API inventory focus/language;stable=true;visibleEvents=quiet');
 });
 register('replace',async()=>{
  const before=visibleEvents.length;
  other=await v.workspace.openTextDocument(v.Uri.joinPath(root,'b.txt'));
  otherEditor=await v.window.showTextDocument(other,2,false);
  assert.strictEqual(otherEditor.document,other);assert.strictEqual(v.window.activeTextEditor,otherEditor);assert.equal(otherEditor.viewColumn,2);
  published(before,[first,otherEditor]);assert(!v.window.visibleTextEditors.includes(shared));state(dirty);
  await v.window.showInformationMessage('API inventory replaced;first=retained;second=other');
 });
 register('inactive',async()=>{
  visible([first,otherEditor]);const otherPoints=points(otherEditor.selections),before=visibleEvents.length;
  await assign(first,new v.Selection(1,4,1,1));
  assert.strictEqual(v.window.activeTextEditor,otherEditor);assert.deepEqual(points(otherEditor.selections),otherPoints);
  assert.equal(other.getText(),'other\r\n');assert.equal(other.isDirty,false);assert.equal(visibleEvents.length,before);state(dirty);
  await v.window.showInformationMessage('API inventory inactive;exactPane=true;focus=other');
 });
 register('retired',async()=>{
  const before=points(otherEditor.selections),firstPoints=points(first.selections);
  assert.throws(()=>{shared.selection=new v.Selection(0,0,0,1);},'removed public editor cannot mutate a replacement pane');
  assert.deepEqual(points(otherEditor.selections),before);assert.deepEqual(points(first.selections),firstPoints);visible([first,otherEditor]);state(dirty);
  await v.window.showInformationMessage('API inventory retired;replacement=untouched');
 });
 register('original',async()=>{
  const before=visibleEvents.length;
  const shown=await v.window.showTextDocument(original,{preview:false,viewColumn:1});assert.strictEqual(shown,first);assert.strictEqual(v.window.activeTextEditor,first);
  visible([first,otherEditor]);assert.equal(visibleEvents.length,before);state(dirty);
  await v.window.showInformationMessage('API inventory original;identity=true;dirty=true');
 });
 register('armMovement',()=>{assert.strictEqual(v.window.activeTextEditor,first);assert.deepEqual(points(first.selections),[[1,4,1,1]]);movement=true;return v.window.showInformationMessage('API inventory native movement armed');});
 register('clean',()=>{assert.strictEqual(v.window.activeTextEditor,first);state(ordinary);closeVisible=visibleEvents.length;closeColumns=columnEvents.length;return v.window.showInformationMessage('API inventory undo;clean=true');});
 register('dirty',()=>{assert.strictEqual(v.window.activeTextEditor,first);state(dirty);return v.window.showInformationMessage('API inventory redo;dirty=true');});
 register('columns',async()=>{
  assert.notEqual(closeVisible,undefined,'clean verification captures pre-close event counters');
  assert.strictEqual(v.window.activeTextEditor,otherEditor);published(closeVisible,[otherEditor]);assert.equal(otherEditor.viewColumn,1);
  assert(columnEvents.slice(closeColumns).some(e=>e.editor===otherEditor&&e.column===1),'surviving editor column change event before command receipt');
  assert.equal(other.getText(),'other\r\n');assert.equal(other.isDirty,false);
  emptyVisible=visibleEvents.length;
  await v.window.showInformationMessage('API inventory close group;survivor=stable;column=1;event=true');
 });
 register('empty',async()=>{
  assert.notEqual(emptyVisible,undefined,'closed-group verification captures pre-close-all event counter');
  assert.equal(v.window.activeTextEditor,undefined);published(emptyVisible,[]);
  await v.window.showInformationMessage('API inventory empty;visible=0;active=undefined');
 });
};
