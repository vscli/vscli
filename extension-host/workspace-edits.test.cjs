'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createWorkspaceEdits } = require('./workspace-edits.cjs');
const { WorkspaceEdit, TextEdit, SnippetTextEdit, NotebookEdit, NotebookRange } = require('./edit-values.cjs');
const { Uri, Range, Position } = require('./api-types.cjs');
const { SnippetString } = require('./snippet-string.cjs');
const { createCallerContext } = require('./caller-context.cjs');
const { createDocumentServices } = require('./document-services.cjs');
const { createNativeRequests } = require('./native-requests.cjs');
const { createApi } = require('./api.cjs');
const a = Uri.file('/tmp/apply-猫.cpp'), b = Uri.parse('untitled:vscli-2');
const range = () => new Range(0, 1, 0, 1);
function deferred() { let resolve, reject; const promise = new Promise((yes,no) => { resolve=yes; reject=no; }); return {promise,resolve,reject}; }
function fixture(call) {
  let lease = {}, policyIdentity = {}, autoSave = true;
  const mirrors = new Map([[1,{document:1,version:3,uri:a.toString(),model:{}}], [2,{document:2,version:7,uri:b.toString(),model:{}}]]);
  const calls = [], notices = [];
  const model = { snapshots: () => mirrors.values(), snapshot: id => mirrors.get(id),
    captureCaller: () => { const captured = lease; return () => { if(captured!==lease) throw new Error('retired caller'); }; },
    refactoringPolicy: () => ({ identity:policyIdentity, autoSave }) };
  const api = createWorkspaceEdits(async (...args) => { calls.push(args); return call ? call(...args) : {applied:true}; },
    {notice:(owner,text)=>notices.push([owner,text])},model).forOwner('fixture.owner');
  return {api, calls, notices, mirrors, model, retire(){lease={};}, settings(){policyIdentity={};}, autoSave(value){autoSave=value;}};
}
function edit(uri=a,text='λ') { const value=new WorkspaceEdit(); value.insert(uri,new Position(0,1),text); return value; }
test('brand and empty conversion never invokes arbitrary entries or requests native work', async () => {
  const f=fixture(); let invoked=0;
  assert.equal(await f.api.applyEdit({get entries(){invoked++;throw new Error('called');}}),false);
  assert.equal(await f.api.applyEdit(new WorkspaceEdit()),false);
  assert.equal(invoked,0); assert.equal(f.calls.length,0);
});
test('grouping retains first URI order and exact equal-insert order without exposing public values', async () => {
  const f=fixture(),value=edit(); value.insert(b,new Position(0,2),'🙂'); value.insert(a,new Position(0,1),'β');
  assert.equal(await f.api.applyEdit(value),true);
  assert.deepEqual(f.calls[0],['fixture.owner','nativeWorkspaceApplyEdit',{entries:[
    {document:1,version:3,uri:a.toString(),edits:[{range:{start:{line:0,character:1},end:{line:0,character:1}},newText:'λ'},
      {range:{start:{line:0,character:1},end:{line:0,character:1}},newText:'β'}]},
    {document:2,version:7,uri:b.toString(),edits:[{range:{start:{line:0,character:2},end:{line:0,character:2}},newText:'🙂'}]}],metadata:{isRefactoring:false}}]);
});
test('URI formatter overrides cannot retarget captured authority', async () => {
  const f=fixture(),value=edit(); let calls=0;
  Object.defineProperty(a,'toString',{configurable:true,value(){calls++;return b.toString();}});
  try { assert.equal(await f.api.applyEdit(value),true); assert.equal(f.calls[0][2].entries[0].document,1); assert.equal(calls,0); }
  finally { delete a.toString; }
});
test('text/range/endpoint getters are captured once and serialization never revisits them', async () => {
  const f=fixture(),value=new WorkspaceEdit(); let text=0,start=0,end=0,line=0,character=0;
  const point={get line(){line++;return 0;},get character(){character++;return 1;}};
  const bounds={get start(){start++;return point;},get end(){end++;return point;}};
  value.set(a,[{get range(){return bounds;},get newText(){text++;return '猫🙂\r\n';}}]);
  text=start=end=line=character=0;
  assert.equal(await f.api.applyEdit(value),true);
  assert.deepEqual([text,start,end,line,character],[1,1,1,2,2]);
});
for (const kind of ['file','snippet','notebook','eol','annotation','outside']) test(`unsupported ${kind} refuses entire request and retains valid peer`, async () => {
  const f=fixture(),value=edit();
  if(kind==='file') value.createFile(b);
  if(kind==='snippet') value.set(b,[new SnippetTextEdit(range(),new SnippetString('${1:x}'))]);
  if(kind==='notebook') value.set(b,[new NotebookEdit(new NotebookRange(0,1),[])]);
  if(kind==='eol') value.set(b,[TextEdit.setEndOfLine(2)]);
  if(kind==='annotation') value.replace(b,range(),'x',{label:'requires approval'});
  if(kind==='outside') value.insert(Uri.file('/tmp/not-mirrored'),new Position(0,0),'x');
  assert.equal(await f.api.applyEdit(value),false); assert.equal(f.calls.length,0); assert.equal(f.notices.length,1);
  assert.equal(value.get(a)[0].newText,'λ');
});
test('isRefactoring is captured once and default automatic-save policy is explicitly refused', async () => {
  const f=fixture(); let count=0;
  assert.equal(await f.api.applyEdit(edit(),{get isRefactoring(){count++;return true;}}),false);
  assert.equal(count,1); assert.equal(f.calls.length,0); assert.match(f.notices[0][1],/automatic save/);
  f.autoSave(false); assert.equal(await f.api.applyEdit(edit(),{isRefactoring:true}),true);
  assert.equal(f.calls[0][2].metadata.isRefactoring,true);
});
for(const mutation of ['lease','version','identity','uri','configuration','edit']) test(`getter ${mutation} mutation cannot publish a native request`,async()=>{
  const f=fixture(),value=new WorkspaceEdit();
  value.set(a,[{range:range(),get newText(){
    if(mutation==='lease')f.retire();
    if(mutation==='configuration')f.settings();
    if(mutation==='version')f.mirrors.get(1).version++;
    if(mutation==='identity')f.mirrors.get(1).model={};
    if(mutation==='uri')f.mirrors.get(1).uri=b.toString();
    if(mutation==='edit')value.insert(b,new Position(0,0),'reentrant');
    return 'x';
  }}]);
  await assert.rejects(f.api.applyEdit(value),/retired|changed/); assert.equal(f.calls.length,0);
});
test('metadata retirement is checked even when the whole request is unsupported',async()=>{
  const f=fixture(),value=edit();value.createFile(b);
  await assert.rejects(f.api.applyEdit(value,{get isRefactoring(){f.retire();return false;}}),/retired/);
  assert.equal(f.calls.length,0);assert.equal(f.notices.length,0);
});
test('raw and escaped text limits reject before dispatch; no partial peer survives',async()=>{
  const f=fixture(); await assert.rejects(f.api.applyEdit(edit(a,'x'.repeat(1024*1024+1))),/1 MiB/);
  await assert.rejects(f.api.applyEdit(edit(a,'\0'.repeat(400000))),/2 MiB/); assert.equal(f.calls.length,0);
});
test('noninteger UTF16 coordinates reject rather than clamp; source values remain untouched',async()=>{
  const f=fixture(),value=new WorkspaceEdit(); value.set(a,[{range:{start:{line:0,character:1.5},end:{line:0,character:2}},newText:'x'}]);
  await assert.rejects(f.api.applyEdit(value),/uintegers/); assert.equal(value.get(a)[0].range.start.character,1.5);assert.equal(f.calls.length,0);
});
test('genuine application refusal returns false with bounded notice; malformed acknowledgement rejects',async()=>{
  const f=fixture(()=>({applied:false,reason:'conflicting edit'}));assert.equal(await f.api.applyEdit(edit()),false);assert.equal(f.notices[0][1],'conflicting edit');
  const malformed=fixture(()=>({applied:1}));await assert.rejects(malformed.api.applyEdit(edit()),/acknowledgement/);
});
test('own native version advancement is allowed but closed/replaced original model rejects success',async()=>{
  const f=fixture(()=>{f.mirrors.get(1).version++;return {applied:true};});assert.equal(await f.api.applyEdit(edit()),true);
  const closed=fixture(()=>{closed.mirrors.delete(1);return {applied:true};});await assert.rejects(closed.api.applyEdit(edit()),/mirrors/);
});
test('retirement while awaiting exact receipt never reacquires a replacement caller lease',async()=>{
  const held=deferred(),f=fixture(()=>held.promise);const pending=f.api.applyEdit(edit());f.retire();held.resolve({applied:true});
  await assert.rejects(pending,/retired/);assert.equal(f.calls.length,1);
});
test('real caller context rejects old descendants after owner retire/reacquire',async()=>{
  const callers=createCallerContext(()=>{}),f=fixture();f.model.captureCaller=owner=>callers.bind(owner,()=>{});
  const api=createWorkspaceEdits((...args)=>{f.calls.push(args);return Promise.resolve({applied:true});},{},f.model).forOwner('A');
  let old;callers.run('A',()=>{old=callers.bind('A',()=>api.applyEdit(edit()));});callers.retire('A');callers.run('A',()=>{});
  assert.throws(()=>old(),/retired/);assert.equal(f.calls.length,0);
});
function clock(){let id=0;const entries=new Map();return {entries,setTimeout(fn,ms){entries.set(++id,{fn,ms});return id;},clearTimeout(id){entries.delete(id);},fire(id){const value=entries.get(id);entries.delete(id);value.fn();}};}
function bridge(){const frames=[],timers=clock();const requests=createNativeRequests(message=>frames.push(JSON.parse(JSON.stringify(message))),()=>({active:true,id:17,owner:'command.owner'}),timers);
 const services=createDocumentServices(requests.request,{session:4},{generation:()=>9,root:()=>'/tmp'});return {frames,timers,requests,services};}
