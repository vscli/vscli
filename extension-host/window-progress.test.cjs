'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
function fixture() {
 const messages=[],runtime=createApi(()=>{throw new Error('Unexpected native request');},(method,params)=>messages.push({method,params}),{session:17});
 return {runtime,messages,a:runtime.forExtension('a'),b:runtime.forExtension('b')};
}
const window={location:10,title:'Index 猫'};
const turn=()=>new Promise(resolve=>setImmediate(resolve));
test('Window progress uses native status updates, coalesces reports and returns exact task identity',async()=>{
 const f=fixture(),value={private:'result'};value.self=value;let release,report;
 assert.equal(f.a.ProgressLocation.Window,10);assert.equal(f.a.ProgressLocation[15],'Notification');
 const result=f.a.window.withProgress({...window,cancellable:true},(progress,token)=>{
  report=progress;assert.equal(token.isCancellationRequested,false);assert.equal(typeof token.onCancellationRequested,'function');
  token.onCancellationRequested(()=>assert.fail('Window has no user cancellation')).dispose();
  return new Promise(resolve=>{release=resolve;});
 });
 assert(f.messages.some(x=>x.params.visible&&x.params.text==='⟳ Index 猫'));
 const before=f.messages.length;
 for(let n=0;n<10000;n++)report.report({message:'phase '+n,increment:10});
 assert.equal(f.messages.length,before);await turn();assert.equal(f.messages.length,before+1);
 assert.equal(f.messages.at(-1).params.text,'⟳ Index 猫: phase 9999');
 assert.equal(f.messages.at(-1).params.owner,'a');release(value);assert.equal(await result,value);
 assert.equal(f.messages.at(-1).params.op,'statusDispose');
 report.report({get message(){assert.fail('Finished report must be inert');}});
});
test('sync and async task failures retain their exact error and remove progress UI',async()=>{
 const f=fixture(),failure=new Error('original');
 for(const task of [()=>{throw failure;},async()=>{throw failure;}]) {
  await assert.rejects(f.a.window.withProgress(window,task),error=>error===failure);
  assert.equal(f.messages.at(-1).params.op,'statusDispose');
 }
 assert.equal(await f.a.window.withProgress(window,async()=>42),42);
});
test('unsupported locations and invalid data refuse without task admission or leaked native presentation',async()=>{
 const f=fixture();let calls=0;
 for(const options of [null,{}, {location:1},{location:15},{location:{viewId:'view'}},{...window,title:'x'.repeat(513)},{...window,cancellable:'yes'}])
  await assert.rejects(f.a.window.withProgress(options,()=>{calls++;}));
 assert.equal(calls,0);assert.equal(f.messages.filter(x=>x.method==='nativeSurface').length,0);
 let progress,release;const task=f.a.window.withProgress(window,p=>{progress=p;return new Promise(done=>{release=done;});});
 for(const value of [null,[],{message:4},{message:'x'.repeat(513)},{increment:NaN},{increment:'1'}])assert.throws(()=>progress.report(value));
 const before=f.messages.length;progress.report({increment:50});await turn();assert.equal(f.messages.length,before);
 release('ok');assert.equal(await task,'ok');
});
test('retired UI retains eight shared actual task slots and cached handles cannot borrow a renewed owner',async()=>{
 const f=fixture(),releases=[],reports=[],old=f.a.window.withProgress;
 const requests=Array.from({length:8},(_,n)=>(n%2?f.b:f.a).window.withProgress(window,p=>{reports.push(p);return new Promise(done=>releases.push(done));}));
 const outcomes=Promise.allSettled(requests);
 f.runtime.disposeOwner('a');const fresh=f.runtime.forExtension('a');
 await assert.rejects(f.b.window.withProgress(window,async()=>0),/limit/);
 await assert.rejects(fresh.window.withProgress(window,async()=>0),/limit/);
 await assert.rejects(old(window,async()=>0),/retired/);
 const count=f.messages.length;reports[0].report({message:'late'});await turn();assert.equal(f.messages.length,count);
 releases.forEach(done=>done('settled'));const settled=await outcomes;
 assert.equal(settled.filter(x=>x.status==='rejected'&&/retired/.test(x.reason.message)).length,4);
 assert.equal(settled.filter(x=>x.status==='fulfilled').length,4);
 assert.equal(await fresh.window.withProgress(window,async()=>99),99);
});
test('option getter renewal refuses before effects and delegated tasks run under the original facade owner',async()=>{
 const f=fixture(),old=f.a.window.withProgress;let calls=0;
 await assert.rejects(old({get location(){f.runtime.disposeOwner('a');f.runtime.forExtension('a');return 10;}},()=>{calls++;}),/retired/);
 assert.equal(calls,0);assert.equal(f.messages.filter(x=>x.method==='nativeSurface').length,0);
 const fresh=f.runtime.forExtension('a');
 const result=await f.runtime.runWithOwner('b',()=>fresh.window.withProgress(window,async()=>f.runtime.currentCallerOwner()));
 assert.equal(result,'a');
});
