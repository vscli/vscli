'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs');
const {createNativeRequests}=require('./native-requests.cjs');
function fixture(handler,enabled=true) {
 const calls=[];
 const runtime=createApi((method,params)=>{calls.push({method,params});return handler ? handler(method,params) : Promise.resolve(null);},()=>{},{session:28,configurationWrites:enabled});
 runtime.configure(process.cwd(),{properties:{'fixture.value':{default:'default',scope:'language-overridable'}}},[{'fixture.value':'before'},{}]);
 runtime.sync({generation:1,documents:[],active:null});
 return {runtime,api:runtime.forExtension('fixture.configuration'),calls};
}
test('configuration updates retain snapshots, publish fresh inspection/events before void and preserve targets',async()=>{
 let resolve;const f=fixture(()=>new Promise(done=>{resolve=done;}));
 const config=f.api.workspace.getConfiguration('fixture');const events=[];
 f.api.workspace.onDidChangeConfiguration(event=>events.push(event.affectsConfiguration('fixture.value')));
 const value={enabled:false,list:['猫🙂',0,null]};const done=config.update('value',value,true);
 value.list.push('later');assert.equal(config.get('value'),'before');assert.deepEqual(events,[]);
 assert.deepEqual(f.calls[0],{method:'nativeConfigurationUpdate',params:{session:28,owner:'fixture.configuration',generation:1,args:{key:'fixture.value',value:{enabled:false,list:['猫🙂',0,null]},remove:false,target:1,language:null,resource:null}}});
 f.runtime.updateConfiguration([{'fixture.value':{enabled:false,list:['猫🙂',0,null]}},{}]);resolve(null);
 assert.equal(await done,undefined);assert.deepEqual(events,[true]);assert.equal(config.get('value'),'before');
 assert.deepEqual(config.inspect('value').globalValue,{enabled:false,list:['猫🙂',0,null]});
 assert.deepEqual(f.api.workspace.getConfiguration('fixture').get('value'),config.inspect('value').globalValue);
 assert.equal(f.api.ConfigurationTarget.WorkspaceFolder,3);assert.equal(f.api.ConfigurationTarget[1],'Global');
});
test('configuration target overloads, remove versus null and existing language overrides match captured intent',async()=>{
 const f=fixture();
 for (const [target,destination] of [[undefined,2],[null,2],[false,2],[true,1],[1,1],[2,2],[3,3]]) {
  await f.api.workspace.getConfiguration().update('fixture.value',null,target);
  assert.equal(f.calls.at(-1).params.args.target,destination);assert.equal(f.calls.at(-1).params.args.remove,false);
 }
 await f.api.workspace.getConfiguration('fixture').update('value',undefined,1);
 assert.equal(f.calls.at(-1).params.args.remove,true);assert.equal(f.calls.at(-1).params.args.value,null);
 f.runtime.updateConfiguration([{'fixture.value':'root','[cpp]':{'fixture.value':false}},{}]);
 const config=f.api.workspace.getConfiguration('fixture',{languageId:'cpp'});
 await config.update('value',0,1);assert.equal(f.calls.at(-1).params.args.language,'cpp');
 await config.update('value',true,1,false);assert.equal(f.calls.at(-1).params.args.language,null);
 await config.update('value',true,2,true);assert.equal(f.calls.at(-1).params.args.language,'cpp');
 const uri=f.api.Uri.file(process.cwd()+'/main.cpp');
 await f.api.workspace.getConfiguration('fixture',{uri,languageId:'cpp'}).update('value',2,3,true);
 assert.equal(f.calls.at(-1).params.args.resource,uri.toString());assert.equal(f.calls.at(-1).params.args.target,3);
});
test('configuration JSON getters run once and retained original callers cannot borrow a replacement lease',async()=>{
 const f=fixture();let reads=0;const value={get key(){reads++;return ['λ',false];}};
 await f.api.workspace.getConfiguration('fixture').update('value',value,1);assert.equal(reads,1);
 assert.deepEqual(f.calls[0].params.args.value,{key:['λ',false]});
 const old=f.api.workspace.getConfiguration('fixture');f.runtime.disposeOwner('fixture.configuration');f.runtime.forExtension('fixture.configuration');
 await assert.rejects(old.update('value',true,1),/retired/);assert.equal(f.calls.length,1);
 const newer=f.runtime.forExtension('fixture.configuration').workspace.getConfiguration('fixture');
 const reentrant={get data(){f.runtime.disposeOwner('fixture.configuration');f.runtime.forExtension('fixture.configuration');return true;}};
 await assert.rejects(newer.update('value',reentrant,1),/retired/);assert.equal(f.calls.length,1);
});
test('configuration bounds and unavailable backends refuse before partial native work',async()=>{
 const f=fixture();const large=[];large.length=0xffffffff;const cycle={};cycle.self=cycle;
 let deep={};for(let index=0;index<65;index++)deep={nested:deep};
 for(const value of ['x'.repeat(65537),large,cycle,deep,1n,()=>{}]) {
  await assert.rejects(f.api.workspace.getConfiguration('fixture').update('value',value,1));
 }
 assert.equal(f.calls.length,0);
 const disabled=fixture(undefined,false);await assert.rejects(disabled.api.workspace.getConfiguration('fixture').update('value',true,1),/not implemented/);assert.equal(disabled.calls.length,0);
 const malformed=fixture(()=>Promise.resolve(false));await assert.rejects(malformed.api.workspace.getConfiguration('fixture').update('value',true,1),/acknowledgement/);
});
test('configuration writes share eight actual native service slots without optimistic reads',async()=>{
 const releases=[],f=fixture(()=>new Promise(done=>releases.push(done)));
 const pending=Array.from({length:8},(_,index)=>f.api.workspace.getConfiguration('fixture').update('value',index,1));
 await assert.rejects(f.api.workspace.getConfiguration('fixture').update('value',99,1),/queue limit/);
 assert.equal(f.calls.length,8);assert.equal(f.api.workspace.getConfiguration('fixture').get('value'),'before');
 releases.forEach(done=>done(null));await Promise.all(pending);
});
test('configuration framing retains timed-out work until exact bounded void/error receipts',async()=>{
 const frames=[];let timeout,delay,released=0;
 const requests=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:7,owner:'fixture.configuration'}),{
  setTimeout(callback,milliseconds){timeout=callback;delay=milliseconds;return 1;},clearTimeout(){},
 });
 const lifecycle={onTerminal(){released++;}};
 const pending=requests.request('nativeConfigurationUpdate',{session:28,owner:'fixture.configuration',args:{key:'fixture.value',value:true}},lifecycle);
 assert.equal(delay,360000);assert.equal(lifecycle.retained,true);assert.equal(frames[0].params.command,7);
 const refused=assert.rejects(pending,/outcome is unknown/);timeout();await refused;assert.equal(released,0);
 for(const message of [{id:frames[0].id,result:false},{id:frames[0].id},{id:frames[0].id,result:null,error:{message:'both'}},{id:frames[0].id,error:{message:'x'.repeat(4097)}}]) {
  assert.equal(requests.settle(message),false);assert.equal(released,0);
 }
 assert.equal(requests.settle({id:frames[0].id,result:null}),true);assert.equal(released,1);
 assert.equal(requests.settle({id:frames[0].id,result:null}),false);assert.equal(released,1);
});
