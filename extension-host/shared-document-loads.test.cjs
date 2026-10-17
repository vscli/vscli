'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {Uri}=require('./api-types.cjs');
function fixture() {
  const calls=[];let generation=0;
  const runtime=createApi((method,params,lifecycle)=>new Promise((resolve,reject)=>calls.push({method,params,lifecycle,resolve,reject})),()=>{}, {session:7,sharedDocumentLoads:true});
  runtime.configure('/tmp/shared-workspace',[],[{},{}]);
  const sync=(documents=[])=>runtime.sync({generation:++generation,documents,active:null,selections:[]});sync();
  function publish(path,id=7) {sync([{id,uri:Uri.file(path).toString(),version:1,text:'猫🙂\r\noriginal',languageId:'plaintext',isDirty:false}]);}
  return {runtime,calls,sync,publish,a:runtime.forExtension('A'),b:runtime.forExtension('B')};
}
test('relative/file-URI concurrent misses share one native customer read and return one public document',async()=>{
  const f=fixture();
  const a=f.a.workspace.openTextDocument('new.txt'),b=f.b.workspace.openTextDocument(Uri.file('/tmp/shared-workspace/new.txt'));
  assert.equal(f.calls.length,1);assert.equal(f.calls[0].method,'nativeDocumentLoad');
  f.publish('/tmp/shared-workspace/new.txt');f.calls[0].resolve({document:7});
  assert.equal(await a,await b);assert.equal(f.a.workspace.textDocuments.length,1);
  assert.equal(f.a.window.activeTextEditor,undefined);
});
test('retiring and replacing the initiating caller does not poison the other original waiter',async()=>{
  const f=fixture();
  const a=f.a.workspace.openTextDocument('new.txt'),rejected=assert.rejects(a,/retired/);
  const b=f.b.workspace.openTextDocument('new.txt');
  f.runtime.disposeOwner('A');f.runtime.forExtension('A');
  f.publish('/tmp/shared-workspace/new.txt');f.calls[0].resolve({document:7});
  await rejected;assert.equal(await b,f.b.workspace.textDocuments[0]);assert.equal(f.calls.length,1);
});
test('retiring a joining caller rejects only its original promise',async()=>{
  const f=fixture(),a=f.a.workspace.openTextDocument('new.txt'),b=f.b.workspace.openTextDocument('new.txt'),rejected=assert.rejects(b,/retired/);
  f.runtime.disposeOwner('B');f.runtime.forExtension('B');
  f.publish('/tmp/shared-workspace/new.txt');f.calls[0].resolve({document:7});
  assert.equal(await a,f.a.workspace.textDocuments[0]);await rejected;assert.equal(f.calls.length,1);
});
test('failed load broadcasts to live waiters and actual settlement admits a fresh retry',async()=>{
  const f=fixture(),a=f.a.workspace.openTextDocument('new.txt'),b=f.b.workspace.openTextDocument('new.txt');
  const settled=Promise.allSettled([a,b]);f.calls[0].reject(new Error('read failed'));
  assert((await settled).every(result=>result.status==='rejected'&&/read failed/.test(result.reason.message)));
  const retry=f.b.workspace.openTextDocument('new.txt');assert.equal(f.calls.length,2);
  f.publish('/tmp/shared-workspace/new.txt');f.calls[1].resolve({document:7});assert.equal(await retry,f.b.workspace.textDocuments[0]);
});
test('eight distinct real loads and 128 live waiters bound admission, while joining uses no new slot',async()=>{
  const f=fixture();
  const started=Array.from({length:8},(_,i)=>f.a.workspace.openTextDocument(`new-${i}.txt`));
  const joined=Array.from({length:120},()=>f.b.workspace.openTextDocument('new-0.txt'));
  const settled=Promise.allSettled([...started,...joined]);
  assert.equal(f.calls.length,8);
  await assert.rejects(f.b.workspace.openTextDocument('new-0.txt'),/waiter limit/);
  for(const call of f.calls)call.reject(new Error('fixture canceled'));
  assert.equal((await settled).length,128);
});
test('timed-out customer reads retain their original shared entry until a real terminal receipt',async()=>{
  const f=fixture(),a=f.a.workspace.openTextDocument('new.txt');
  f.calls[0].lifecycle.retained=true;f.calls[0].reject(new Error('timeout'));
  await assert.rejects(a,/timeout/);await assert.rejects(f.b.workspace.openTextDocument('new.txt'),/timeout/);
  assert.equal(f.calls.length,1);
  f.calls[0].lifecycle.onTerminal();
  const retry=f.b.workspace.openTextDocument('new.txt');assert.equal(f.calls.length,2);
  f.publish('/tmp/shared-workspace/new.txt');f.calls[1].resolve({document:7});assert.equal(await retry,f.b.workspace.textDocuments[0]);
});
test('a shared load refused before native admission does not leave a permanently rejected URI entry',async()=>{
  const f=fixture();
  const settled=Promise.allSettled(Array.from({length:8},()=>f.a.commands.executeCommand('undo')));
  await assert.rejects(f.a.workspace.openTextDocument('new.txt'),/queue limit/);
  assert.equal(f.calls.length,8);
  for(const call of f.calls)call.reject(new Error('fixture canceled'));
  await settled;
  const retry=f.b.workspace.openTextDocument('new.txt');assert.equal(f.calls.length,9);
  f.publish('/tmp/shared-workspace/new.txt');f.calls[8].resolve({document:7});assert.equal(await retry,f.b.workspace.textDocuments[0]);
});
