'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri,Range,Selection}=require('./api-types.cjs');
const {normalizeOptions,decodeOptions,decodeVisibleRanges}=require('./editor-presentation.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
const OPTIONS=Object.freeze({tabSize:4,indentSize:4,originalIndentSize:'tabSize',insertSpaces:true,cursorStyle:1,lineNumbers:1});
const caret=character=>[{anchor:{line:0,character},active:{line:0,character}}];
const range=(start,end)=>({start:{line:start,character:0},end:{line:end,character:0}});
function held(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(handler,flags={}){
  let runtime,generation=0,activeEditor='left';
  const doc={id:1,uri:Uri.file('/tmp/猫.cpp').toString(),text:'猫🙂\r\nsecond\r\nthird',eol:2,languageId:'cpp',languageEpoch:0,version:1,isDirty:true};
  let inventory=[{id:'left',document:1,viewColumn:1,selections:caret(1),options:{...OPTIONS},visibleRanges:[range(0,2)]},
    {id:'right',document:1,viewColumn:2,selections:caret(3),options:{...OPTIONS},visibleRanges:[]}];
  const calls=[],messages=[];
  const sync=()=>runtime.sync({generation:++generation,documents:[doc],editors:inventory,activeEditor});
  runtime=createApi((method,params,lifecycle)=>{calls.push({method,params,lifecycle});return Promise.resolve(handler?handler({method,params,lifecycle,sync}):{document:1,editor:params.args.editor});},(method,params)=>messages.push({method,params}),{session:1,editorOptions:true,editorReveal:true,editorSelections:true,...flags});
  sync();const api=runtime.forExtension('owner');return{runtime,api,doc,calls,messages,sync,get inventory(){return inventory;},set inventory(value){inventory=value;},focus(value){activeEditor=value;sync();}};
}
test('source tab/indent validators floor numbers and parse decimal prefixes; invalid values are ignored',()=>{
  for(const [value,result] of [[6.9,6],['6.9',6],['12px',12],['1e3',1],['  +08abc',8],['0x10',null],[-1,null],[0,null],['no',null],[null,null],[{},null],[NaN,null]]){
    const normalized=normalizeOptions(value,OPTIONS,'tabSize');assert.deepEqual(normalized.update,result===null?{}:{tabSize:result});
  }
  assert.deepEqual(normalizeOptions('auto',OPTIONS,'tabSize').update,{tabSize:'auto'});
  assert.deepEqual(normalizeOptions('tabSize',OPTIONS,'indentSize').update,{indentSize:'tabSize'});
  assert.throws(()=>normalizeOptions(Infinity,OPTIONS,'tabSize'),/safe integer/);assert.throws(()=>normalizeOptions('2'.repeat(4097),OPTIONS,'tabSize'),/4096/);
});
test('insertSpaces retains pinned false-string and Boolean semantics without invoking object coercion',()=>{
  for(const [value,result] of [['false',false],[false,false],['',false],[0,false],[undefined,false],[null,false],['true',true],['0',true],[{},true]]){
    const update=normalizeOptions(value,{...OPTIONS,insertSpaces:!result},'insertSpaces').update;assert.deepEqual(update,{insertSpaces:result});
  }
  const value={valueOf(){throw Error('coercion');},toString(){throw Error('coercion');}};assert.deepEqual(normalizeOptions(value,{...OPTIONS,insertSpaces:false},'insertSpaces').update,{insertSpaces:true});
});
test('options getter is stable per scoped editor; legacy/unknown frames do not fabricate resolved defaults',()=>{
  const f=fixture(),[left,right]=f.api.window.visibleTextEditors;assert.equal(left.options,left.options);assert.notEqual(left.options,right.options);
  assert.deepEqual({...left.options},{tabSize:4,indentSize:4,insertSpaces:true,cursorStyle:1,lineNumbers:1});
  assert.deepEqual(left.visibleRanges.map(value=>[value.start.line,value.end.line]),[[0,2]]);assert.deepEqual(right.visibleRanges,[]);
  const runtime=createApi(()=>Promise.resolve({}),()=>{},{});runtime.sync({generation:1,documents:[f.doc],active:1,selections:caret(0)});const editor=runtime.forExtension('owner').window.activeTextEditor;
  assert.equal(editor.options.tabSize,undefined);assert.equal(editor.options.cursorStyle,undefined);assert.deepEqual(editor.visibleRanges,[]);
});
test('single options update optimistically, use exact inactive pane, and emit only actual native changes',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),[left,right]=f.api.window.visibleTextEditors,events=[];f.focus('right');
  f.api.window.onDidChangeTextEditorOptions(event=>events.push(event));const options=left.options;options.tabSize='6.9px';assert.equal(options.tabSize,6);assert.equal(right.options.tabSize,4);assert.equal(events.length,0);
  assert.deepEqual(f.calls[0].params.args,{document:1,version:1,editor:'left',options:{tabSize:6}});
  for(const entry of f.inventory)entry.options={...entry.options,tabSize:6,indentSize:6};f.sync();
  assert.equal(events.length,2);assert.equal(events[0].textEditor,left);assert.equal(events[0].options.originalIndentSize,'tabSize');assert.equal(options.indentSize,6);assert.equal(f.api.window.activeTextEditor,right);
  wait.resolve({document:1,editor:'left'});await tick();assert.equal(f.messages.length,0);
});
test('auto and tabSize modes keep numeric getters until the actual resolved receipt',async()=>{
  const wait=held(),f=fixture(()=>wait.promise),editor=f.api.window.activeTextEditor;
  editor.options={tabSize:'auto',insertSpaces:'auto',indentSize:'tabSize'};
  assert.deepEqual({...editor.options},{tabSize:4,indentSize:4,insertSpaces:true,cursorStyle:1,lineNumbers:1});
  assert.deepEqual(f.calls[0].params.args.options,{tabSize:'auto',indentSize:'tabSize',insertSpaces:'auto'});
  f.inventory[0].options={...OPTIONS,tabSize:2,indentSize:2,insertSpaces:false};f.sync();assert.equal(editor.options.tabSize,2);assert.equal(editor.options.indentSize,2);assert.equal(editor.options.insertSpaces,false);
  wait.resolve({document:1,editor:'left'});await tick();
});
test('numeric indent tracks originalIndentSize for no-op, including resolved-equal tabSize mode',async()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;editor.options.indentSize=4;assert.deepEqual(f.calls[0].params.args.options,{indentSize:4});assert.equal(editor.options.indentSize,4);
  editor.options.indentSize=4;assert.equal(f.calls.length,1);
  f.inventory[0].options={...OPTIONS,originalIndentSize:4};f.sync();editor.options.indentSize=4;assert.equal(f.calls.length,1);await tick();
});
test('bulk setters snapshot each field once and submit one combined normalized request',async()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor,input={},reads={};
  for(const [key,value] of Object.entries({tabSize:'2px',indentSize:3.8,insertSpaces:'false',cursorStyle:5,lineNumbers:3}))Object.defineProperty(input,key,{get(){reads[key]=(reads[key]||0)+1;return value;}});
  editor.options=input;assert.deepEqual(reads,{tabSize:1,indentSize:1,insertSpaces:1,cursorStyle:1,lineNumbers:1});assert.equal(f.calls.length,1);
  assert.deepEqual(f.calls[0].params.args.options,{tabSize:2,indentSize:3,insertSpaces:false,cursorStyle:5,lineNumbers:3});assert.equal(editor.options.lineNumbers,f.api.TextEditorLineNumbersStyle.Interval);await tick();
});
test('no-op and invalid numeric setters remain quiet without synthetic events or native work',()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;let events=0;f.api.window.onDidChangeTextEditorOptions(()=>events++);
  editor.options.tabSize=4;editor.options.tabSize='bad';editor.options.indentSize=0;editor.options.insertSpaces=true;editor.options.cursorStyle=1;editor.options.lineNumbers=1;editor.options={unknown:'ignored'};
  assert.equal(f.calls.length,0);assert.equal(events,0);assert.equal(editor.options.tabSize,4);
});
test('option getter source/owner/view retirement rejects whole bulk before staging or wire',()=>{
  for(const reason of ['epoch','owner','view']){
    const f=fixture(),editor=f.api.window.activeTextEditor,input={};let reads=0;
    Object.defineProperty(input,'tabSize',{get(){reads++;if(reason==='epoch'){f.doc.languageId='python';f.doc.languageEpoch++;f.sync();f.doc.languageId='cpp';f.doc.languageEpoch++;f.sync();}else if(reason==='owner'){f.runtime.disposeOwner('owner');f.runtime.forExtension('owner');}else{f.inventory=f.inventory.slice(1);f.focus('right');}return 2;}});
    assert.throws(()=>{editor.options=input;},/source changed|retired|no longer available/);assert.equal(reads,1);assert.equal(editor.options.tabSize,4);assert.equal(f.calls.length,0);
  }
});
test('older failed option work cannot erase newer overlay; failures handled and authoritative ranges untouched',async()=>{
  const a=held(),b=held(),f=fixture(()=>f.calls.length===1?a.promise:b.promise),editor=f.api.window.activeTextEditor;
  editor.options.tabSize=2;editor.options.tabSize=8;a.reject(Error('older'));await tick();assert.equal(editor.options.tabSize,8);
  b.reject(Error('latest'));await tick();assert.equal(editor.options.tabSize,4);assert.equal(f.messages.length,2);assert.deepEqual(editor.visibleRanges.map(value=>value.end.line),[2]);
});
test('public option/editor metadata overrides cannot authorize a different private pane or source',async()=>{
  const f=fixture(),editor=f.api.window.visibleTextEditors[1],options=editor.options;
  Object.defineProperty(editor,'document',{get(){throw Error('public editor source');}});Object.defineProperty(editor,'options',{get(){throw Error('public options');}});f.doc.publicField='irrelevant';
  f.api.workspace.textDocuments[0]._snapshot={id:99,version:88};options.lineNumbers=3;
  assert.equal(f.calls[0].params.args.editor,'right');assert.equal(f.calls[0].params.args.document,1);assert.equal(f.calls[0].params.args.version,1);await tick();
});
test('visible range and option callbacks see all pane properties published, with scoped identity and no noops',()=>{
  const f=fixture(),[left,right]=f.api.window.visibleTextEditors,events=[];
  f.api.window.onDidChangeTextEditorOptions(event=>{events.push(['options',event.textEditor]);assert.equal(right.options.cursorStyle,2);assert.equal(left.selection.active.character,3);assert.equal(left.visibleRanges[0].start.line,1);});
  f.api.window.onDidChangeTextEditorSelection(event=>events.push(['selection',event.textEditor]));
  f.api.window.onDidChangeTextEditorVisibleRanges(event=>{events.push(['ranges',event.textEditor]);assert(event.visibleRanges[0] instanceof Range);assert.equal(event.textEditor,left);});
  f.inventory[0].options={...OPTIONS,tabSize:2};f.inventory[1].options={...OPTIONS,cursorStyle:2};f.inventory[0].selections=caret(3);f.inventory[0].visibleRanges=[range(1,3)];f.sync();f.sync();
  assert.deepEqual(events,[['options',left],['selection',left],['ranges',left],['options',right]]);
});
test('reveals send original ranges and enum types without optimistic scrolling, selection or focus',async()=>{
  const f=fixture(),[left,right]=f.api.window.visibleTextEditors;f.focus('right');const before=left.selection;let events=0;f.api.window.onDidChangeTextEditorVisibleRanges(()=>events++);
  for(const kind of [undefined,1,2,3])left.revealRange(new Range(1,1,2,4),kind);
  assert.equal(f.calls.length,4);assert.deepEqual(f.calls.map(call=>call.params.args.revealType),[0,1,2,3]);
  assert.deepEqual(f.calls[0].params.args,{document:1,version:1,editor:'left',range:{start:{line:1,character:1},end:{line:2,character:4}},revealType:0});
  assert.equal(left.selection,before);assert.equal(f.api.window.activeTextEditor,right);assert.equal(events,0);assert.equal(left.visibleRanges[0].start.line,0);await tick();
});
test('reveal getter retargeting, bad shapes/types and retired handles never enter service',()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor,value=new Range(0,0,1,0);let reads=0;
  Object.defineProperty(value,'start',{get(){reads++;f.doc.version++;f.sync();return {line:0,character:0};}});
  assert.throws(()=>editor.revealRange(value),/source changed/);assert.equal(reads,1);assert.equal(f.calls.length,0);
  assert.throws(()=>editor.revealRange({},0),/Range/);assert.throws(()=>editor.revealRange(new Range(0,0,0,0),4),/RevealType/);
  f.inventory=f.inventory.slice(1);f.focus('right');assert.throws(()=>editor.revealRange(new Range(0,0,0,0)),/no longer available/);assert.throws(()=>{editor.options.tabSize=2;},/no longer available/);assert.equal(f.calls.length,0);
});
test('visibleRanges getter cannot overwrite private range proof and is read-only',()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor,range=editor.visibleRanges;range[0]._start={line:99,character:0};range.length=0;
  assert.equal(editor.visibleRanges[0].start.line,0);assert.throws(()=>{editor.visibleRanges=[];},TypeError);
});
test('malformed option/range frames refuse atomically before generation/model/editor mutation',()=>{
  const f=fixture(),editor=f.api.window.activeTextEditor;
  for(const entry of [{...f.inventory[0],options:{...OPTIONS,originalIndentSize:0}},{...f.inventory[0],options:{...OPTIONS,lineNumbers:4}},
    {...f.inventory[0],visibleRanges:Array(4097).fill(range(0,1))},{...f.inventory[0],visibleRanges:[range(2,1)]}]){
    assert.throws(()=>f.runtime.sync({generation:99,documents:[{...f.doc,text:'lost',version:2}],editors:[entry],activeEditor:'left'}),/Invalid native|range is reversed/);
    assert.equal(editor.document.getText(),'猫🙂\r\nsecond\r\nthird');assert.equal(editor.options.tabSize,4);
  }
  f.sync();assert.equal(f.api.window.activeTextEditor,editor);
});
test('capabilities gate native calls while actual mirrored getters/events remain readable',()=>{
  const f=fixture(null,{editorOptions:false,editorReveal:false}),editor=f.api.window.activeTextEditor;
  assert.equal(editor.options.tabSize,4);assert.throws(()=>{editor.options.tabSize=2;},/not implemented/);assert.throws(()=>editor.revealRange(new Range(0,0,0,0)),/not implemented/);assert.equal(f.calls.length,0);
});
test('typed options/reveal retain actual slots after timeout and accept only exact original terminal editor',async()=>{
  for(const method of ['nativeEditorOptions','nativeEditorReveal']){
    const frames=[],timers=[];let terminal=0;const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:3,owner:'origin'}),{setTimeout(fn){timers.push(fn);return fn;},clearTimeout(){}}),lifecycle={onTerminal(){terminal++;}};
    const promise=queue.request(method,{owner:'caller',args:{document:1,editor:'left'}},lifecycle),rejected=assert.rejects(promise,/outcome is unknown/);timers[0]();await rejected;
    assert.equal(lifecycle.retained,true);assert.equal(frames[0].params.commandOwner,'origin');for(const result of [{document:1},{document:1,editor:'right'},{document:1,editor:'left',extra:true}])assert.equal(queue.settle({id:frames[0].id,result}),false);
    assert.equal(terminal,0);assert.equal(queue.settle({id:frames[0].id,result:{document:1,editor:'left'}}),true);assert.equal(terminal,1);
  }
});
test('all eight timed-out service requests retain occupancy until typed actual receipts, including reveal',async()=>{
  const frames=[],timers=[];const queue=createNativeRequests(frame=>frames.push(frame),()=>undefined,{setTimeout(fn){timers.push(fn);return fn;},clearTimeout(){}}),f=fixture(({method,params,lifecycle})=>queue.request(method,params,lifecycle)),editor=f.api.window.activeTextEditor;
  for(let i=0;i<8;i++)editor.revealRange(new Range(0,0,1,0));assert.equal(frames.length,8);for(const timer of timers)timer();await tick();
  editor.revealRange(new Range(0,0,1,0));await tick();assert.equal(frames.length,8);assert(f.messages.some(value=>value.params.text.includes('queue limit')));
  for(const frame of frames)assert.equal(queue.settle({id:frame.id,result:{document:1,editor:'left'}}),true);
  editor.revealRange(new Range(0,0,1,0));assert.equal(frames.length,9);assert.equal(queue.settle({id:frames[8].id,result:{document:1,editor:'left'}}),true);await tick();queue.shutdown();
});
