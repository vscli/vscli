'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { RelativePattern, createFileSearch, isFindFilesResult } = require('./workspace-files.cjs');
const { Uri } = require('./api-types.cjs');
const { CancellationTokenSource, CancellationToken } = require('./cancellation.cjs');
const { createCallerContext } = require('./caller-context.cjs');
const { createDocumentServices } = require('./document-services.cjs');
const { createNativeRequests } = require('./native-requests.cjs');
const { createApi } = require('./api.cjs');
function deferred() { let resolve, reject; const promise = new Promise((yes,no) => { resolve=yes; reject=no; }); return {promise,resolve,reject}; }
const file = Uri.file('/tmp/native-find/猫.rs');
function fixture(custom) {
  let lease = {}, workspace = {root:'/tmp/native-find',identity:{}};
  const calls = [], cancellations = [];
  const model = {captureCaller(){const captured=lease; return () => {if(captured!==lease)throw new Error('retired caller');};},workspace:()=>workspace};
  const options = {cancel(owner,id){cancellations.push({owner,id});}};
  const call = (owner,method,args,controls) => {calls.push({owner,method,args,controls});controls.onRequest(`host-${calls.length}`);return custom ? custom(...[owner,method,args,controls]) : Promise.resolve({uris:[file.toString()],cancelled:false});};
  const api = createFileSearch(call,options,model).forOwner('fixture.owner');
  return {api,calls,cancellations,model,options,call,retire(){lease={};},configure(root='/tmp/native-find'){workspace={root,identity:{}};}};
}
test('RelativePattern supports URI, path and folder values and keeps the URI reference',()=>{
  const uri=Uri.file('/tmp/outside-猫'), folder={uri,name:'Outside',index:0};
  for(const base of [uri,folder,uri.fsPath]) {
    const pattern=new RelativePattern(base,'**/*.rs');
    assert.equal(pattern.pattern,'**/*.rs');assert.equal(pattern.base,uri.fsPath);
    assert.deepEqual(pattern.toJSON(),{pattern:'**/*.rs',base:uri.fsPath,baseUri:uri.toJSON()});
    if(typeof base!=='string')assert.equal(pattern.baseUri,uri);
  }
  assert.throws(()=>new RelativePattern({},'x'),/base/);assert.throws(()=>new RelativePattern(uri,7),/pattern/);
});
test('RelativePattern setters match the pinned public value and ES5 constructor call',()=>{
  function Derived(base,glob){RelativePattern.call(this,base,glob);}
  Derived.prototype=Object.create(RelativePattern.prototype);Derived.prototype.constructor=Derived;
  const value=new Derived('/relative','a');assert.equal(value instanceof RelativePattern,true);
  value.base='/next';assert.equal(value.baseUri.toString(),Uri.file('/next').toString());
  const next=Uri.parse('custom:/virtual');value.baseUri=next;assert.equal(value.baseUri,next);assert.equal(value.base,next.fsPath);
});
test('string includes preserve exclude undefined/null/explicit distinctions and bounded maximum',async()=>{
  const f=fixture();
  for(const excluded of [undefined,null,'**/skip/**'])await f.api.findFiles('**/*.rs',excluded,0);
  assert.deepEqual(f.calls.map(value=>value.args),[
    {include:{pattern:'**/*.rs',baseUri:null},exclude:{mode:'default',pattern:null},maxResults:0},
    {include:{pattern:'**/*.rs',baseUri:null},exclude:{mode:'none',pattern:null},maxResults:0},
    {include:{pattern:'**/*.rs',baseUri:null},exclude:{mode:'pattern',pattern:'**/skip/**'},maxResults:0},
  ]);
  assert.equal(f.calls.every(value=>value.owner==='fixture.owner'&&value.method==='nativeFindFiles'),true);
});
test('relative includes preserve ordinary outside-workspace bases; exclude bases remain ignored',async()=>{
  const f=fixture(),base=Uri.file('/tmp/outside'),excluded=new RelativePattern(Uri.parse('custom:/ignored'),'**/skip/**');
  const result=await f.api.findFiles(new RelativePattern(base,'**/*.{rs,cpp}'),excluded,4096);
  assert.equal(result[0] instanceof Uri,true);assert.equal(result[0].toString(),file.toString());
  assert.deepEqual(f.calls[0].args,{include:{pattern:'**/*.{rs,cpp}',baseUri:base.toString()},exclude:{mode:'pattern',pattern:'**/skip/**'},maxResults:4096});
});
test('no workspace returns no query while an independent file base still queries',async()=>{
  const f=fixture(),identity={};f.model.workspace=()=>({root:undefined,identity});
  assert.deepEqual(await f.api.findFiles('**/*'),[]);assert.equal(f.calls.length,0);
  await f.api.findFiles(new RelativePattern('/tmp/outside','**/*'));assert.equal(f.calls.length,1);
});
test('empty include/exclude strings normalize all-files/default excludes without host traversal',async()=>{
  const f=fixture();await f.api.findFiles('','');
  assert.deepEqual(f.calls[0].args,{include:{pattern:null,baseUri:null},exclude:{mode:'default',pattern:null},maxResults:null});
});
test('precancellation reads exclude first, skips include and sends no native request',async()=>{
  const f=fixture(),order=[];
  const excluded={get pattern(){order.push('exclude');return '**/x';}};
  const token={get isCancellationRequested(){order.push('token');return true;},get onCancellationRequested(){throw new Error('must not subscribe');}};
  const included={get baseUri(){throw new Error('must not convert include');}};
  assert.deepEqual(await f.api.findFiles(included,excluded,undefined,token),[]);assert.deepEqual(order,['exclude','token']);assert.equal(f.calls.length,0);
  await assert.rejects(f.api.findFiles(included,{get pattern(){throw new Error('exclude failure');}},undefined,token),/exclude failure/);
});
test('each base/glob field is captured once and overridden URI formatters cannot retarget',async()=>{
  const f=fixture(),base=Uri.file('/tmp/exact'),counts={base:0,pattern:0};let formatted=0;
  Object.defineProperty(base,'toString',{value(){formatted++;return 'file:///retarget';}});
  Object.defineProperty(base,'toJSON',{value(){formatted++;throw new Error('must not flatten user URI');}});
  await f.api.findFiles({get baseUri(){counts.base++;return base;},get pattern(){counts.pattern++;return '*.rs';}});
  assert.deepEqual(counts,{base:1,pattern:1});assert.equal(formatted,0);assert.equal(f.calls[0].args.include.baseUri,'file:///tmp/exact');
});
for(const mutation of ['caller','workspace','subscription','last-token-read'])test(`reentrant ${mutation} mutation cannot admit stale search`,async()=>{
  const f=fixture();let stateReads=0;
  const token={get isCancellationRequested(){stateReads++;if(mutation==='last-token-read'&&stateReads===2){f.retire();return true;}return false;},onCancellationRequested(){if(mutation==='subscription')f.configure();return {dispose(){}};}};
  const included={baseUri:Uri.file('/tmp/exact'),get pattern(){if(mutation==='caller')f.retire();if(mutation==='workspace')f.configure();return '**/*';}};
  await assert.rejects(f.api.findFiles(included,undefined,undefined,token),/retired|changed/);assert.equal(f.calls.length,0);
});
test('all bounded input refusals send no partial native query',async()=>{
  const f=fixture();
  for(const maximum of [-1,1.5,4097,Infinity,null,'3'])await assert.rejects(f.api.findFiles('**/*',null,maximum),/maxResults/);
  for(const glob of ['a'.repeat(4097),'猫'.repeat(1400),'x\0y'])await assert.rejects(f.api.findFiles(glob),/bounded glob/);
  await assert.rejects(f.api.findFiles(new RelativePattern(Uri.parse('custom:/x'),'**/*')),/ordinary file/);
  await assert.rejects(f.api.findFiles(new RelativePattern(Uri.parse('file:///tmp/x?q=1'),'**/*')),/ordinary file/);
  await assert.rejects(f.api.findFiles(new RelativePattern(Uri.file('/'+'猫'.repeat(1000)),'**/*')),/encoded base/);
  assert.equal(f.calls.length,0);
});
test('active cancellation resolves [] once while original work stays retained independently',async()=>{
  const held=deferred(),f=fixture(()=>held.promise),source=new CancellationTokenSource();
  const waiting=f.api.findFiles('**/*',null,undefined,source.token);assert.equal(f.calls.length,1);
  source.cancel();source.cancel();assert.deepEqual(await waiting,[]);
  assert.deepEqual(f.cancellations,[{owner:'fixture.owner',id:'host-1'}]);
  held.resolve({uris:[file.toString()],cancelled:false});await held.promise;source.dispose();
});
test('a synchronous cancellation during subscription produces no request',async()=>{
  const f=fixture();let disposed=0;
  const token={isCancellationRequested:false,onCancellationRequested(listener){listener();return {dispose(){disposed++;}};}};
  assert.deepEqual(await f.api.findFiles('**/*',null,undefined,token),[]);assert.equal(f.calls.length,0);assert.equal(disposed,1);
});
test('onRequest follows admission; cancellation during send emits the exact later identity',async()=>{
  const source=new CancellationTokenSource(),held=deferred(),f=fixture();const order=[];
  const api=createFileSearch((owner,method,args,controls)=>{order.push('send');source.cancel();controls.onRequest('host-42');return held.promise;},
    {cancel(owner,id){order.push(`cancel:${owner}:${id}`);}},f.model).forOwner('A');
  assert.deepEqual(await api.findFiles('**/*',null,undefined,source.token),[]);
  assert.deepEqual(order,['send','cancel:A:host-42']);held.resolve({uris:[],cancelled:true});await held.promise;source.dispose();
});
test('retiring caller while waiting rejects the old receipt instead of reacquiring a lease',async()=>{
  const held=deferred(),f=fixture(()=>held.promise);const waiting=f.api.findFiles('**/*');f.retire();held.resolve({uris:[file.toString()],cancelled:false});
  await assert.rejects(waiting,/retired/);assert.equal(f.calls.length,1);
});
test('native cancellation and genuine empty results return []; malformed resources reject',async()=>{
  assert.deepEqual(await fixture(()=>({uris:[],cancelled:true})).api.findFiles('*'),[]);
  assert.deepEqual(await fixture(()=>({uris:[],cancelled:false})).api.findFiles('*'),[]);
  for(const result of [{uris:[],cancelled:0},{uris:[],cancelled:false,extra:1},{uris:['custom:/x'],cancelled:false},{uris:['file:///x?q=1'],cancelled:false}])
    await assert.rejects(fixture(()=>result).api.findFiles('*'),/acknowledgement|resource/);
});
test('typed terminal validator bounds the complete DTO and cancellation shape',()=>{
  assert.equal(isFindFilesResult({uris:[file.toString()],cancelled:false}),true);
  assert.equal(isFindFilesResult({uris:['x'],cancelled:true}),false);
  assert.equal(isFindFilesResult({uris:new Array(4097).fill('x'),cancelled:false}),false);
  assert.equal(isFindFilesResult({uris:['x'.repeat(4097)],cancelled:false}),false);
  assert.equal(isFindFilesResult({uris:new Array(4096).fill('a'.repeat(1024)),cancelled:false}),false);
});
test('foreign token cleanup keeps original caller receiver/scope, then old callbacks become inert',async()=>{
  const callers=createCallerContext(()=>{}),f=fixture();f.model.captureCaller=owner=>callers.bind(owner,()=>{});let listener,disposeOwner,receiver;
  const registration={dispose(){disposeOwner=callers.currentOwner();receiver=this;}};
  const token={isCancellationRequested:false,onCancellationRequested(callback){listener=callback;return registration;}};
  const api=createFileSearch(f.call,{...f.options,unowned:callers.unowned},f.model).forOwner('A');
  await callers.run('A',()=>api.findFiles('*',null,undefined,token));assert.equal(disposeOwner,'A');assert.equal(receiver,registration);
  listener();assert.equal(f.cancellations.length,0);
});
function clock(){let id=0;const entries=new Map();return {entries,setTimeout(fn,ms){entries.set(++id,{fn,ms});return id;},clearTimeout(id){entries.delete(id);},fire(id){const value=entries.get(id);entries.delete(id);value.fn();}};}
function bridge(sendHook){const frames=[],timers=clock();const requests=createNativeRequests(message=>{frames.push(JSON.parse(JSON.stringify(message)));sendHook?.(message);},()=>({active:true,id:17,owner:'command.owner'}),timers);
  const services=createDocumentServices(requests.request,{session:4},{generation:()=>9,root:()=>'/tmp'});return {frames,timers,requests,services};}
