'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');const {Uri}=require('./api-types.cjs');
function deferred(){let resolve;const promise=new Promise(done=>resolve=done);return {promise,resolve};}
function fixture(finish){
  const calls=[];let runtime,generation=0,id=0,ticket=0;
  const records=new Map();
  runtime=createApi(async(method,params,lifecycle)=>{
    calls.push({method,params});
    if(method==='nativeDocumentLoadBegin')return {load:`load-${++ticket}`};
    if(method==='nativeDocumentLoadCancel')return {canceled:true};
    if(finish) { const value=finish(method,params,lifecycle); if(value!==undefined)return value; }
    const args=params.args,uri=args.uri||`untitled:generated-${id+1}`;
    const row={id:++id,uri,version:1,text:args.content,languageId:args.language||'plaintext',isReadonly:method==='nativeVirtualDocumentLoad',isDirty:method==='nativeUntitledDocumentLoad'};
    records.set(row.id,row);runtime.sync({generation:++generation,documents:[...records.values()],active:null,selections:[]});return {document:row.id};
  },()=>{}, {session:7,virtualDocuments:true,untitledDocuments:true,sharedGeneratedDocumentLoads:true});
  runtime.configure('/tmp/generated',[],[{},{}]);runtime.sync({generation:0,documents:[],active:null,selections:[]});
  return {runtime,calls,a:runtime.forExtension('A'),b:runtime.forExtension('B'),p:runtime.forExtension('P')};
}
test('native admission precedes provider callback and first consumer retirement leaves other waiter live',async()=>{
  const f=fixture(),held=deferred();let called=0;
  f.p.workspace.registerTextDocumentContentProvider('preview',{provideTextDocumentContent(){called++;assert.equal(f.calls[0].method,'nativeDocumentLoadBegin');return held.promise;}});
  const uri=Uri.parse('preview:/a'),a=f.a.workspace.openTextDocument(uri),failed=assert.rejects(a,/retired/),b=f.b.workspace.openTextDocument(uri);
  while(!called)await new Promise(resolve=>setImmediate(resolve));
  f.runtime.disposeOwner('A');f.runtime.forExtension('A');held.resolve('猫🙂\n');
  await failed;assert.equal(await b,f.b.workspace.textDocuments[0]);assert.equal(called,1);
  assert.deepEqual(f.calls.map(call=>call.method),['nativeDocumentLoadBegin','nativeVirtualDocumentLoad']);
});
test('provider disposal refuses publication and cancels the admitted customer ticket',async()=>{
  const f=fixture(),held=deferred();let called=0;
  const registration=f.p.workspace.registerTextDocumentContentProvider('preview',{provideTextDocumentContent(){called++;return held.promise;}});
  const pending=f.a.workspace.openTextDocument(Uri.parse('preview:/a')),failed=assert.rejects(pending,/disposed/);
  while(!called)await new Promise(resolve=>setImmediate(resolve));registration.dispose();held.resolve('late');await failed;
  assert.deepEqual(f.calls.map(call=>call.method),['nativeDocumentLoadBegin','nativeDocumentLoadCancel']);assert.equal(f.a.workspace.textDocuments.length,0);
});
test('explicit untitled URI shares one admission/creation while noarg creations remain distinct',async()=>{
  const f=fixture(),uri=Uri.parse('untitled:/same.cpp');
  const [a,b]=await Promise.all([f.a.workspace.openTextDocument(uri),f.b.workspace.openTextDocument(uri)]);assert.equal(a,b);
  assert.equal(f.calls.filter(call=>call.method==='nativeDocumentLoadBegin').length,1);
  const [first,second]=await Promise.all([f.a.workspace.openTextDocument({content:'猫🙂'}),f.b.workspace.openTextDocument({content:'猫🙂'})]);
  assert.notEqual(first,second);assert.notEqual(first.uri.toString(),second.uri.toString());
  assert.equal(f.calls.filter(call=>call.method==='nativeUntitledDocumentLoad').length,3);
});
for(const virtual of [false,true])test(`${virtual?'virtual':'untitled'} publication timeout keeps its shared entry until the actual terminal response`,async()=>{
  let timeout=true,terminal;
  const f=fixture((method,params,lifecycle)=>{if(timeout){lifecycle.retained=true;terminal=lifecycle.onTerminal;return Promise.reject(new Error('publication timeout'));}});
  const uri=Uri.parse(virtual?'preview:/same':'untitled:/same.cpp');let calls=0;
  if(virtual)f.p.workspace.registerTextDocumentContentProvider('preview',{provideTextDocumentContent(){calls++;return '猫🙂';}});
  await assert.rejects(f.a.workspace.openTextDocument(uri),/timeout/);
  await assert.rejects(f.b.workspace.openTextDocument(uri),/timeout/);
  assert.equal(f.calls.length,2);assert(!f.calls.some(call=>call.method==='nativeDocumentLoadCancel'));
  terminal();timeout=false;
  const doc=await f.b.workspace.openTextDocument(uri);assert.equal(doc.getText(),virtual?'猫🙂':'');
  assert.equal(f.calls.length,4);if(virtual)assert.equal(calls,2);
});
test('shared provider work bounds pending consumers independently of its one callback',async()=>{
  const f=fixture(),held=deferred();let calls=0;
  f.p.workspace.registerTextDocumentContentProvider('preview',{provideTextDocumentContent(){calls++;return held.promise;}});
  const uri=Uri.parse('preview:/same'),pending=Array.from({length:128},()=>f.a.workspace.openTextDocument(uri));
  const settled=Promise.allSettled(pending);await assert.rejects(f.b.workspace.openTextDocument(uri),/waiter limit/);
  while(!calls)await new Promise(resolve=>setImmediate(resolve));assert.equal(calls,1);held.resolve('猫🙂');
  const results=await settled;assert(results.every(result=>result.status==='fulfilled'&&result.value===results[0].value));
  assert.equal(f.calls.length,2);
});
