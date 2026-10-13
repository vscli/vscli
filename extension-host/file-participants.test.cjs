'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const {createFileParticipants} = require('./file-participants.cjs');
const {createCallerContext} = require('./caller-context.cjs');
const {createApi} = require('./api.cjs');
const {Uri,Range} = require('./api-types.cjs');
const {CancellationTokenSource,CancellationToken} = require('./cancellation.cjs');
const {WorkspaceEdit,SnippetTextEdit,NotebookEdit} = require('./edit-values.cjs');
const {SnippetString} = require('./snippet-string.cjs');
const uri = Uri.parse('file:///tmp/source%20%F0%9F%99%82.cpp');
const other = Uri.parse('file:///tmp/destination.cpp');
const frame = (sequence=1,kind='rename') => ({session:7,sequence,kind,files:[kind==='rename'?{source:uri.toString(),target:other.toString()}:{target:uri.toString()}]});
const deferred = () => {let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};};
const tick = () => new Promise(resolve=>setImmediate(resolve));
function fixture() {
  const retired = new Set(), errors = [], versions = new Map();
  const callers = createCallerContext(owner=>{if(retired.has(owner))throw new Error('original caller retired');});
  const engine = createFileParticipants({session:7,bind:callers.bind,captureCaller:owner=>callers.bind(owner,()=>{}),
    versionInfo:()=>({getTextDocumentVersion:resource=>versions.get(resource.toString()),getNotebookDocumentVersion:()=>undefined}),
    extensionName:owner=>`Display ${owner}`,report:error=>errors.push(error.message)});
  return {engine,callers,errors,versions,retire(owner){retired.add(owner);callers.retire(owner);},renew(owner){retired.delete(owner);}};
}
function edit(text, resource=uri) {const value=new WorkspaceEdit();value.replace(resource,new Range(0,0,0,0),text);return value;}

test('participants deliver sequentially but collect one listener contributions in settlement order',async()=>{
  const {engine,callers,versions}=fixture(), first=deferred(), second=deferred(), order=[], events=[];
  versions.set(uri.toString(),3);
  engine.forOwner('one').onWillRenameFiles(function(event){
    assert.equal(this.name,'receiver');assert.equal(callers.currentOwner(),'one');order.push('one');events.push(event);
    assert.equal(Object.isFrozen(event),false);assert.ok(event.files[0].oldUri instanceof Uri);
    event.files.push({oldUri:uri,newUri:other});event.waitUntil(first.promise);event.waitUntil(second.promise);
    return edit('ignored return');
  },{name:'receiver'});
  engine.forOwner('two').onWillRenameFiles(event=>{order.push('two');events.push(event);assert.equal(event.files.length,2);event.waitUntil(Promise.resolve(edit('third')));});
  const run=engine.fire(frame(),5000);assert.equal(engine.busy(),true);assert.deepEqual(order,['one']);
  second.resolve(edit('second'));await tick();assert.deepEqual(order,['one']);
  versions.set(uri.toString(),9);first.resolve(edit('first'));
  const result=await run.result;await run.settled;
  assert.deepEqual(order,['one','two']);assert.notEqual(events[0],events[1]);assert.equal(events[0].files,events[1].files);
  assert.deepEqual(result.edit.edits.map(row=>row.textEdit.text),['second','first','third']);
  assert.deepEqual(result.edit.edits.map(row=>row.versionId),[9,9,9]);
  assert.deepEqual(result.extensionNames,['Display one','Display two']);assert.equal(engine.busy(),false);
  assert.throws(()=>events[0].waitUntil(Promise.resolve()),/can NOT be called asynchronous/);
});