test('eight shared service leases stay occupied after findFiles timeout until exact typed terminal',async()=>{
  const f=bridge(),waiting=[];
  for(let i=0;i<8;i++)waiting.push(f.services.call('find.owner','nativeFindFiles',{include:{pattern:'**/*',baseUri:null},exclude:{mode:'none',pattern:null},maxResults:null}).catch(error=>error.message));
  assert.equal(f.frames.length,8);assert.equal(f.frames[0].params.commandOwner,'command.owner');assert.equal(f.frames[0].params.owner,'find.owner');assert.equal(f.timers.entries.get(1).ms,5000);
  f.timers.fire(1);assert.match(await waiting[0],/outcome is unknown/);
  await assert.rejects(f.services.call('find.owner','nativeDocumentOpen',{path:'/tmp/a'}),/queue limit/);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:[]}),false);assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:false,extra:true}}),false);
  await assert.rejects(f.services.call('find.owner','nativeDocumentSave',{document:1,version:1}),/queue limit/);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:true}}),true);
  const replacement=f.services.call('find.owner','nativeDocumentOpen',{path:'/tmp/a'}).catch(error=>error.message);assert.equal(f.frames.length,9);
  f.requests.shutdown();await Promise.all([...waiting,replacement]);
});
test('64 framing slots remain held after timeout and exact result releases only one',async()=>{
  const f=bridge(),waiting=[];
  for(let i=0;i<64;i++)waiting.push(f.requests.request('nativeFindFiles',{owner:'A',args:{}}).catch(error=>error.message));
  for(const id of [...f.timers.entries.keys()])f.timers.fire(id);await Promise.all(waiting);
  await assert.rejects(f.requests.request('nativeFindFiles',{}),/request limit/);
  assert.equal(f.requests.settle({id:'wrong',result:{uris:[],cancelled:true}}),false);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:false},error:{message:'ambiguous'}}),false);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:true}}),true);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:true}}),false);
  const one=f.requests.request('nativeFindFiles',{}).catch(error=>error.message);assert.equal(f.frames.length,65);f.requests.shutdown();await one;
});
test('onRequest after successful send may reject but cannot release the actual occupied lane',async()=>{
  const f=bridge(),waiting=[],order=[];
  await assert.rejects(f.services.call('A','nativeFindFiles',{}, {onRequest(id){assert.equal(f.frames[0].id,id);order.push('post-send');throw new Error('hook pressure');}}),/hook pressure/);
  assert.deepEqual(order,['post-send']);
  for(let i=0;i<7;i++)waiting.push(f.services.call('A','nativeFindFiles',{}).catch(error=>error.message));
  await assert.rejects(f.services.call('A','nativeDocumentOpen',{path:'/tmp/a'}),/queue limit/);
  assert.equal(f.requests.settle({id:f.frames[0].id,result:{uris:[],cancelled:true}}),true);
  const replacement=f.services.call('A','nativeFindFiles',{}).catch(error=>error.message);assert.equal(f.frames.length,9);f.requests.shutdown();await Promise.all([...waiting,replacement]);
});
test('public capability remains gated and constructor is shared while native frame carries selected owner',async()=>{
  const disabled=createApi(()=>{},()=>{});assert.equal(disabled.api.RelativePattern,RelativePattern);assert.throws(()=>disabled.forExtension('A').workspace.findFiles,/not implemented/);
  const frames=[],api=createApi((method,params,lifecycle)=>{frames.push({method,params});lifecycle?.onRequest?.('host-7');return Promise.resolve({uris:[file.toString()],cancelled:false});},()=>{},{session:7,findFiles:true});
  api.configure('/tmp/native-find',[],[{},{}]);const owner=api.forExtension('selected.owner');assert.equal(owner.RelativePattern,RelativePattern);
  const result=await owner.workspace.findFiles('**/*.rs',null);assert.equal(result[0].toString(),file.toString());
  assert.equal(frames[0].method,'nativeFindFiles');assert.equal(frames[0].params.owner,'selected.owner');assert.equal(frames[0].params.session,7);
});
test('cached old owner descendants cannot reacquire findFiles after caller lease ABA',async()=>{
  const callers=createCallerContext(()=>{}),f=fixture();f.model.captureCaller=owner=>callers.bind(owner,()=>{});
  const api=createFileSearch(f.call,{...f.options,unowned:callers.unowned},f.model).forOwner('A');let old;
  callers.run('A',()=>{old=callers.bindAsync('A',()=>api.findFiles('**/*'));});callers.retire('A');callers.run('A',()=>{});
  assert.throws(()=>old(),/retired/);assert.equal(f.calls.length,0);
});
test('actual cancellation observer remains live after owner retirement while public continuation rejects',async()=>{
  const held=deferred(),callers=createCallerContext(()=>{}),f=fixture(()=>held.promise);f.model.captureCaller=owner=>callers.bind(owner,()=>{});
  const source=new CancellationTokenSource(),api=createFileSearch(f.call,{...f.options,unowned:callers.unowned},f.model).forOwner('A');
  const waiting=callers.run('A',()=>api.findFiles('**/*',null,undefined,source.token));callers.retire('A');callers.run('A',()=>{});
  source.cancel();await assert.rejects(waiting,/retired/);assert.deepEqual(f.cancellations,[{owner:'A',id:'host-1'}]);
  held.resolve({uris:[],cancelled:true});await held.promise;source.dispose();
});