test('eight leases are shared with Open/Show/Save and retained after ApplyEdit timeout until exact valid receipt',async()=>{
 const f=bridge(),waiting=[];
 for(let i=0;i<8;i++)waiting.push(f.services.call('edit.owner','nativeWorkspaceApplyEdit',{entries:[],metadata:{isRefactoring:false}}).catch(error=>error.message));
 assert.equal(f.frames.length,8);assert.equal(f.frames[0].params.commandOwner,'command.owner');assert.equal(f.frames[0].params.owner,'edit.owner');
 assert.equal(f.timers.entries.get(1).ms,5000);
 const first=f.frames[0].id;f.timers.fire(1);assert.match(await waiting[0],/outcome is unknown/);
 await assert.rejects(f.services.call('edit.owner','nativeDocumentOpen',{path:'/tmp/a'}),/queue limit/);
 assert.equal(f.requests.settle({id:first,result:true}),false);
 await assert.rejects(f.services.call('edit.owner','nativeDocumentSave',{document:1,version:1}),/queue limit/);
 assert.equal(f.requests.settle({id:first,result:{applied:false,reason:'retired'}}),true);
 const replacement=f.services.call('edit.owner','nativeDocumentOpen',{path:'/tmp/a'}).catch(error=>error.message);assert.equal(f.frames.length,9);
 f.requests.shutdown();await Promise.all([...waiting,replacement]);
});
test('64 actual framing slots stay occupied after timeout; malformed/duplicate/wrong-id responses do not release',async()=>{
 const f=bridge(),waiting=[];
 for(let i=0;i<64;i++)waiting.push(f.requests.request('nativeWorkspaceApplyEdit',{owner:'A',args:{entries:[],metadata:{isRefactoring:false}}}).catch(error=>error.message));
 for(const id of [...f.timers.entries.keys()])f.timers.fire(id);
 await Promise.all(waiting);await assert.rejects(f.requests.request('nativeWorkspaceApplyEdit',{}),/request limit/);
 assert.equal(f.requests.settle({id:'wrong',result:{applied:false}}),false);
 assert.equal(f.requests.settle({id:f.frames[0].id,result:{applied:true},error:{message:'ambiguous'}}),false);
 assert.equal(f.requests.settle({id:f.frames[0].id,result:{applied:false}}),true);
 assert.equal(f.requests.settle({id:f.frames[0].id,result:{applied:false}}),false);
 const one=f.requests.request('nativeWorkspaceApplyEdit',{}).catch(error=>error.message);assert.equal(f.frames.length,65);f.requests.shutdown();await one;
});
test('queued serialized payload is bounded separately from count and stays charged while timed out',async()=>{
 const f=bridge(),waiting=[],args={entries:[{document:1,version:1,uri:a.toString(),edits:[{range:{start:{line:0,character:0},end:{line:0,character:0}},newText:'x'.repeat(1024*1024)}]}],metadata:{isRefactoring:false}};
 for(let i=0;i<7;i++)waiting.push(f.services.call('A','nativeWorkspaceApplyEdit',args).catch(error=>error.message));
 await assert.rejects(f.services.call('A','nativeWorkspaceApplyEdit',args),/payload limit/);
 f.timers.fire(1);await waiting[0];await assert.rejects(f.services.call('A','nativeWorkspaceApplyEdit',args),/payload limit/);
 f.requests.shutdown();await Promise.all(waiting);
});
test('framed receipt applies only after complete mirror publication; optional facade remains gated',async()=>{
 let runtime;const frames=[];const initial={generation:1,documents:[{id:1,uri:a.toString(),version:1,text:'猫🙂\r\n',languageId:'cpp',isDirty:true},{id:2,uri:b.toString(),version:1,text:'x\r\n',languageId:'plaintext',isDirty:true}],active:null,selections:[]};
 const blocked=createApi(()=>{throw new Error('unqualified request');},()=>{},{session:4});blocked.sync(initial);
 assert.throws(()=>blocked.forExtension('A').workspace.applyEdit,/not implemented/);
 const timers=clock(),transport=createNativeRequests(message=>{
   const body=Buffer.from(JSON.stringify(message)),header=Buffer.from(`Content-Length: ${body.length}\r\n\r\n`);const frame=Buffer.concat([header,body]);
   frames.push(JSON.parse(frame.subarray(frame.indexOf('\r\n\r\n')+4).toString()));
 },()=>({active:true,id:5,owner:'A'}),timers);
 runtime=createApi(transport.request,()=>{},{session:4,workspaceEdits:true});runtime.sync(initial);
 const api=runtime.forExtension('A'),identity=api.workspace.textDocuments[0],second=api.workspace.textDocuments[1],events=[],value=edit(a,'λ');
 value.insert(b,new Position(0,1),'β');
 api.workspace.onDidChangeTextDocument(event=>{events.push(event.document);assert.equal(identity.getText(),'猫λ🙂\r\n');assert.equal(second.getText(),'xβ\r\n');});
 const pending=api.workspace.applyEdit(value).then(result=>{assert.deepEqual(events,[identity,second]);return result;});
 assert.equal(frames.length,1);runtime.sync({...initial,generation:2,documents:[{...initial.documents[0],version:2,text:'猫λ🙂\r\n'},{...initial.documents[1],version:2,text:'xβ\r\n'}]});
 transport.settle(JSON.parse(JSON.stringify({id:frames[0].id,result:{applied:true}})));
 assert.equal(await pending,true);assert.equal(api.workspace.textDocuments[0],identity);assert.equal(identity.getText(),'猫λ🙂\r\n');transport.shutdown();
});
test('public document._snapshot mutation during an edit getter cannot change private wire identity',async()=>{
 const calls=[],runtime=createApi(async(method,params)=>{calls.push([method,params]);return {applied:false};},()=>{},{session:4,workspaceEdits:true});
 runtime.sync({generation:1,documents:[{id:1,uri:a.toString(),version:1,text:'猫🙂\r\n',languageId:'cpp',isDirty:true}],active:null,selections:[]});
 const api=runtime.forExtension('A'),doc=api.workspace.textDocuments[0],value=new WorkspaceEdit();value.set(a,[{range:range(),get newText(){doc._snapshot={id:999,version:999,uri:b.toString()};return 'λ';}}]);
 assert.equal(await api.workspace.applyEdit(value),false);assert.equal(calls[0][1].args.entries[0].document,1);assert.equal(calls[0][1].args.entries[0].version,1);
});

