'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const {createEditorSnippets} = require('./editor-snippets.cjs');
const {Position, Range, Selection, Uri} = require('./api-types.cjs');
const {SnippetString} = require('./snippet-string.cjs');
const {createCallerContext} = require('./caller-context.cjs');
const {createApi} = require('./api.cjs');
const {createNativeRequests} = require('./native-requests.cjs');
function held() { let resolve, reject; const promise = new Promise((a,b) => {resolve=a;reject=b;}); return {promise,resolve,reject}; }
function fixture(handler, enabled=true) {
  const callers = createCallerContext(() => {}), document = {}, base = {}, calls = [];
  const state = {nativeId:'pane-original',actual:[new Selection(1,5,0,3),new Selection(2,1,2,1)]};
  let version = 7, epoch = 2, alive = true;
  const module = createEditorSnippets({enabled,
    captureEditor(owner, doc, editor) {
      const guard=callers.bind(owner,()=>{}), source={document:17,version}, originalEpoch=epoch;
      guard(); if (doc!==document||editor!==base||!alive) throw Error('Text editor is no longer available');
      return {source,state,recheck(){guard();if(!alive)throw Error('Text editor is no longer available');if(version!==source.version||epoch!==originalEpoch)throw Error('Editor source changed');},
        accept(reply){guard();if(!alive||reply.document!==source.document||reply.editor!==state.nativeId)throw Error('Editor mirror was not published');}};
    },
    call(owner,method,args){calls.push({owner,method,args});return handler ? handler(args) : Promise.resolve({document:17,editor:state.nativeId,applied:true});},
  });
  return {calls,state,document,base,module, insert:(snippet,where,options)=>module.insertSnippet('owner',document,base,snippet,where,options),
    edit(){version++;epoch++;},editUndo(){epoch++;},hide(){alive=false;},retire(){callers.retire('owner');callers.bind('owner',()=>{})();}};
}
const point=(line,character)=>({line,character});
const range=(start,end)=>({start,end});
const defaultRanges=[range(point(0,3),point(1,5)),range(point(2,1),point(2,1))];

