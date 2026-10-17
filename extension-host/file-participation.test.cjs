'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri,Range}=require('./api-types.cjs');
const {WorkspaceEdit}=require('./edit-values.cjs');
const turn=()=>new Promise(resolve=>setImmediate(resolve));
const deferred=()=>{let resolve;const promise=new Promise(done=>resolve=done);return{resolve,promise};};
const uri=Uri.file('/tmp/a.cpp');
function request(id=1,timeout=1000){return{session:7,request:id,timeout,frame:{session:7,sequence:id,kind:'create',files:[{target:uri.toString()}]}};}
function fixture(enabled=true){
 const frames=[],runtime=createApi(async()=>null,(method,params)=>frames.push({method,params}),{session:7,fileParticipants:enabled});
 return{runtime,api:runtime.forExtension('creator'),frames};
}
function releases(frames){return frames.filter(frame=>frame.method==='fileParticipantReleased').map(frame=>frame.params);}

test('negotiated public will subscriptions deliver full native participant DTOs with actual completion',async()=>{
 const {runtime,api,frames}=fixture();let calls=0;
 api.workspace.onWillCreateFiles(event=>{
  calls++;assert.equal(event.files[0].fsPath,'/tmp/a.cpp');
  const edit=new WorkspaceEdit();edit.replace(uri,new Range(0,0,0,0),'猫🙂');
  edit.createFile(Uri.file('/tmp/b.cpp'),{contents:new Uint8Array([0,1,255])},{label:'binary',needsConfirmation:true});
  event.waitUntil(Promise.resolve(edit));
 });
 const value=await runtime.fileParticipation.provide(request());await turn();
 assert.equal(calls,1);assert.equal(value.edit.edits[0].textEdit.text,'猫🙂');
 assert.equal(value.edit.edits[1].options.contents.value,'AAH/');assert.equal(value.edit.edits[1].metadata.needsConfirmation,true);
 assert.deepEqual(value.extensionNames,['creator']);assert.equal(runtime.fileParticipation.busy(),false);
 assert.deepEqual(releases(frames),[{session:7,request:1,accepted:true}]);
});
test('cancel races the logical reply but late contribution getters and actual capacity remain fenced',async()=>{
 const {runtime,api,frames}=fixture(),held=deferred();let token,getters=0;
 api.workspace.onWillCreateFiles(event=>{token=event.token;event.waitUntil(held.promise);});
 const reply=runtime.fileParticipation.provide(request());
 assert.equal(runtime.fileParticipation.cancel({session:8,request:1}),false);
 assert.equal(runtime.fileParticipation.cancel({session:7,request:2}),false);
 assert.equal(runtime.fileParticipation.cancel({session:7,request:1,extra:1}),false);
 assert.equal(token.isCancellationRequested,false);
 assert.equal(runtime.fileParticipation.cancel({session:7,request:1}),true);assert.equal(await reply,undefined);
 assert.equal(token.isCancellationRequested,true);assert.equal(runtime.fileParticipation.busy(),true);assert.deepEqual(releases(frames),[]);
 const edit=new WorkspaceEdit();edit.createFile(uri,undefined,{get label(){getters++;return'late';}});held.resolve(edit);await turn();
 assert.equal(getters,0);assert.equal(runtime.fileParticipation.busy(),false);
 assert.deepEqual(releases(frames),[{session:7,request:1,accepted:true}]);
});
test('duplicate and malformed live identities cannot release original unfinished work',async()=>{
 const {runtime,api,frames}=fixture(),held=deferred();
 api.workspace.onWillCreateFiles(event=>event.waitUntil(held.promise));
 const reply=runtime.fileParticipation.provide(request());
 assert.throws(()=>runtime.fileParticipation.provide(request()),/Duplicate/);
 assert.throws(()=>runtime.fileParticipation.provide({...request(),frame:null}),/Invalid/);
 assert.deepEqual(releases(frames),[]);
 assert.throws(()=>runtime.fileParticipation.provide(request(2)),/occupied/);
 assert.deepEqual(releases(frames),[{session:7,request:2,accepted:false}]);assert.equal(runtime.fileParticipation.busy(),true);
 held.resolve();await reply;await turn();assert.deepEqual(releases(frames).at(-1),{session:7,request:1,accepted:true});
});
test('malformed and unsupported admissions acknowledge only work known not to have begun',()=>{
 const {runtime,api,frames}=fixture(false);
 assert.throws(()=>api.workspace.onWillCreateFiles,/not implemented/);
 assert.throws(()=>runtime.fileParticipation.provide(request()),/unavailable/);
 assert.throws(()=>runtime.fileParticipation.provide({...request(2),extra:true}),/Invalid/);
 assert.throws(()=>runtime.fileParticipation.provide({...request(3),session:8}),/Invalid/);
 assert.deepEqual(releases(frames),[{session:7,request:1,accepted:false},{session:7,request:2,accepted:false}]);
});
test('logical failure does not release a callback that threw after admitting unfinished waits',async()=>{
 const {runtime,api,frames}=fixture(),held=deferred(),savedError=console.error;console.error=()=>{};
 try{
  api.workspace.onWillCreateFiles(event=>{event.waitUntil(held.promise);throw new Error('callback failed');});
  assert.equal(await runtime.fileParticipation.provide(request()),undefined);assert.equal(runtime.fileParticipation.busy(),true);assert.deepEqual(releases(frames),[]);
  held.resolve();await turn();assert.equal(runtime.fileParticipation.busy(),false);assert.deepEqual(releases(frames),[{session:7,request:1,accepted:true}]);
 }finally{held.resolve();console.error=savedError;}
});
test('owner disposal cannot reacquire authority through a held native participant result',async()=>{
 const {runtime,api,frames}=fixture(),held=deferred(),savedError=console.error;console.error=()=>{};
 try{
  api.workspace.onWillCreateFiles(event=>event.waitUntil(held.promise));const reply=runtime.fileParticipation.provide(request());
  runtime.disposeOwner('creator');runtime.forExtension('creator');held.resolve(new WorkspaceEdit());
  assert.equal(await reply,undefined);await turn();assert.deepEqual(releases(frames),[{session:7,request:1,accepted:true}]);
 }finally{held.resolve();console.error=savedError;}
});
test('early timeout re-arms against its original instant and cancellation retains real occupancy',async()=>{
 const savedSet=global.setTimeout,savedClear=global.clearTimeout,descriptor=Object.getOwnPropertyDescriptor(performance,'now'),timers=[];
 let clock=100,token;const held=deferred();
 Object.defineProperty(performance,'now',{configurable:true,value:()=>clock});
 global.setTimeout=(callback,delay)=>{const timer={callback,delay,cleared:false};timers.push(timer);return timer;};global.clearTimeout=timer=>{if(timer)timer.cleared=true;};
 try{
  const {runtime,api,frames}=fixture();api.workspace.onWillCreateFiles(event=>{token=event.token;event.waitUntil(held.promise);});
  let completed=false;const reply=runtime.fileParticipation.provide(request(1,30));reply.then(()=>completed=true);
  clock=129.5;timers[0].callback();await turn();assert.equal(completed,false);assert.equal(token.isCancellationRequested,false);assert.equal(timers[1].delay,1);
  clock=130;timers[1].callback();assert.equal(await reply,undefined);assert.equal(token.isCancellationRequested,true);
  assert.equal(runtime.fileParticipation.busy(),true);assert.deepEqual(releases(frames),[]);
  held.resolve();await turn();assert.deepEqual(releases(frames),[{session:7,request:1,accepted:true}]);
 }finally{held.resolve();global.setTimeout=savedSet;global.clearTimeout=savedClear;if(descriptor)Object.defineProperty(performance,'now',descriptor);else delete performance.now;}
});
test('bad extension display names cannot leave a partially accepted contribution behind',async()=>{
 const {createFileParticipants}=require('./file-participants.cjs'),{createCallerContext}=require('./caller-context.cjs');
 const callers=createCallerContext(()=>{}),errors=[];
 const engine=createFileParticipants({session:7,bind:callers.bind,captureCaller:owner=>callers.bind(owner,()=>{}),versionInfo:()=>({}),extensionName:()=>({}),report:error=>errors.push(error.message)});
 const value=new WorkspaceEdit();value.replace(uri,new Range(0,0,0,0),'must not escape');
 engine.forOwner('creator').onWillCreateFiles(event=>event.waitUntil(Promise.resolve(value)));
 const run=engine.fire(request().frame,1000);assert.equal(await run.result,undefined);await run.settled;assert.deepEqual(errors,['Invalid file participant extension name']);
});
test('private target provenance captures conversion-time identity and versions despite public document spoofing',async()=>{
 const {runtime,api}=fixture(),held=deferred();
 const state=(generation,version,text)=>({generation,documents:[{id:7,uri:uri.toString(),version,text,languageId:'cpp',isDirty:true}],active:null,selections:[]});
 runtime.sync(state(1,3,'old 猫🙂\r\n'));
 const document=api.workspace.textDocuments[0];
 Object.defineProperty(document,'version',{value:900,configurable:true});Object.defineProperty(document,'_id',{value:999,configurable:true});
 api.workspace.onWillCreateFiles(event=>event.waitUntil(held.promise));
 const response=runtime.fileParticipation.provide(request());runtime.sync(state(2,4,'edited while waiting 猫🙂\r\n'));
 const edit=new WorkspaceEdit();edit.replace(uri,new Range(0,0,0,0),'contribution');held.resolve(edit);
 const result=await response;
 assert.equal(document.version,900);assert.equal(result.edit.edits[0].versionId,4);
 assert.deepEqual(result.source,{owners:['creator'],models:[{uri:uri.toString(),document:7,version:4}]});
});
test('private target proofs follow per-edit version suppression and never invent authority for create targets',async()=>{
 const {runtime,api}=fixture();runtime.sync({generation:1,documents:[{id:7,uri:uri.toString(),version:3,text:'old',languageId:'cpp',isDirty:true}],active:null,selections:[]});
 api.workspace.onWillCreateFiles(event=>{
  const edit=new WorkspaceEdit();edit.replace(uri,new Range(0,0,0,0),'before create');edit.createFile(uri,{overwrite:true});event.waitUntil(Promise.resolve(edit));
 });
 const result=await runtime.fileParticipation.provide(request());
 assert.equal(result.edit.edits[0].versionId,undefined);assert.deepEqual(result.source,{owners:['creator'],models:[]});
});