test('mirror inventory overflow refuses before any extension getters; exactly 128 targets stay bounded',async()=>{
 const f=fixture();for(let id=3;id<=129;id++)f.mirrors.set(id,{document:id,version:1,uri:Uri.file(`/tmp/target-${id}`).toString(),model:{}});
 let calls=0;const metadata={get isRefactoring(){calls++;return false;}};
 await assert.rejects(f.api.applyEdit(edit(),metadata),/exceed 128/);assert.equal(calls,0);assert.equal(f.calls.length,0);
 f.mirrors.delete(129);assert.equal(await f.api.applyEdit(edit()),true);
});
test('Save retains its existing 360 second clock and exact boolean receipt independently of ApplyEdit',async()=>{
 const f=bridge(),saved=f.services.call('A','nativeDocumentSave',{document:1,version:1});
 assert.equal(f.timers.entries.get(1).ms,360000);assert.equal(f.requests.settle({id:f.frames[0].id,result:{applied:true}}),false);
 assert.equal(f.requests.settle({id:f.frames[0].id,result:true}),true);assert.equal(await saved,true);
 const next=f.services.call('A','nativeWorkspaceApplyEdit',{entries:[],metadata:{isRefactoring:false}});
 assert.equal(f.timers.entries.get(2).ms,5000);assert.equal(f.requests.settle({id:f.frames[1].id,result:{applied:false}}),true);assert.deepEqual(await next,{applied:false});
});