test('omitted/falsy/empty where snapshots private reversed selections in original order',async()=>{
  for(const where of [undefined,null,false,0,'',[]]) {
    const f=fixture();assert.equal(await f.insert(new SnippetString('${1:猫🙂}\0$1$0'),where),true);
    assert.deepEqual(f.calls,[{owner:'owner',method:'nativeEditorInsertSnippet',args:{document:17,version:7,editor:'pane-original',snippet:'${1:猫🙂}\0$1$0',ranges:defaultRanges,undoStopBefore:true,undoStopAfter:true,keepWhitespace:false}}]);
  }
});
test('Position, Range, Selection and mixed arrays preserve UTF16 coordinates without host clamping',async()=>{
  const values=[new Position(0,2),new Range(1,1,1,5),new Selection(3,4,2,1)];
  const expected=[range(point(0,2),point(0,2)),range(point(1,1),point(1,5)),range(point(2,1),point(3,4))];
  for(let index=0;index<values.length;index++){
    const f=fixture();await f.insert(new SnippetString('$0'),values[index]);assert.deepEqual(f.calls[0].args.ranges,[expected[index]]);
  }
  const f=fixture();await f.insert(new SnippetString('$0'),values);assert.deepEqual(f.calls[0].args.ranges,expected);
  await f.insert(new SnippetString('x'),new Position(0,2147483647));assert.equal(f.calls[1].args.ranges[0].start.character,2147483647);
  let lineReads=0,characterReads=0;
  await f.insert(new SnippetString('x'),[{get line(){lineReads++;return 4;},get character(){characterReads++;return 2;}}]);
  assert.deepEqual(f.calls[2].args.ranges,[range(point(4,2),point(4,2))]);assert.equal(lineReads,1);assert.equal(characterReads,1);
});
test('provided option fields use truthiness and are captured once without mutating frozen options',async()=>{
  const f=fixture(),reads=[],options=Object.freeze({get undoStopBefore(){reads.push('before');return 0;},get undoStopAfter(){reads.push('after');return 'false';},get keepWhitespace(){reads.push('whitespace');return {yes:true};}});
  await f.insert(new SnippetString('x'),new Position(0,0),options);
  assert.deepEqual(reads,['before','after','whitespace']);
  assert.equal(f.calls[0].args.undoStopBefore,false);assert.equal(f.calls[0].args.undoStopAfter,true);assert.equal(f.calls[0].args.keepWhitespace,true);
  const g=fixture();await g.insert(new SnippetString('x'),undefined,{});assert.equal(g.calls[0].args.undoStopBefore,false);assert.equal(g.calls[0].args.undoStopAfter,false);assert.equal(g.calls[0].args.keepWhitespace,false);
});
test('explicit location getters and snippet value each execute once; template syntax remains unchanged',async()=>{
  const f=fixture(),reads={start:0,end:0,line:0,character:0,value:0},value=new Range(0,0,0,1),start={get line(){reads.line++;return 1;},get character(){reads.character++;return 3;}};
  Object.defineProperty(value,'start',{get(){reads.start++;return start;}});Object.defineProperty(value,'end',{get(){reads.end++;return point(1,5);}});
  await f.insert({get value(){reads.value++;return '${1|猫,🙂|} ${1/(.*)/${1:/upcase}/} $TM_FILENAME $0';}},value);
  assert.deepEqual(reads,{start:1,end:1,line:1,character:1,value:1});assert.deepEqual(f.calls[0].args.ranges,[range(point(1,3),point(1,5))]);assert.equal(f.calls[0].args.snippet,'${1|猫,🙂|} ${1/(.*)/${1:/upcase}/} $TM_FILENAME $0');
});
test('private optimistic selection cohort is captured before template getter and public metadata is ignored',async()=>{
  const f=fixture();f.document._snapshot={id:99,version:100};f.base.selections=[new Selection(9,9,9,9)];f.state.overlay={values:[new Selection(4,2,4,5)]};
  await f.insert({get value(){f.state.overlay={values:[new Selection(8,0,8,0)]};return 'x';}});
  assert.deepEqual(f.calls[0].args.ranges,[range(point(4,2),point(4,5))]);assert.equal(f.calls[0].args.document,17);assert.equal(f.calls[0].args.version,7);
});
test('source edit, edit/Undo epoch, view retirement and owner ABA from getters all refuse entire request',async()=>{
  for(const mutation of ['edit','editUndo','hide','retire'])for(const getter of ['value','range','option']){
    const f=fixture();let reads=0;const mutate=()=>{reads++;f[mutation]();};
    const snippet=getter==='value'?{get value(){mutate();return 'x';}}:new SnippetString('x');
    const where=new Range(0,0,0,1);
    if(getter==='range')Object.defineProperty(where,'start',{get(){mutate();return point(0,0);}});
    const options=getter==='option'?{get keepWhitespace(){mutate();return false;}}:undefined;
    await assert.rejects(f.insert(snippet,where,options),/source changed|no longer available|retired/);
    assert.equal(reads,1);assert.equal(f.calls.length,0);
  }
});
test('legacy capability refusal and count/template refusal precede late getters and any native call',async()=>{
  let reads=0;const f=fixture(undefined,false);await assert.rejects(f.insert({get value(){reads++;throw Error('getter');}}),/not implemented/);assert.equal(reads,0);
  const g=fixture(),locations=new Array(4097);Object.defineProperty(locations,0,{get(){reads++;return new Position(0,0);}});
  await assert.rejects(g.insert({get value(){reads++;return 'x';}},locations),/4096/);assert.equal(reads,0);
  for(const value of ['x'.repeat(65537),'猫'.repeat(21846)])await assert.rejects(g.insert({value},undefined,{get keepWhitespace(){reads++;return false;}}),/64 KiB/);
  assert.equal(reads,0);assert.equal(g.calls.length,0);
});
test('maximum admitted template and range cohort stays bounded with escaped NUL, no partial wire',async()=>{
  const f=fixture(),locations=Array.from({length:4096},()=>new Position(2147483647,2147483647));
  assert.equal(await f.insert(new SnippetString('\0'.repeat(65536)),locations),true);
  assert.equal(f.calls[0].args.ranges.length,4096);assert.equal(f.calls[0].args.snippet.length,65536);assert(Buffer.byteLength(JSON.stringify(f.calls[0].args))<=1024*1024);
});
test('malformed arguments reject as Promise with no request and no partially evaluated following range',async()=>{
  for(const snippet of [undefined,null,'x',{value:1}]){const f=fixture();await assert.rejects(f.insert(snippet),/value must be a string/);assert.equal(f.calls.length,0);}
  const invalidPosition=new Position(0,0);Object.defineProperty(invalidPosition,'line',{value:-1});
  for(const where of [{line:0,character:0},[null],invalidPosition]){
    const f=fixture();await assert.rejects(f.insert(new SnippetString('x'),where),/Position|nonnegative/);assert.equal(f.calls.length,0);
  }
  for(const options of [null,false,'x']){const f=fixture();await assert.rejects(f.insert(new SnippetString('x'),undefined,options),/options must be an object/);assert.equal(f.calls.length,0);}
  const f=fixture(),bad=new Range(0,0,0,1),late=new Range(0,0,0,1);let reads=0;
  Object.defineProperty(bad,'start',{get(){throw Error('original getter failure');}});Object.defineProperty(late,'start',{get(){reads++;return point(0,0);}});
  await assert.rejects(f.insert(new SnippetString('x'),[bad,late]),/original getter failure/);assert.equal(reads,0);assert.equal(f.calls.length,0);
});
test('held exact ACK is required, applied false remains false, and native errors propagate',async()=>{
  const wait=held(),f=fixture(()=>wait.promise);let settled=false;
  const promise=f.insert(new SnippetString('x')).then(value=>{settled=true;return value;});await Promise.resolve();assert.equal(settled,false);
  wait.resolve({document:17,editor:'pane-original',applied:true});assert.equal(await promise,true);
  const g=fixture(()=>Promise.resolve({document:17,editor:'pane-original',applied:false}));assert.equal(await g.insert(new SnippetString('x')),false);
  const h=fixture(()=>Promise.reject(Error('native refusal')));await assert.rejects(h.insert(new SnippetString('x')),/native refusal/);
});
test('wrong/missing/extra ACK fields cannot manufacture success',async()=>{
  for(const reply of [null,true,{document:17,editor:'pane-original'},
    {document:18,editor:'pane-original',applied:true},{document:17,editor:'replacement',applied:true},
    {document:17,editor:'pane-original',applied:'true'},{document:17,editor:'pane-original',applied:true,extra:1}]){
    const f=fixture(()=>Promise.resolve(reply));await assert.rejects(f.insert(new SnippetString('x')),/Invalid native snippet acknowledgement/);
  }
});
test('original consumer and editor lifetime are checked again after held acknowledgement',async()=>{
  for(const mutation of ['retire','hide']){
    const wait=held(),f=fixture(()=>wait.promise),promise=f.insert(new SnippetString('x')),rejected=assert.rejects(promise,/retired|not published/);
    f[mutation]();wait.resolve({document:17,editor:'pane-original',applied:true});await rejected;assert.equal(f.calls.length,1);
  }
});