test('synchronous registration only, ignored return values, rejected promises and callback errors are isolated',async()=>{
  const {engine,errors}=fixture();let saved;
  engine.forOwner('return').onWillCreateFiles(()=>edit('ignored'));
  engine.forOwner('invalid').onWillCreateFiles(event=>{saved=event;event.waitUntil(Promise.resolve({edits:[]}));event.waitUntil(Promise.reject(new Error('rejected contribution')));});
  engine.forOwner('throws').onWillCreateFiles(()=>{throw new Error('callback error');});
  engine.forOwner('valid').onWillCreateFiles(event=>event.waitUntil(Promise.resolve(edit('valid'))));
  const run=engine.fire(frame(1,'create'),5000);const result=await run.result;await run.settled;
  assert.deepEqual(errors,['rejected contribution','callback error']);assert.equal(result.edit.edits[0].textEdit.text,'valid');
  assert.throws(()=>saved.waitUntil(Promise.resolve()),/can NOT be called asynchronous/);
});

test('queued listener disposal and newly registered listeners follow AsyncEmitter snapshot semantics',async()=>{
  const {engine}=fixture(), gate=deferred(), calls=[];let later;
  engine.forOwner('one').onWillDeleteFiles(event=>{
    calls.push('one');later.dispose();engine.forOwner('added').onWillDeleteFiles(()=>calls.push('added'));
    event.waitUntil(gate.promise);
  });
  const disposables=[];later=engine.forOwner('two').onWillDeleteFiles(()=>calls.push('two'),undefined,disposables);
  assert.equal(disposables[0],later);
  const first=engine.fire(frame(1,'delete'),5000);gate.resolve();assert.equal(await first.result,undefined);await first.settled;
  assert.deepEqual(calls,['one','two']);
  const second=engine.fire(frame(2,'delete'),5000);await second.result;await second.settled;
  assert.deepEqual(calls,['one','two','one','added']);
});

test('cancellation discards contributions but retains actual occupancy and source queued delivery data',async()=>{
  const {engine}=fixture(), gate=deferred(), cts=new CancellationTokenSource(), calls=[];let cancellation=0;
  engine.forOwner('one').onWillRenameFiles(event=>{
    calls.push(event.files[0].newUri.fsPath);event.token.onCancellationRequested(()=>cancellation++);event.waitUntil(gate.promise);
  });
  engine.forOwner('two').onWillRenameFiles(event=>calls.push(`two:${event.files[0].newUri.fsPath}`));
  const first=engine.fire(frame(),5000,cts.token);cts.cancel();assert.equal(cancellation,1);
  assert.equal(engine.busy(),true);assert.throws(()=>engine.fire(frame(2),5000),/actually occupied/);
  gate.resolve(edit('discarded'));assert.equal(await first.result,undefined);await first.settled;assert.equal(engine.busy(),false);
  const next=frame(2);next.files[0].target='file:///tmp/new.cpp';
  const second=engine.fire(next,5000);await second.result;await second.settled;
  assert.deepEqual(calls,['/tmp/destination.cpp','two:/tmp/destination.cpp','/tmp/new.cpp','two:/tmp/new.cpp']);cts.dispose();
});

test('already cancelled delivery is queued and cannot silently consume unbounded cohorts',async()=>{
  const {engine}=fixture();let calls=0;
  const subscriptions=Array.from({length:1024},(_,index)=>engine.forOwner(String(index)).onWillCreateFiles(()=>calls++));
  const first=engine.fire(frame(1,'create'),5000,CancellationToken.Cancelled);assert.equal(await first.result,undefined);await first.settled;
  assert.equal(calls,0);assert.throws(()=>engine.fire(frame(2,'create'),5000),/queue exceeds/);
  subscriptions.forEach(value=>value.dispose());
  const second=engine.fire(frame(2,'create'),5000);await second.result;await second.settled;assert.equal(calls,1024);
});

test('callback throw does not pretend its already admitted unresolved work stopped',async()=>{
  const {engine,errors}=fixture(), gate=deferred();let saved;
  engine.forOwner('throw').onWillDeleteFiles(event=>{saved=event;event.waitUntil(gate.promise);throw new Error('throw after admission');});
  const run=engine.fire(frame(1,'delete'),5000);assert.equal(await run.result,undefined);
  assert.equal(engine.busy(),true);assert.throws(()=>engine.fire(frame(2,'delete'),5000),/actually occupied/);
  // Source's synchronous-throw arm never freezes the collection.
  saved.waitUntil(Promise.reject(new Error('late wait rejection')));gate.reject(new Error('late rejection'));await run.settled;
  assert.equal(engine.busy(),false);assert.deepEqual(errors,['throw after admission','late wait rejection','late rejection']);
});

