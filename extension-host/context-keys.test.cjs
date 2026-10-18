'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
function fixture(handler,enabled=true){
 const calls=[];
 const runtime=createApi(async(method,params)=>{calls.push({method,params});return handler?handler(method,params):null;},()=>{},{session:8,contextKeys:enabled});
 runtime.sync({generation:1,documents:[],active:null,selections:[]});
 return {runtime,api:runtime.forExtension('fixture.context'),calls};
}
test('setContext works at welcome, keeps false/zero/null distinct and removes undefined',async()=>{
 const f=fixture();assert((await f.api.commands.getCommands()).includes('setContext'));
 assert.throws(()=>f.api.commands.registerCommand('setContext',()=>{}),/reserved/);
 for(const value of [false,0,null,['a'],{a:false},undefined]){
  assert.equal(await f.api.commands.executeCommand('setContext','fixture.key',value),undefined);
  assert.deepEqual(f.calls.at(-1),{method:'nativeContextSet',params:{session:8,owner:'fixture.context',generation:1,args:{key:'fixture.key',remove:value===undefined,value:value===undefined?null:value}}});
 }
 await f.api.commands.executeCommand('setContext','');assert.equal(f.calls.at(-1).params.args.remove,true);
 await f.api.commands.executeCommand('setContext','key',true,'ignored');assert.equal(f.calls.at(-1).params.args.value,true);
 const {Uri}=require('./api-types.cjs');const uri=Uri.file('/tmp/context λ');await f.api.commands.executeCommand('setContext','uris',{items:[uri]});assert.deepEqual(f.calls.at(-1).params.args.value,{items:[uri.toString()]});
 assert.equal(f.api.workspace.textDocuments.length,0);assert.equal(f.api.window.activeTextEditor,undefined);
});
test('setContext validates arguments before IPC and snapshots mutable values before awaiting',async()=>{
 let resolve;const f=fixture(()=>new Promise(done=>{resolve=done;}));
 for(const args of [[],[false,true],['k'.repeat(1025),true],['key',()=>{}],['key','x'.repeat(65536)]]){
  await assert.rejects(f.api.commands.executeCommand('setContext',...args));
 }
 const cycle={};cycle.self=cycle;await assert.rejects(f.api.commands.executeCommand('setContext','cycle',cycle));assert.equal(f.calls.length,0);
 const original={items:['before']}, pending=f.api.commands.executeCommand('setContext','fixture.key',original);original.items.push('after');
 assert.deepEqual(f.calls[0].params.args.value,{items:['before']});resolve(null);await pending;
});
test('context requests share the existing eight occupied document-service slots',async()=>{
 const releases=[],f=fixture(()=>new Promise(done=>releases.push(done)));
 const pending=Array.from({length:8},(_,i)=>f.api.commands.executeCommand('setContext','key'+i,i));
 await assert.rejects(f.api.commands.executeCommand('setContext','overflow',true),/queue limit/);
 assert.equal(f.calls.length,8);releases.forEach(done=>done(null));await Promise.all(pending);
 const next=f.api.commands.executeCommand('setContext','later',true);releases.at(-1)(null);await next;
});
test('unavailable context backends and malformed acknowledgements reject explicitly',async()=>{
 const unavailable=fixture(undefined,false);assert(!(await unavailable.api.commands.getCommands()).includes('setContext'));
 await assert.rejects(unavailable.api.commands.executeCommand('setContext','key',true),/unavailable/);assert.equal(unavailable.calls.length,0);
 const malformed=fixture(()=>false);await assert.rejects(malformed.api.commands.executeCommand('setContext','key',true),/acknowledgement/);
});
test('context wire acknowledgement preserves occupied lifecycle through malformed responses',async()=>{
 const {createNativeRequests}=require('./native-requests.cjs');const frames=[];let released=0;
 const requests=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:4,owner:'fixture.context'}));
 const lifecycle={onTerminal:()=>released++};const pending=requests.request('nativeContextSet',{session:8,owner:'fixture.context',args:{key:'key',remove:false,value:true}},lifecycle);
 assert.equal(lifecycle.retained,true);assert.equal(frames[0].params.command,4);
 const id=frames[0].id;
 for(const message of [{id,result:false},{id},{id,result:null,error:{message:'both'}},{id,error:{message:3}}]){assert.equal(requests.settle(message),false);assert.equal(released,0);}
 assert.equal(requests.settle({id,result:null}),true);assert.equal(await pending,null);assert.equal(released,1);
});
test('context timeout reports unknown outcome and retains the slot until its exact terminal reply',async()=>{
 const {createNativeRequests}=require('./native-requests.cjs');const frames=[];let timeout,released=0;
 const requests=createNativeRequests(frame=>frames.push(frame),()=>undefined,{setTimeout:callback=>{timeout=callback;return 1;},clearTimeout:()=>{}});
 const pending=requests.request('nativeContextSet',{session:8,owner:'fixture.context',args:{key:'key',remove:false,value:true}},{onTerminal:()=>released++});
 const refused=assert.rejects(pending,/outcome is unknown/);timeout();await refused;assert.equal(released,0);
 assert.equal(requests.settle({id:frames[0].id,result:null}),true);assert.equal(released,1);
});