function publicFixture() {
  const frames=[],timers=[];
  const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:42,owner:'command.origin'}),{
    setTimeout(callback,delay){const timer={callback,delay};timers.push(timer);return timer;},clearTimeout(){},
  });
  const runtime=createApi(queue.request,()=>{},{session:9,editorSnippets:true});
  const document={id:17,uri:Uri.file('/tmp/snippet-public.cpp').toString(),text:'猫🙂\r\nsecond',version:7,eol:2,languageId:'cpp',languageEpoch:2,isDirty:true};
  let generation=0,inventory=[{id:'left',document:17,viewColumn:1,selections:[{anchor:point(0,0),active:point(0,0)}]},
    {id:'right',document:17,viewColumn:2,selections:[{anchor:point(1,4),active:point(1,1)}]}];
  const sync=()=>runtime.sync({generation:++generation,documents:[document],editors:inventory,activeEditor:'left',active:17});
  sync();const api=runtime.forExtension('snippet.consumer');
  return {runtime,queue,frames,timers,document,sync,api,editor:api.window.visibleTextEditors[1],
    replaceRight(){inventory=[inventory[0],{...inventory[1],id:'fresh-right'}];sync();}};
}

test('actual scoped facade inserts in inactive shared pane with original caller and independent command origin',async()=>{
  const f=publicFixture(),editor=f.editor,events=[];
  assert.equal(editor.document,f.api.window.activeTextEditor.document);
  f.api.workspace.onDidChangeTextDocument(event=>events.push(event));
  const oldText=editor.document.getText();
  editor.document._snapshot={id:999,version:999};
  Object.defineProperty(editor,'document',{get(){throw Error('Untrusted public editor document');}});
  Object.defineProperty(editor,'selections',{get(){throw Error('Untrusted public selections');}});
  let completed=false;
  const pending=editor.insertSnippet(new SnippetString('${1:λ} $1$0')).then(value=>{completed=true;return value;});
  const frame=f.frames[0];assert.equal(frame.method,'nativeEditorInsertSnippet');
  assert.deepEqual(frame.params,{session:9,owner:'snippet.consumer',generation:1,command:42,commandOwner:'command.origin',
    args:{document:17,version:7,editor:'right',snippet:'${1:λ} $1$0',ranges:[range(point(1,1),point(1,4))],undoStopBefore:true,undoStopAfter:true,keepWhitespace:false}});
  await Promise.resolve();assert.equal(completed,false);assert.equal(events.length,0);
  assert.equal(f.api.workspace.textDocuments[0].getText(),oldText);
  f.document.text='猫🙂\r\nsλ λnd';f.document.version=8;
  f.sync();assert.equal(events.length,1);assert.equal(events[0].document,f.api.workspace.textDocuments[0]);
  assert.equal(f.api.window.activeTextEditor.viewColumn,1);
  assert.equal(f.queue.settle({id:frame.id,result:{document:17,editor:'right',applied:true}}),true);
  assert.equal(await pending,true);assert.equal(f.api.workspace.textDocuments[0].getText(),f.document.text);
  f.queue.shutdown();
});