test('retired original owners cannot assimilate or convert contributions with renewed leases',async()=>{
  const {engine,retire,renew,errors}=fixture(), gate=deferred();
  engine.forOwner('owner').onWillRenameFiles(event=>event.waitUntil(gate.promise));
  const first=engine.fire(frame(),5000);retire('owner');renew('owner');gate.resolve(edit('stale'));
  assert.equal(await first.result,undefined);await first.settled;assert.match(errors[0],/missing or retired/);
  const second=engine.fire(frame(2),5000);assert.equal(await second.result,undefined);await second.settled;assert.match(errors[1],/missing or retired/);
});

test('foreign/malformed/replayed requests are fully rejected before callbacks or sequence consumption',async()=>{
  const {engine}=fixture();let calls=0;engine.forOwner('owner').onWillRenameFiles(()=>calls++);
  for(const value of [{...frame(),session:9},{...frame(),extra:1},{...frame(),files:[frame().files[0],{target:1}]},frame(0)])assert.throws(()=>engine.fire(value,5000));
  assert.equal(calls,0);assert.throws(()=>engine.fire(frame(),-1));
  const run=engine.fire(frame(),5000);await run.result;await run.settled;assert.equal(calls,1);assert.throws(()=>engine.fire(frame(),5000));
});

test('all WorkspaceEdit kinds survive collection, with per-edit create suppression and notebook versions',async()=>{
  const {engine,versions}=fixture();versions.set(uri.toString(),11);versions.set(other.toString(),22);
  const rich=edit('before create');rich.createFile(uri,{contents:new Uint8Array([0,1,255]),overwrite:true},{label:'create',needsConfirmation:true});
  rich.set(other,[SnippetTextEdit.replace(new Range(0,0,0,0),new SnippetString('${1:猫}'))]);
  rich.renameFile(uri,other,{overwrite:true});rich.deleteFile(other,{recursive:true});
  rich.set(Uri.parse('notebook:///tmp/a'),[NotebookEdit.updateNotebookMetadata({flag:true})]);
  engine.forOwner('one').onWillCreateFiles(event=>event.waitUntil(Promise.resolve(rich)));
  engine.forOwner('two').onWillCreateFiles(event=>event.waitUntil(Promise.resolve(edit('separate edit'))));
  const run=engine.fire(frame(1,'create'),5000);const result=await run.result;await run.settled;
  assert.equal(result.edit.edits.length,7);assert.equal(result.edit.edits[0].versionId,undefined);
  assert.equal(result.edit.edits[1].options.contents.value,'AAH/');assert.equal(result.edit.edits[1].metadata.needsConfirmation,true);
  assert.equal(result.edit.edits[2].textEdit.insertAsSnippet,true);assert.equal(result.edit.edits[2].versionId,22);
  assert.equal(result.edit.edits[3].oldResource,uri.toString());assert.equal(result.edit.edits[4].newResource,undefined);
  assert.equal(result.edit.edits[5].cellEdit.editType,5);assert.equal(result.edit.edits[5].notebookVersionId,undefined);
  assert.equal(result.edit.edits[6].versionId,11);
});

test('combined raw payload and metadata wire bounds apply across individually valid contributions',async()=>{
  for(const kind of ['raw','metadata']){
    const {engine}=fixture();
    for(const owner of ['one','two'])engine.forOwner(owner).onWillCreateFiles(event=>{
      const changes=edit(kind==='raw'?'x'.repeat(600000):'x');
      if(kind==='metadata')changes.createFile(other,undefined,{label:'x'.repeat(1100000)});
      event.waitUntil(Promise.resolve(changes));
    });
    const run=engine.fire(frame(1,'create'),5000);await assert.rejects(run.result,kind==='raw'?/native budget/:/wire exceeds/);await run.settled;assert.equal(engine.busy(),false);
  }
});