test('unpaired UTF16 text rejects before framing; genuine emoji pairs survive exactly',async()=>{
 const f=fixture();await assert.rejects(f.api.applyEdit(edit(a,'\ud800')),/Unicode scalars/);await assert.rejects(f.api.applyEdit(edit(a,'\udc00')),/Unicode scalars/);
 assert.equal(f.calls.length,0);assert.equal(await f.api.applyEdit(edit(a,'🙂')),true);assert.equal(f.calls[0][2].entries[0].edits[0].newText,'🙂');
});
test('circular unsupported annotations are refused without serializing or invoking toJSON',async()=>{
 const f=fixture(),value=edit(),metadata={get toJSON(){throw new Error('must not serialize');}};metadata.self=metadata;value.replace(b,range(),'x',metadata);
 assert.equal(await f.api.applyEdit(value),false);assert.equal(f.calls.length,0);
});
test('oversized terminal reason/error cannot release retained native request',async()=>{
 const f=bridge(),pending=f.services.call('A','nativeWorkspaceApplyEdit',{entries:[],metadata:{isRefactoring:false}}).catch(error=>error.message),id=f.frames[0].id;
 assert.equal(f.requests.settle({id,result:{applied:false,reason:'x'.repeat(4097)}}),false);
 assert.equal(f.requests.settle({id,error:{message:'x'.repeat(4097)}}),false);
 assert.equal(f.requests.settle({id,error:{message:'retired owner'}}),true);assert.equal(await pending,'retired owner');
});