test('actual facade getter reentrancy cannot borrow source version, replacement pane or renewed owner',async()=>{
  for(const mutation of ['source','pane','owner']){
    const f=publicFixture();let reads=0;
    await assert.rejects(f.editor.insertSnippet({get value(){
      reads++;
      if(mutation==='source'){f.document.version++;f.sync();}
      else if(mutation==='pane')f.replaceRight();
      else{f.runtime.disposeOwner('snippet.consumer');f.runtime.forExtension('snippet.consumer');}
      return 'lost';
    }}),/source changed|no longer available|retired/);
    assert.equal(reads,1);assert.equal(f.frames.length,0);assert.equal(f.api.workspace.textDocuments[0].getText(),'猫🙂\r\nsecond');
    f.queue.shutdown();
  }
});

test('actual facade rejects held ACK after original caller retirement even when native terminal releases',async()=>{
  const f=publicFixture(),pending=f.editor.insertSnippet(new SnippetString('x')),rejected=assert.rejects(pending,/retired/);
  f.runtime.disposeOwner('snippet.consumer');f.runtime.forExtension('snippet.consumer');
  assert.equal(f.queue.settle({id:f.frames[0].id,result:{document:17,editor:'right',applied:true}}),true);
  await rejected;assert.equal(f.frames.length,1);assert.equal(f.api.workspace.textDocuments[0].getText(),'猫🙂\r\nsecond');
  f.queue.shutdown();
});

