'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Selection,Uri}=require('./api-types.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
function held(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
async function tick(){await new Promise(resolve=>setImmediate(resolve));}
function fixture(handler,flags={}){
  let runtime,generation=0;const records=new Map([[1,{id:1,uri:Uri.file('/tmp/a.cpp').toString(),text:'猫🙂\r\nsecond',eol:2,version:1,languageId:'cpp',languageEpoch:0,isDirty:true,savedGeneration:0}]]),calls=[],messages=[];let active=1;
  const origin=[{anchor:{line:0,character:0},active:{line:0,character:0}}];
  const sync=()=>runtime.sync({generation:++generation,documents:[...records.values()],active,selections:records.get(active)?.selections || origin});
  function publish(id,patch){records.set(id,{...records.get(id),...patch});sync();}
  runtime=createApi((method,params,lifecycle)=>{calls.push({method,params,lifecycle});if(handler)return handler({method,params,lifecycle,publish});return Promise.resolve({document:params.args.document});},(method,params)=>messages.push({method,params}),{session:8,documentLanguage:true,editorSelections:true,...flags});
  sync();const api=runtime.forExtension('test.owner');return{runtime,api,records,calls,messages,publish,sync,focus(id){active=id;sync();}};
}
test('language change keeps identity/version/dirty/text; old-language close then new-language open precedes ACK',async()=>{
  const f=fixture(({params,publish})=>{publish(1,{languageId:params.args.language,languageEpoch:1});return {document:1};});
  const doc=f.api.workspace.textDocuments[0],editor=f.api.window.activeTextEditor,events=[];
  f.api.workspace.onDidCloseTextDocument(d=>{events.push(['close',d,d.languageId,d.isClosed]);assert.equal(f.api.workspace.textDocuments[0],doc);});
  f.api.workspace.onDidOpenTextDocument(d=>events.push(['open',d,d.languageId,d.isClosed]));
  assert.equal(await f.api.languages.setTextDocumentLanguage(doc,'python'),doc);
  assert.deepEqual(events,[['close',doc,'cpp',false],['open',doc,'python',false]]);assert.equal(f.api.window.activeTextEditor,editor);assert.equal(doc.getText(),'猫🙂\r\nsecond');assert.equal(doc.version,1);assert.equal(doc.isDirty,true);
  assert.deepEqual(f.calls[0].params.args,{document:1,version:1,language:'python'});
});
test('same language is quiet and malformed, foreign, closed and unknown languages never fake successful change',async()=>{
  const f=fixture(({params})=>{if(params.args.language==='unknown')throw Error('Unknown language');return {document:1};}),doc=f.api.workspace.textDocuments[0];let events=0;
  f.api.workspace.onDidCloseTextDocument(()=>events++);f.api.workspace.onDidOpenTextDocument(()=>events++);
  assert.equal(await f.api.languages.setTextDocumentLanguage(doc,'cpp'),doc);assert.equal(events,0);
  await assert.rejects(f.api.languages.setTextDocumentLanguage(doc,'unknown'),/Unknown/);assert.equal(doc.languageId,'cpp');
  for(const value of ['',null,{},'猫'.repeat(50)])await assert.rejects(f.api.languages.setTextDocumentLanguage(doc,value),/language/);
  await assert.rejects(f.api.languages.setTextDocumentLanguage({_snapshot:{id:1,version:1}},'python'),/no longer open/);assert.equal(f.calls.length,2);
  f.records.clear();f.sync();await assert.rejects(f.api.languages.setTextDocumentLanguage(doc,'python'),/no longer open/);
});
test('language wire and acknowledgement use private native identity instead of public getters',async()=>{
  const f=fixture(({publish})=>{publish(1,{languageId:'python',languageEpoch:1});return {document:1};}),doc=f.api.workspace.textDocuments[0];
  doc._snapshot={id:99,version:88,languageId:'python',languageEpoch:99};Object.defineProperty(doc,'uri',{get(){throw Error('untrusted resource');}});
  assert.equal(await f.api.languages.setTextDocumentLanguage(doc,'python'),doc);assert.deepEqual(f.calls[0].params.args,{document:1,version:1,language:'python'});
});
test('missing language mirror and caller retirement after actual publication reject old continuation',async()=>{
  const f=fixture(()=>({document:1})),doc=f.api.workspace.textDocuments[0];await assert.rejects(f.api.languages.setTextDocumentLanguage(doc,'python'),/mirror.*before/);
  const wait=held(),second=fixture(()=>wait.promise),old=second.api.workspace.textDocuments[0];const request=second.api.languages.setTextDocumentLanguage(old,'python');
  second.runtime.disposeOwner('test.owner');second.runtime.forExtension('test.owner');second.publish(1,{languageId:'python',languageEpoch:1});wait.resolve({document:1});await assert.rejects(request,/retired/);assert.equal(old.languageId,'python');
});
test('single and multiple setters expose assigned selections immediately but emit only authoritative native events',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),editor=f.api.window.activeTextEditor,events=[];f.api.window.onDidChangeTextEditorSelection(event=>events.push(event));
  const a=new Selection(1,6,0,1),b=new Selection(0,3,0,3);editor.selections=[a,b];assert.equal(editor.selection,a);assert.deepEqual(editor.selections,[a,b]);assert.equal(events.length,0);
  assert.deepEqual(f.calls[0].params.args,{document:1,version:1,selections:[{anchor:{line:1,character:6},active:{line:0,character:1}},{anchor:{line:0,character:3},active:{line:0,character:3}}]});
  f.publish(1,{selections:f.calls[0].params.args.selections,selectionKind:3});assert.equal(events.length,1);assert.equal(events[0].textEditor,editor);assert.equal(events[0].kind,f.api.TextEditorSelectionChangeKind.Command);
  assert.equal(events[0].selections[0].isReversed,true);wait.resolve({document:1});await tick();assert.equal(f.messages.length,0);
});
test('empty selections normalize to origin; invalid shape/count refuses synchronously without wire',async()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;editor.selections=[];assert.deepEqual([editor.selection.anchor.line,editor.selection.active.character],[0,0]);assert.deepEqual(f.calls[0].params.args.selections,[{anchor:{line:0,character:0},active:{line:0,character:0}}]);
  for(const value of [null,{},[{}],new Array(10001).fill(new Selection(0,0,0,0))])assert.throws(()=>{editor.selections=value;},/Selection|10000/);
  assert.throws(()=>{editor.selection={anchor:{line:0,character:0},active:{line:0,character:0}};},/Selection/);assert.equal(f.calls.length,1);await tick();
});
test('native clamped mirror replaces optimistic out-of-bounds values and preserves order/direction',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),editor=f.api.window.activeTextEditor,events=[];f.api.window.onDidChangeTextEditorSelection(event=>events.push(event));
  editor.selections=[new Selection(999,999,0,1),new Selection(0,3,0,2)];assert.equal(editor.selection.anchor.line,999);
  const actual=[{anchor:{line:1,character:6},active:{line:0,character:1}},{anchor:{line:0,character:3},active:{line:0,character:1}}];
  f.publish(1,{selections:actual,selectionKind:3});assert.equal(editor.selection.anchor.line,1);assert.equal(editor.selections[1].active.character,1);assert.equal(events.length,1);wait.resolve({document:1});await tick();
});
test('inactive retained editor mirrors selection changes without changing active editor',async()=>{
  const f=fixture(),first=f.api.window.activeTextEditor,events=[];f.records.set(2,{id:2,uri:Uri.file('/tmp/b.cpp').toString(),text:'B',version:1,languageId:'cpp',languageEpoch:0,selections:[{anchor:{line:0,character:0},active:{line:0,character:0}}]});f.focus(2);const second=f.api.window.activeTextEditor;
  f.api.window.onDidChangeTextEditorSelection(event=>events.push(event));first.selection=new Selection(1,2,1,4);f.publish(1,{selections:f.calls[0].params.args.selections,selectionKind:3});
  assert.equal(f.api.window.activeTextEditor,second);assert.equal(events[0].textEditor,first);assert.equal(first.selection.active.character,4);await tick();
});
test('selection validation captures getters once and prevents native source/caller retargeting',()=>{
  for(const cause of ['version','retirement']){
    const f=fixture(),editor=f.api.window.activeTextEditor,value=new Selection(0,0,0,1);let anchorReads=0,activeReads=0;
    Object.defineProperty(value,'anchor',{get(){anchorReads++;if(cause==='version')f.publish(1,{version:2,text:'new'});else{f.runtime.disposeOwner('test.owner');f.runtime.forExtension('test.owner');}return {line:0,character:0};}});
    Object.defineProperty(value,'active',{get(){activeReads++;return {line:0,character:1};}});
    assert.throws(()=>{editor.selection=value;},/source changed|retired/);assert.equal(anchorReads,1);assert.equal(activeReads,1);assert.equal(f.calls.length,0);
  }
});
test('failed older setter cannot erase a newer optimistic overlay and failures are handled',async()=>{
  const first=held(),second=held(),f=fixture(()=>f.calls.length===1?first.promise:second.promise),editor=f.api.window.activeTextEditor;
  const a=new Selection(0,1,0,1),b=new Selection(0,3,0,3);editor.selection=a;editor.selection=b;first.reject(Error('stale setter'));
  await tick();assert.equal(editor.selection,b);assert.equal(f.messages.length,1);second.reject(Error('refused'));await tick();assert.equal(editor.selection.active.character,0);assert.equal(f.messages.length,2);
});
test('public metadata forgery cannot target selections; old editor after Save As cannot send',()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;editor.document._snapshot={id:99,version:88};editor.selection=new Selection(0,1,0,1);assert.equal(f.calls[0].params.args.document,1);assert.equal(f.calls[0].params.args.version,1);
  f.publish(1,{uri:Uri.file('/tmp/replaced.cpp').toString()});assert.throws(()=>{editor.selection=new Selection(0,0,0,0);},/no longer open/);assert.equal(f.calls.length,1);
});
test('native selection events suppress noops and preserve only proven kind values',()=>{
  const f=fixture(),events=[];f.api.window.onDidChangeTextEditorSelection(event=>events.push(event));const value=[{anchor:{line:0,character:1},active:{line:0,character:3}}];
  f.publish(1,{selections:value,selectionKind:1});f.publish(1,{selections:value,selectionKind:3});assert.equal(events.length,1);assert.equal(events[0].kind,1);
  f.publish(1,{selections:[{anchor:{line:0,character:2},active:{line:0,character:2}}],selectionKind:99});assert.equal(events.length,2);assert.equal(events[1].kind,undefined);
});
test('capabilities gate mutations while selection event API remains available',()=>{
  const f=fixture(null,{documentLanguage:false,editorSelections:false});assert.throws(()=>f.api.languages.setTextDocumentLanguage,/not implemented/);assert.equal(typeof f.api.window.onDidChangeTextEditorSelection,'function');assert.throws(()=>{f.api.window.activeTextEditor.selection=new Selection(0,0,0,0);},/not implemented/);assert.equal(f.calls.length,0);
});
test('typed editor operations retain exact slots after timeout and reject malformed terminal frames',async()=>{
  for(const method of ['nativeDocumentLanguage','nativeEditorSelections']){
    const timers=[],frames=[],queue=createNativeRequests(frame=>frames.push(frame),()=>({id:9,owner:'origin',active:true}),{setTimeout(fn){timers.push(fn);return fn;},clearTimeout(){}});let terminals=0;
    const lifecycle={onTerminal:()=>terminals++},pending=queue.request(method,{session:1,owner:'caller',args:{document:1,version:1}},lifecycle);const rejected=assert.rejects(pending,/outcome is unknown/);assert(lifecycle.retained);assert.equal(frames[0].params.commandOwner,'origin');timers[0]();await rejected;
    for(const result of [{document:0},{document:1,other:true},true])assert.equal(queue.settle({id:frames[0].id,result}),false);
    assert.equal(terminals,0);assert.equal(queue.settle({id:frames[0].id,result:{document:1}}),true);assert.equal(terminals,1);assert.equal(queue.settle({id:frames[0].id,result:{document:1}}),false);
  }
});
test('owner retirement and inactive source-version changes retire only presentation overlays, retaining actual work',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),editor=f.api.window.activeTextEditor;editor.selection=new Selection(0,3,0,3);assert.equal(editor.selection.active.character,3);
  f.runtime.disposeOwner('test.owner');f.runtime.forExtension('test.owner');assert.equal(editor.selection.active.character,0);assert.equal(f.calls.length,1);
  wait.reject(Error('retired original work'));await tick();assert.equal(f.messages.filter(frame=>frame.method==='message').length,1);
  const second=held(),g=fixture(()=>second.promise),inactive=g.api.window.activeTextEditor;inactive.selection=new Selection(0,3,0,3);
  g.records.set(2,{id:2,uri:Uri.file('/tmp/b.cpp').toString(),text:'B',version:1,languageId:'cpp',languageEpoch:0});g.focus(2);
  g.publish(1,{version:2,text:'new'});assert.equal(inactive.selection.active.character,0);assert.equal(g.calls.length,1);second.reject(Error('stale original work'));await tick();
});