test('conversion executes getters under original owner and rejects retirement during conversion',async()=>{
  const {engine,callers,retire}=fixture();
  const changes=new WorkspaceEdit();changes.createFile(other,undefined,{get label(){assert.equal(callers.currentOwner(),'one');retire('one');return 'unsafe';}});
  engine.forOwner('one').onWillCreateFiles(event=>event.waitUntil(Promise.resolve(changes)));
  const run=engine.fire(frame(1,'create'),5000);await assert.rejects(run.result,/missing or retired/);await run.settled;assert.equal(engine.busy(),false);
});

test('production runtime has a private tracked engine without advertising public will APIs',async()=>{
  const runtime=createApi(async()=>null,()=>{},{session:7});let calls=0;
  const api=runtime.fileParticipants.forOwner('fixture.owner');api.onWillCreateFiles(()=>calls++);
  assert.throws(()=>runtime.forExtension('fixture.owner').workspace.onWillCreateFiles,/not implemented/);
  const first=runtime.fileParticipants.fire(frame(1,'create'),5000);await first.result;await first.settled;assert.equal(calls,1);
  runtime.disposeOwner('fixture.owner');
  const second=runtime.fileParticipants.fire(frame(2,'create'),5000);await second.result;await second.settled;assert.equal(calls,1);
});

test('timeout value diagnoses slowness without pretending to cancel a still running extension',async()=>{
  const {engine,errors}=fixture(), gate=deferred(), savedNow=Date.now;let event,clock=100;
  Date.now=()=>clock;
  try{
    engine.forOwner('slow').onWillCreateFiles(value=>{event=value;value.waitUntil(gate.promise);});
    const run=engine.fire(frame(1,'create'),5);clock=200;await tick();
    assert.equal(engine.busy(),true);assert.equal(event.token.isCancellationRequested,false);
    gate.resolve(edit('slow edit'));assert.equal((await run.result).edit.edits[0].textEdit.text,'slow edit');await run.settled;
    assert.deepEqual(errors,['SLOW file-participant slow']);
  }finally{Date.now=savedNow;}
});

test('listener, pending delivery bytes and waitUntil counts are bounded before accepting extra work',async()=>{
  const {engine,errors}=fixture();let events=0;
  const subscriptions=Array.from({length:1024},(_,index)=>engine.forOwner(String(index)).onWillCreateFiles(()=>events++));
  assert.throws(()=>engine.forOwner('excess').onWillCreateFiles(()=>{}),/listener limit/);
  const large=frame(1,'create');large.files[0].target='file:///tmp/'+ 'x'.repeat(2000);
  assert.throws(()=>engine.fire(large,5000),/queue exceeds/);assert.equal(events,0);
  subscriptions.forEach(value=>value.dispose());
  engine.forOwner('waits').onWillCreateFiles(event=>{
    for(let index=0;index<4096;index++)event.waitUntil(Promise.resolve());
    assert.throws(()=>event.waitUntil(Promise.resolve()),/wait limit/);
  });
  const run=engine.fire(frame(1,'create'),5000);assert.equal(await run.result,undefined);await run.settled;assert.deepEqual(errors,[]);
});

test('aggregate edit count and resource count refuse the complete DTO without truncation',async()=>{
  for(const kind of ['edits','resources']){
    const {engine}=fixture();
    for(const [index,owner]of ['one','two'].entries())engine.forOwner(owner).onWillCreateFiles(event=>{
      const value=new WorkspaceEdit();
      if(kind==='edits')for(let row=0;row<2050;row++)value.replace(uri,new Range(0,0,0,0),'');
      else for(let row=0;row<(index?65:64);row++)value.createFile(Uri.parse(`file:///tmp/${owner}-${row}`));
      event.waitUntil(Promise.resolve(value));
    });
    const run=engine.fire(frame(1,'create'),5000);await assert.rejects(run.result,/native budget/);await run.settled;assert.equal(engine.busy(),false);
  }
});