const badResults=[{document:18,editor:'right',applied:false},{document:17,editor:'wrong',applied:false},
  {document:17,editor:'right'},{document:17,applied:false},{editor:'right',applied:false},
  {document:17,editor:'right',applied:false,extra:0},{document:17,editor:'right',applied:0}];

test('actual facade retains all eight service slots through 5s timeout and malformed ACKs until exact false terminal',async()=>{
  const f=publicFixture(),failures=[];
  try{
    for(let index=0;index<8;index++)failures.push(assert.rejects(f.editor.insertSnippet(new SnippetString('x')),/outcome is unknown/));
    assert.equal(f.frames.length,8);
    await assert.rejects(f.editor.insertSnippet(new SnippetString('ninth')),/queue limit/);
    assert.equal(f.frames.length,8);
    for(const timer of f.timers){assert.equal(timer.delay,5000);timer.callback();}
    await Promise.all(failures);
    for(const result of badResults)assert.equal(f.queue.settle({id:f.frames[0].id,result}),false);
    assert.equal(f.queue.settle({id:f.frames[0].id,result:{document:17,editor:'right',applied:false},error:{message:'ambiguous'}}),false);
    await assert.rejects(f.editor.insertSnippet(new SnippetString('still occupied')),/queue limit/);
    assert.equal(f.frames.length,8);assert.equal(f.api.workspace.textDocuments[0].getText(),'猫🙂\r\nsecond');
    assert.equal(f.queue.settle({id:f.frames[0].id,result:{document:17,editor:'right',applied:false}}),true);
    const next=f.editor.insertSnippet(new SnippetString('fresh'));
    assert.equal(f.frames.length,9);assert.equal(f.queue.settle({id:f.frames[8].id,result:{document:17,editor:'right',applied:false}}),true);
    assert.equal(await next,false);assert.equal(f.api.workspace.textDocuments[0].getText(),'猫🙂\r\nsecond');
  }finally{f.queue.shutdown();}
});

test('framing retains all 64 occupied snippet requests after timeout; wrong ACK never releases capacity',async()=>{
  const frames=[],timers=[];let terminal=0;
  const queue=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:51,owner:'command.owner'}),{
    setTimeout(callback,delay){assert.equal(delay,5000);timers.push(callback);return callback;},clearTimeout(){},
  });
  const params={owner:'consumer',args:{document:17,editor:'right'}},failures=[];
  try{
    for(let index=0;index<64;index++){
      const lifecycle={onTerminal(){terminal++;}};
      failures.push(assert.rejects(queue.request('nativeEditorInsertSnippet',params,lifecycle),/outcome is unknown/));
      assert.equal(lifecycle.retained,true);
    }
    for(const callback of timers)callback();await Promise.all(failures);
    await assert.rejects(queue.request('nativeEditorInsertSnippet',params),/request limit/);
    assert.equal(frames.length,64);assert.equal(terminal,0);assert.equal(frames[0].params.owner,'consumer');assert.equal(frames[0].params.commandOwner,'command.owner');
    for(const result of badResults)assert.equal(queue.settle({id:frames[0].id,result}),false);
    assert.equal(terminal,0);await assert.rejects(queue.request('nativeEditorInsertSnippet',params),/request limit/);
    assert.equal(queue.settle({id:frames[0].id,result:{document:17,editor:'right',applied:false}}),true);assert.equal(terminal,1);
    assert.equal(queue.settle({id:frames[0].id,result:{document:17,editor:'right',applied:false}}),false);assert.equal(terminal,1);
    const fresh=queue.request('nativeEditorInsertSnippet',params,{onTerminal(){terminal++;}});
    assert.equal(frames.length,65);assert.equal(queue.settle({id:frames[64].id,result:{document:17,editor:'right',applied:false}}),true);
    assert.deepEqual(await fresh,{document:17,editor:'right',applied:false});assert.equal(terminal,2);
  }finally{queue.shutdown();}
});