test('real API owner retirement/reacquisition during metadata capture rejects the old facade lease',async()=>{
 const calls=[],runtime=createApi(async(...args)=>{calls.push(args);return {applied:true};},()=>{},{session:4,workspaceEdits:true});
 runtime.sync({generation:1,documents:[{id:1,uri:a.toString(),version:1,text:'猫🙂\r\n',languageId:'cpp',isDirty:true}],active:null,selections:[]});
 const api=runtime.forExtension('A');
 await assert.rejects(api.workspace.applyEdit(edit(),{get isRefactoring(){runtime.disposeOwner('A');runtime.runWithOwner('A',()=>{});return false;}}),/retired/);
 assert.equal(calls.length,0);
});
test('real mirrored edit-Undo version advancement during getters rejects the original capture',async()=>{
 const calls=[],runtime=createApi(async(...args)=>{calls.push(args);return {applied:true};},()=>{},{session:4,workspaceEdits:true});
 const initial={generation:1,documents:[{id:1,uri:a.toString(),version:1,text:'猫🙂\r\n',languageId:'cpp',isDirty:true}],active:null,selections:[]};runtime.sync(initial);
 const api=runtime.forExtension('A');
 await assert.rejects(api.workspace.applyEdit(edit(),{get isRefactoring(){runtime.sync({...initial,generation:2,documents:[{...initial.documents[0],version:3}]});return false;}}),/version changed/);
 assert.equal(calls.length,0);assert.equal(api.workspace.textDocuments[0].getText(),initial.documents[0].text);
});

test('full escaped frame envelope is charged before admission, including original command owner',async()=>{
 const frames=[],timers=clock(),requests=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:1,owner:'x'.repeat(4096)}),timers);
 const args={entries:[{document:1,version:1,uri:a.toString(),edits:[{range:{start:{line:0,character:0},end:{line:0,character:0}},newText:'x'.repeat(2*1024*1024-512)}]}],metadata:{isRefactoring:false}},lifecycle={retained:false,onTerminal(){throw new Error('not occupied');}};
 await assert.rejects(requests.request('nativeWorkspaceApplyEdit',{session:1,owner:'A',args},lifecycle),/wire payload limit/);
 assert.equal(frames.length,0);assert.equal(timers.entries.size,0);assert.equal(lifecycle.retained,false);requests.shutdown();
});
test('live cross-owner delegation keeps caller input getters in original scope and wire owner explicit',async()=>{
 const calls=[],runtime=createApi(async(...args)=>{calls.push(args);return {applied:false};},()=>{},{session:4,workspaceEdits:true});
 runtime.sync({generation:1,documents:[{id:1,uri:a.toString(),version:1,text:'猫🙂\r\n',languageId:'cpp',isDirty:true}],active:null,selections:[]});
 const api=runtime.forExtension('A'),value=new WorkspaceEdit();value.set(a,[{range:range(),get newText(){assert.equal(runtime.currentCallerOwner(),'B');return 'λ';}}]);
 assert.equal(await runtime.runWithOwner('B',()=>api.workspace.applyEdit(value)),false);assert.equal(calls[0][1].owner,'A');
});

