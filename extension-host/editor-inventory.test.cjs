'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri,Selection,Position}=require('./api-types.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
// Stable upstream extHostDocumentsAndEditors.ts / extHostTextEditors.ts:
// https://github.com/microsoft/vscode/tree/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common
const point=character=>({line:0,character});
const caret=character=>[{anchor:point(character),active:point(character)}];
function held(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(handler){
  let runtime,generation=0,activeEditor='left';
  const docs=[{id:1,uri:Uri.file('/tmp/a.cpp').toString(),text:'猫🙂\r\nsecond',version:1,eol:2,languageId:'cpp',languageEpoch:0,isDirty:true},
    {id:2,uri:Uri.file('/tmp/b.cpp').toString(),text:'B',version:1,languageId:'cpp',languageEpoch:0,isDirty:false}];
  let inventory=[{id:'left',document:1,viewColumn:1,selections:caret(1)},{id:'right',document:1,viewColumn:2,selections:caret(3)}];
  const calls=[],messages=[];
  function sync(){runtime.sync({generation:++generation,documents:docs,editors:inventory,activeEditor,active:1,selections:caret(0)});}
  runtime=createApi((method,params,lifecycle)=>{calls.push({method,params,lifecycle});return Promise.resolve(handler?handler({method,params,lifecycle,sync}):{document:params.args?.document,editor:params.args?.editor});},(method,params)=>messages.push({method,params}),{session:1,editorSelections:true,documentLanguage:true});
  sync();const api=runtime.forExtension('owner');
  return{runtime,api,docs,calls,messages,sync,get inventory(){return inventory;},set inventory(value){inventory=value;},focus(value){activeEditor=value;sync();}};
}
test('split handles are distinct and stable, sharing document but keeping per-pane selections and owner caches',()=>{
  const f=fixture(),[left,right]=f.api.window.visibleTextEditors;
  assert.notEqual(left,right);assert.equal(left.document,right.document);assert.equal(left.selection.active.character,1);assert.equal(right.selection.active.character,3);
  assert.equal(left.viewColumn,1);assert.equal(right.viewColumn,2);assert.equal(f.api.window.activeTextEditor,left);
  const foreign=f.runtime.forExtension('another');assert.notEqual(foreign.window.visibleTextEditors[0],left);assert.equal(foreign.window.visibleTextEditors[0].document,left.document);
  f.focus('right');assert.equal(f.api.window.activeTextEditor,right);assert.equal(f.api.window.visibleTextEditors[0],left);
  f.docs[0].languageId='python';f.docs[0].languageEpoch++;f.sync();assert.equal(f.api.window.visibleTextEditors[1],right);assert.equal(right.document.languageId,'python');
});
test('all inventory and selections publish before visible then active notifications',()=>{
  const f=fixture(),events=[],old=f.api.window.activeTextEditor;
  f.api.window.onDidChangeVisibleTextEditors(editors=>{events.push('visible');assert.deepEqual(editors,f.api.window.visibleTextEditors);assert.equal(f.api.window.activeTextEditor.document,f.api.workspace.textDocuments[1]);});
  f.api.window.onDidChangeActiveTextEditor(editor=>{events.push('active');assert.equal(editor,f.api.window.visibleTextEditors[1]);assert.equal(editor.selection.active.character,1);});
  f.inventory=[f.inventory[0],{id:'b',document:2,viewColumn:2,selections:caret(1)}];f.focus('b');
  assert.deepEqual(events,['visible','active']);assert.equal(f.api.window.visibleTextEditors[0],old);
});
test('column-only, focus, language and quiet frames do not emit visible-list changes',()=>{
  const f=fixture(),[left,right]=f.api.window.visibleTextEditors,columns=[];let visible=0;
  f.api.window.onDidChangeVisibleTextEditors(()=>visible++);f.api.window.onDidChangeTextEditorViewColumn(event=>columns.push(event));
  f.focus('right');f.docs[0].languageId='python';f.docs[0].languageEpoch++;f.sync();
  f.inventory=[{...f.inventory[1],viewColumn:1},{...f.inventory[0],viewColumn:2}];f.sync();f.sync();
  assert.equal(visible,0);assert.deepEqual(columns,[{textEditor:right,viewColumn:1},{textEditor:left,viewColumn:2}]);assert.equal(right.viewColumn,1);assert.equal(left.viewColumn,2);
});
test('inactive selection setter is optimistic only for exact pane and native event never retargets active',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),[left,right]=f.api.window.visibleTextEditors,events=[];f.focus('right');
  f.api.window.onDidChangeTextEditorSelection(event=>events.push(event));left.selection=new Selection(1,2,0,1);
  assert.equal(left.selection.anchor.line,1);assert.equal(right.selection.active.character,3);assert.equal(events.length,0);
  const args=f.calls[0].params.args;assert.equal(args.editor,'left');assert.equal(args.document,1);assert.equal(args.version,1);
  f.inventory[0]={...f.inventory[0],selections:args.selections,selectionKind:3};f.sync();
  assert.equal(events.length,1);assert.equal(events[0].textEditor,left);assert.equal(events[0].kind,3);assert.equal(f.api.window.activeTextEditor,right);
  wait.resolve({document:1,editor:'left'});await tick();assert.equal(f.messages.length,0);
});
test('retired editors preserve presentation but refuse all requests even when same document/pane is reopened',async()=>{
  const f=fixture(),old=f.api.window.visibleTextEditors[1],document=old.document;
  f.inventory=[f.inventory[0]];f.sync();assert.equal(old.selection.active.character,3);assert.equal(old.viewColumn,2);
  assert.throws(()=>{old.selection=new Selection(0,0,0,0);},/no longer available/);await assert.rejects(old.edit(edit=>edit.insert(new Position(0,0),'X')),/no longer available/);
  f.inventory.push({id:'right',document:1,viewColumn:2,selections:caret(0)});f.sync();const replacement=f.api.window.visibleTextEditors[1];
  assert.notEqual(old,replacement);assert.equal(replacement.document,document);assert.equal(old.selection.active.character,3);
  assert.throws(()=>{old.selections=[];},/no longer available/);assert.equal(f.calls.length,0);
});
test('selection getter reentrancy cannot borrow replacement exact editor identity',()=>{
  const f=fixture(),old=f.api.window.visibleTextEditors[1],value=new Selection(0,0,0,1);let reads=0;
  Object.defineProperty(value,'anchor',{get(){reads++;f.inventory=[f.inventory[0]];f.sync();f.inventory.push({id:'fresh-right',document:1,viewColumn:2,selections:caret(0)});f.sync();return point(0);}});
  assert.throws(()=>{old.selection=value;},/no longer available/);assert.equal(reads,1);assert.equal(f.calls.length,0);
});
test('explicit edit carries pane ID and callback retirement rejects before sending any partial work',async()=>{
  const f=fixture(()=>({applied:true})),right=f.api.window.visibleTextEditors[1];
  assert.equal(await right.edit(edit=>edit.insert(new Position(0,1),'λ')),true);assert.equal(f.calls[0].params.editor,'right');assert.equal(f.calls[0].params.document,1);
  await assert.rejects(right.edit(edit=>{edit.insert(new Position(0,0),'lost');f.inventory=[f.inventory[0]];f.sync();}),/source changed/);assert.equal(f.calls.length,1);
});
test('public editor/document getter forgery cannot redirect selection or edit service',async()=>{
  const f=fixture(()=>({applied:true})),right=f.api.window.visibleTextEditors[1];
  Object.defineProperty(right,'document',{get(){throw Error('untrusted editor document');}});Object.defineProperty(right,'viewColumn',{get(){throw Error('untrusted editor column');}});
  const doc=f.api.workspace.textDocuments[0];doc._snapshot={id:99,version:88};right.selection=new Selection(0,1,0,1);
  assert.equal(f.calls[0].params.args.editor,'right');assert.equal(f.calls[0].params.args.document,1);
  assert.equal(await right.edit(edit=>edit.insert(new Position(0,0),'λ')),true);assert.equal(f.calls[1].params.editor,'right');assert.equal(f.calls[1].params.version,1);await tick();
});
test('showTextDocument targets exact preserveFocus editor, supports positional and option columns',async()=>{
  const f=fixture(({params})=>({document:params.args.document,editor:'right'})),doc=f.api.workspace.textDocuments[0],[left,right]=f.api.window.visibleTextEditors;
  assert.equal(await f.api.window.showTextDocument(doc,2,true),right);assert.equal(f.api.window.activeTextEditor,left);
  assert.deepEqual(f.calls[0].params.args,{document:1,version:1,selection:null,viewColumn:2,preserveFocus:true});
  assert.equal(await f.api.window.showTextDocument(doc,{viewColumn:-2,preserveFocus:true,preview:false}),right);
  assert.equal(f.calls[1].params.args.viewColumn,-2);assert.equal(await f.api.window.showTextDocument(doc),right);assert.equal(f.calls[2].params.args.viewColumn,-1);
});
test('show getter captures once and private source/caller recheck refuses before native admission',async()=>{
  for(const cause of ['source','owner']){
    const f=fixture(),doc=f.api.workspace.textDocuments[0],options={};let reads=0;
    Object.defineProperty(options,'viewColumn',{get(){reads++;if(cause==='source'){f.docs[0].version++;f.sync();}else{f.runtime.disposeOwner('owner');f.runtime.forExtension('owner');}return 2;}});
    await assert.rejects(f.api.window.showTextDocument(doc,options),/source changed|retired/);assert.equal(reads,1);assert.equal(f.calls.length,0);
  }
});
test('wrong document/editor show ACK and retired selected frame cannot manufacture success',async()=>{
  for(const reply of [{document:1,editor:'missing'},{document:2,editor:'right'},{document:1}]){
    const f=fixture(()=>reply);await assert.rejects(f.api.window.showTextDocument(f.api.workspace.textDocuments[0],2),/mirror.*before/);
  }
  const wait=held(),f=fixture(()=>wait.promise),doc=f.api.workspace.textDocuments[0],promise=f.api.window.showTextDocument(doc,2);
  f.inventory=[f.inventory[0]];f.sync();wait.resolve({document:1,editor:'right'});await assert.rejects(promise,/mirror.*before/);
});
test('preview and unsupported columns/invalid options refuse before opening input path',async()=>{
  const f=fixture();for(const options of [{preview:true},{viewColumn:5},{viewColumn:1.5},{preserveFocus:1},null])await assert.rejects(f.api.window.showTextDocument('/tmp/not-opened',options),/Preview|viewColumn|boolean|options/);
  assert.equal(f.calls.length,0);
});
test('malformed inventory rejects atomically before documents or generation change',()=>{
  const f=fixture(),old=f.api.window.activeTextEditor,doc=old.document;
  for(const patch of [{editors:Array(5).fill(f.inventory[0]),activeEditor:'left'},{editors:[...f.inventory,f.inventory[0]],activeEditor:'left'},{editors:f.inventory,activeEditor:'unknown'},
    {editors:[{...f.inventory[0],document:99}],activeEditor:'left'},{editors:[{...f.inventory[0],viewColumn:5}],activeEditor:'left'}]){
    assert.throws(()=>f.runtime.sync({generation:100,documents:[{...f.docs[0],text:'invalid',version:2}],...patch}),/Invalid native/);
    assert.equal(doc.getText(),'猫🙂\r\nsecond');assert.equal(f.api.window.activeTextEditor,old);
  }
  f.docs[0].text='valid';f.docs[0].version=2;f.sync();assert.equal(doc.getText(),'valid');
});
test('visible removal publishes empty active and old editor remains readable',()=>{
  const f=fixture(),old=f.api.window.activeTextEditor,events=[];f.api.window.onDidChangeVisibleTextEditors(value=>{events.push(['visible',value]);assert.equal(f.api.window.activeTextEditor,undefined);});f.api.window.onDidChangeActiveTextEditor(value=>events.push(['active',value]));
  f.inventory=[];f.focus(null);assert.deepEqual(events,[['visible',[]],['active',undefined]]);assert.equal(old.document.getText(),'猫🙂\r\nsecond');assert.equal(old.selection.active.character,1);
});
test('explicit typed selection/show occupancy survives timeout and wrong-pane/malformed terminal receipts',async()=>{
  for(const method of ['nativeEditorSelections','nativeDocumentShow']){
    const frames=[],timers=[];let terminals=0;
    const queue=createNativeRequests(frame=>frames.push(frame),()=>undefined,{setTimeout(fn){timers.push(fn);return fn;},clearTimeout(){}}),lifecycle={onTerminal(){terminals++;}};
    const pending=queue.request(method,{args:{document:1,editor:method==='nativeEditorSelections'?'left':undefined}},lifecycle),rejected=assert.rejects(pending,/outcome is unknown/);
    assert.equal(lifecycle.retained,true);timers[0]();await rejected;
    for(const result of [{document:1,editor:0},{document:1,editor:'left',other:0},{document:1,editor:''}])assert.equal(queue.settle({id:frames[0].id,result}),false);
    if(method==='nativeEditorSelections'){assert.equal(queue.settle({id:frames[0].id,result:{document:1}}),false);assert.equal(queue.settle({id:frames[0].id,result:{document:1,editor:'wrong'}}),false);}
    assert.equal(terminals,0);assert.equal(queue.settle({id:frames[0].id,result:{document:1,editor:'left'}}),true);assert.equal(terminals,1);
  }
});

test('scoped editor original owner lease cannot revive after owner disposal and reacquisition',async()=>{
  const f=fixture(()=>({applied:true})),old=f.api.window.visibleTextEditors[1];
  f.runtime.disposeOwner('owner');const replacement=f.runtime.forExtension('owner').window.visibleTextEditors[1];
  assert.notEqual(old,replacement);assert.equal(old.document,replacement.document);
  assert.throws(()=>{old.selection=new Selection(0,0,0,0);},/retired/);
  await assert.rejects(old.edit(edit=>edit.insert(new Position(0,0),'lost')),/retired/);assert.equal(f.calls.length,0);
});
test('language epoch ABA in an edit callback rejects unchanged text-version admission',async()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;
  await assert.rejects(editor.edit(edit=>{edit.insert(new Position(0,0),'lost');f.docs[0].languageId='python';f.docs[0].languageEpoch=1;f.sync();f.docs[0].languageId='cpp';f.docs[0].languageEpoch=2;f.sync();}),/source changed/);
  assert.equal(f.calls.length,0);assert.equal(editor.document.version,1);assert.equal(editor.document.getText(),'猫🙂\r\nsecond');
});