for(const brand of ['unknown','empty']) {
 test(`${brand} conversion evaluates metadata getter once before local false`,async()=>{
  const f=fixture();let metadataReads=0,editReads=0;
  const value=brand==='empty'?new WorkspaceEdit():{get entries(){editReads++;throw new Error('not an API brand');}};
  assert.equal(await f.api.applyEdit(value,{get isRefactoring(){metadataReads++;return true;}}),false);
  assert.equal(metadataReads,1);assert.equal(editReads,0);assert.equal(f.calls.length,0);assert.equal(f.notices.length,0);
 });
 test(`${brand} metadata getter errors remain rejected rather than becoming false`,async()=>{
  const f=fixture(),error=new Error('original metadata failure');
  await assert.rejects(f.api.applyEdit(brand==='empty'?new WorkspaceEdit():{}, {get isRefactoring(){throw error;}}),actual=>actual===error);
  assert.equal(f.calls.length,0);assert.equal(f.notices.length,0);
 });
 test(`${brand} metadata getter owner retirement never reacquires a fresh lease`,async()=>{
  const f=fixture();await assert.rejects(f.api.applyEdit(brand==='empty'?new WorkspaceEdit():{}, {get isRefactoring(){f.retire();return false;}}),/retired/);
  assert.equal(f.calls.length,0);assert.equal(f.notices.length,0);
 });
}
for(const [name,metadata] of [['undefined',undefined],['null',null],['number',3],['zero',0],['false',false],['string','metadata'],['symbol',Symbol('metadata')],['array',[]]]) {
 test(`optional metadata ${name} follows property-access semantics without object rejection`,async()=>{
  const f=fixture();assert.equal(await f.api.applyEdit(edit(),metadata),true);
  assert.equal(f.calls[0][2].metadata.isRefactoring,false);
 });
}
for(const [name,value,expected] of [['undefined',undefined,false],['null',null,false],['zero',0,false],['negative-zero',-0,false],['NaN',NaN,false],['empty-string','',false],['false',false,false],['one',1,true],['negative-one',-1,true],['false-string','false',true],['empty-array',[],true],['object',{},true],['true',true,true]]) {
 test(`metadata flag ${name} normalizes downstream JS truthiness once to native boolean`,async()=>{
  const f=fixture();f.autoSave(false);let reads=0;
  assert.equal(await f.api.applyEdit(edit(),{get isRefactoring(){reads++;return value;}}),true);
  assert.equal(reads,1);assert.equal(f.calls[0][2].metadata.isRefactoring,expected);
 });
}
test('truthiness normalization never invokes extension valueOf, toString or toJSON',async()=>{
 const f=fixture();f.autoSave(false);const value={get valueOf(){throw new Error('coercion');},get toString(){throw new Error('coercion');},get toJSON(){throw new Error('opaque wire graph');}};
 assert.equal(await f.api.applyEdit(edit(),{isRefactoring:value}),true);assert.equal(f.calls[0][2].metadata.isRefactoring,true);
});
test('text conversion completes before metadata evaluation, and conversion failure skips metadata',async()=>{
 const f=fixture(),value=new WorkspaceEdit(),events=[];value.set(a,[{range:range(),get newText(){events.push('text');return 'λ';}}]);
 assert.equal(await f.api.applyEdit(value,{get isRefactoring(){events.push('metadata');return false;}}),true);assert.deepEqual(events,['text','metadata']);
 const bad=new WorkspaceEdit(),error=new Error('conversion failed');let reads=0;bad.set(a,[{range:range(),get newText(){throw error;}}]);
 await assert.rejects(f.api.applyEdit(bad,{get isRefactoring(){reads++;return false;}}),actual=>actual===error);assert.equal(reads,0);
});
test('real unknown-brand metadata retirement rejects despite native owner reacquisition',async()=>{
 const calls=[],runtime=createApi(async(...args)=>{calls.push(args);return {applied:true};},()=>{},{session:4,workspaceEdits:true});
 const api=runtime.forExtension('A');await assert.rejects(api.workspace.applyEdit({}, {get isRefactoring(){runtime.disposeOwner('A');runtime.runWithOwner('A',()=>{});return false;}}),/retired/);
 assert.equal(calls.length,0);
});
