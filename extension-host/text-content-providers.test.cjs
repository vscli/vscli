'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {createTextContentProviders}=require('./text-content-providers.cjs');
const {Uri,EventEmitter}=require('./api-types.cjs');
const {createApi}=require('./api.cjs');
const {createCallerContext}=require('./caller-context.cjs');
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}
async function until(predicate){for(let i=0;i<100;i++){if(predicate())return;await new Promise(resolve=>setImmediate(resolve));}assert.fail('Virtual content fixture did not settle');}
const uri=Uri.parse('preview-test:/sample.cpp');
function fixture(custom){
  let lease={},next=0;const mirrors=new Map(),calls=[],notifications=[],errors=[];
  const options={session:7,notify:(method,value)=>notifications.push({method,value}),track:(_owner,item)=>item,
    captureCaller:()=>{const captured=lease;return()=>{if(captured!==lease)throw new Error('retired caller');};},
    mirrors:()=>mirrors.values(),mirror:id=>mirrors.get(id),diagnostic:(owner,error)=>errors.push({owner,error}),
    call(owner,method,args){calls.push({owner,method,args});if(custom)return custom(owner,method,args);
      let mirror=[...mirrors.values()].find(value=>value.uri===args.uri);
      if(method==='nativeVirtualDocumentOpen'){
        if(!mirror){mirror={document:++next,version:1,uri:args.uri,text:args.content,model:{id:next}};mirrors.set(next,mirror);}
      }else{mirror={...mirror,version:mirror.version+1,text:args.content};mirrors.set(mirror.document,mirror);}
      manager.documentChanged();return Promise.resolve({document:mirror.document});},
  };
  const manager=createTextContentProviders(options);return{manager,api:manager.forOwner('provider.owner'),options,mirrors,calls,notifications,errors,retire(){lease={};}};
}
test('registration emits monotonic exact handles; disposal is idempotent and custom-only',()=>{
  const f=fixture(),a=f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=>''}),b=f.api.registerTextDocumentContentProvider('second',{provideTextDocumentContent:()=>''});
  assert.deepEqual(f.notifications.map(value=>value.value),[
    {session:7,generation:1,providers:[{scheme:'preview-test',owner:'provider.owner',handle:1}]},
    {session:7,generation:2,providers:[{scheme:'preview-test',owner:'provider.owner',handle:1},{scheme:'second',owner:'provider.owner',handle:2}]},
  ]);
  a.dispose();a.dispose();b.dispose();assert.equal(f.notifications.length,4);assert.equal(f.manager.snapshot().generation,4);
  for(const scheme of ['file','untitled','FILE','a b','','1bad','x'.repeat(129)])assert.throws(()=>f.api.registerTextDocumentContentProvider(scheme,{provideTextDocumentContent:()=>''}),/scheme/);
});
test('provider content normalizes LF and native mirror precedes open acknowledgement',async()=>{
  const f=fixture(),provider={provideTextDocumentContent(resource,token){assert.equal(this,provider);assert.equal(resource.toString(),uri.toString());assert.equal(token.isCancellationRequested,false);return '猫🙂\r\nnext\rlast\n';}};
  f.api.registerTextDocumentContentProvider('preview-test',provider);const document=await f.manager.open('consumer.owner',uri);
  assert.equal(document,f.mirrors.get(1).model);assert.deepEqual(f.calls[0],{owner:'consumer.owner',method:'nativeVirtualDocumentOpen',args:{uri:uri.toString(),content:'猫🙂\nnext\nlast\n',providerOwner:'provider.owner',providerHandle:1}});
});
test('cached and concurrently opening documents preserve identity without extra provider calls',async()=>{
  const held=deferred(),f=fixture();let invoked=0;
  const registration=f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(){invoked++;return held.promise;}});
  const first=f.manager.open('A',uri),second=f.manager.open('B',uri);await until(()=>invoked===1);held.resolve('content');
  assert.equal(await first,await second);assert.equal(f.calls.length,1);
  registration.dispose();assert.equal(await f.manager.open('C',uri),f.mirrors.get(1).model);assert.equal(invoked,1);
});
test('latest registered provider wins, null/undefined fall through and empty content is valid',async()=>{
  const f=fixture(),seen=[];
  f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(){seen.push('old');return '';}});
  f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(){seen.push('new');return null;}});
  const result=await f.manager.open('consumer',uri);assert.equal(result,f.mirrors.get(1).model);assert.deepEqual(seen,['new','old']);assert.equal(f.calls[0].args.providerHandle,1);
});
test('refresh preserves identity and skips same-line CRLF content without native work',async()=>{
  const f=fixture(),event=new EventEmitter();let value='first\nsecond';
  f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=>value,onDidChange:event.event});
  const doc=await f.manager.open('A',uri);value='first\r\nsecond';event.fire(uri);await until(()=>f.calls.length===1&&f.manager.snapshot().providers.length===1);await new Promise(resolve=>setImmediate(resolve));
  assert.equal(f.calls.length,1);assert.equal(f.mirrors.get(1).version,1);
  value='猫\r\nchanged';event.fire(uri);await until(()=>f.calls.length===2);await until(()=>f.mirrors.get(1).version===2);
  assert.equal(f.mirrors.get(1).model,doc);assert.equal(f.mirrors.get(1).text,'猫\nchanged');assert.deepEqual(f.calls[1].args,{uri:uri.toString(),content:'猫\nchanged',document:1,version:1,providerOwner:'provider.owner',providerHandle:1});
});
test('held obsolete refresh is canceled and one latest coalesced callback begins only after settlement',async()=>{
  const f=fixture(),event=new EventEmitter(),held=deferred();let invoked=0,value='initial',active=0,max=0,heldToken;
  f.api.registerTextDocumentContentProvider('preview-test',{onDidChange:event.event,provideTextDocumentContent(_uri,token){invoked++;active++;max=Math.max(max,active);if(invoked===2){heldToken=token;return held.promise.finally(()=>active--);}active--;return value;}});
  const doc=await f.manager.open('A',uri);value='old';event.fire(uri);await until(()=>invoked===2);
  value='latest';for(let i=0;i<50;i++)event.fire(uri);
  assert.equal(heldToken.isCancellationRequested,true);assert.equal(invoked,2);held.resolve('obsolete');
  await until(()=>invoked===3&&f.calls.length===2);assert.equal(max,1);assert.equal(f.mirrors.get(1).text,'latest');assert.equal(f.mirrors.get(1).model,doc);
  assert.equal(f.calls.some(call=>call.args.content==='obsolete'),false);
});
test('wrong schemes and unopened events do not invoke the provider',async()=>{
  const f=fixture(),event=new EventEmitter();let invoked=0;
  f.api.registerTextDocumentContentProvider('preview-test',{onDidChange:event.event,provideTextDocumentContent(){invoked++;return 'x';}});
  event.fire(Uri.parse('other:/sample.cpp'));event.fire(uri);await new Promise(resolve=>setImmediate(resolve));assert.equal(invoked,0);assert.equal(f.calls.length,0);
});
test('closed documents and disposed registrations cancel actual callbacks and cannot republish',async()=>{
  for(const retirement of ['close','dispose']){
    const f=fixture(),event=new EventEmitter(),held=deferred();let invoked=0,token;
    const registration=f.api.registerTextDocumentContentProvider('preview-test',{onDidChange:event.event,provideTextDocumentContent(_uri,t){invoked++;if(invoked===2){token=t;return held.promise;}return 'initial';}});
    await f.manager.open('A',uri);event.fire(uri);await until(()=>invoked===2);
    if(retirement==='close'){f.mirrors.clear();f.manager.documentChanged();}else registration.dispose();
    assert.equal(token.isCancellationRequested,true);held.resolve('late');await new Promise(resolve=>setImmediate(resolve));assert.equal(f.calls.length,1);
  }
});
test('disposed/replaced held opens never create a document under a new handle',async()=>{
  const f=fixture(),held=deferred();let token;
  const old=f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(_uri,t){token=t;return held.promise;}});
  const waiting=f.manager.open('A',uri);await until(()=>token);old.dispose();
  f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=> 'new'});held.resolve('old');
  await assert.rejects(waiting,/disposed/);assert.equal(f.calls.length,0);
  await f.manager.open('A',uri);assert.equal(f.calls[0].args.providerHandle,2);assert.equal(f.mirrors.get(1).text,'new');
});
test('eight actual ignored-cancellation callbacks remain occupied until settlement',async()=>{
  const f=fixture(),held=deferred();let invoked=0;const registration=f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent(){invoked++;return held.promise;}});
  const waiting=[];for(let i=0;i<8;i++)waiting.push(f.manager.open('A',Uri.parse(`preview-test:/${i}`)).catch(error=>error.message));await until(()=>invoked===8);
  registration.dispose();f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=> 'new'});
  await assert.rejects(f.manager.open('A',Uri.parse('preview-test:/extra')),/callback limit/);assert.equal(f.calls.length,0);
  held.resolve('ignored');await Promise.all(waiting);await f.manager.open('A',Uri.parse('preview-test:/extra'));assert.equal(f.calls.length,1);
});
test('oversized and invalid output reject before any native request',async()=>{
  for(const value of ['x'.repeat(1024*1024+1),'猫'.repeat(400000),{},42]){
    const f=fixture();f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=>value});
    await assert.rejects(f.manager.open('A',uri),/1 MiB|string/);assert.equal(f.calls.length,0);
  }
  const f=fixture();await assert.rejects(f.manager.open('A',Uri.parse('preview-test:/'+'x'.repeat(4096))),/URI/);
});
test('ack without the matching current native mirror never returns a fabricated document',async()=>{
  const f=fixture(()=>({document:1}));f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=> 'x'});
  await assert.rejects(f.manager.open('A',uri),/mirror/);
});
test('registration getters cannot retire/reenter admission; event cleanup retains original receiver',()=>{
  const f=fixture();let receiver,disposed=0;const subscription={dispose(){receiver=this;disposed++;}};
  const registration=f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=>'',onDidChange(){return subscription;}});registration.dispose();assert.equal(receiver,subscription);assert.equal(disposed,1);
  assert.throws(()=>f.api.registerTextDocumentContentProvider('preview-test',{get provideTextDocumentContent(){f.retire();return()=>'';}}),/retired/);assert.equal(f.manager.snapshot().providers.length,0);
});
test('real provider caller context owns Thenable assimilation, events and cleanup across owner delegation',async()=>{
  const f=fixture(),callers=createCallerContext(()=>{});f.options.captureCaller=owner=>callers.bind(owner,()=>{});
  Object.assign(f.options,{unowned:callers.unowned,bindProvider:callers.bindProvider,bindProviderResult:callers.bindProviderResult,invokeProviderResult:callers.invokeProviderResult,bindProviderCleanup:callers.bindProviderCleanup});
  const manager=createTextContentProviders(f.options),seen=[];let raw;
  raw={provideTextDocumentContent(){seen.push(['callback',callers.currentOwner(),this===raw]);return {get then(){seen.push(['then',callers.currentOwner()]);return resolve=>resolve('x');}};}};
  const registration=callers.run('provider',()=>manager.forOwner('provider').registerTextDocumentContentProvider('preview-test',raw));
  await callers.run('consumer',()=>manager.open('consumer',uri));assert.deepEqual(seen,[['callback','provider',true],['then','provider']]);registration.dispose();
});
test('provider getter disposal prevents callback invocation and native admission',async()=>{
  const f=fixture();let registration,invoked=0,reads=0;
  registration=f.api.registerTextDocumentContentProvider('preview-test',{get provideTextDocumentContent(){if(++reads>1)registration.dispose();return()=>{invoked++;return 'x';};}});
  await assert.rejects(f.manager.open('A',uri),/disposed/);assert.equal(invoked,0);assert.equal(f.calls.length,0);
});
test('selected facade native roundtrip publishes stable host document and open/change events before ACK',async()=>{
  const notifications=[],frames=[];let runtime,generation=0,version=0,text='';const resource=uri.toString();
  function publish(){runtime.sync({generation:++generation,active:1,selections:[],documents:[{id:1,uri:resource,version,text,isReadonly:true,languageId:'cpp',isDirty:false}]});}
  runtime=createApi((method,params)=>{frames.push({method,params});version++;text=params.args.content;publish();return Promise.resolve({document:1});},(method,params)=>notifications.push({method,params}),{session:7,virtualDocuments:true});
  runtime.configure('/tmp/workspace',[],[{},{}]);const provider=runtime.forExtension('P'),consumer=runtime.forExtension('C'),events=new EventEmitter();let supplied='猫🙂\r\n';const seen=[];
  provider.workspace.registerTextDocumentContentProvider('preview-test',{onDidChange:events.event,provideTextDocumentContent:()=>supplied});
  consumer.workspace.onDidOpenTextDocument(doc=>seen.push(['open',doc]));consumer.workspace.onDidChangeTextDocument(event=>seen.push(['change',event.document]));
  const first=await consumer.workspace.openTextDocument(uri);assert.equal(first.getText(),'猫🙂\n');assert.equal(first.languageId,'cpp');assert.equal(seen[0][1],first);
  assert.equal(frames[0].params.owner,'C');assert.equal(frames[0].params.args.providerOwner,'P');assert.equal(notifications[0].method,'textContentProviders');
  supplied='updated';events.fire(uri);await until(()=>seen.length===2);assert.equal(first.getText(),'updated');assert.equal(seen[1][1],first);assert.equal(await consumer.workspace.openTextDocument(uri),first);assert.equal(frames.length,2);
});
test('capability is gated; virtual registry and callback limits remain explicit',()=>{
  const runtime=createApi(()=>{},()=>{});assert.throws(()=>runtime.forExtension('P').workspace.registerTextDocumentContentProvider,/not implemented/);
  const f=fixture();for(let i=0;i<64;i++)f.api.registerTextDocumentContentProvider(`scheme-${i}`,{provideTextDocumentContent:()=>''});
  assert.throws(()=>f.api.registerTextDocumentContentProvider('extra',{provideTextDocumentContent:()=>''}),/registration limit/);assert.equal(f.manager.snapshot().providers.length,64);
});
test('URI getter retirement refuses before work and URI formatter overrides cannot retarget',async()=>{
  const f=fixture();f.api.registerTextDocumentContentProvider('preview-test',{provideTextDocumentContent:()=> 'original'});
  const input=Uri.parse('preview-test:/exact.cpp');let formatted=0;
  Object.defineProperty(input,'toString',{value(){formatted++;return 'preview-test:/retarget.cpp';}});
  Object.defineProperty(input,'toJSON',{value(){formatted++;throw new Error('must not serialize extension URI');}});
  await f.manager.open('A',input);assert.equal(f.calls[0].args.uri,'preview-test:/exact.cpp');assert.equal(formatted,0);
  const stale=Uri.parse('preview-test:/stale.cpp');Object.defineProperty(stale,'path',{get(){f.retire();return '/stale.cpp';}});
  await assert.rejects(f.manager.open('A',stale),/retired/);assert.equal(f.calls.length,1);
});
test('raw subscription cleanup runs after provider retirement without reacquiring its new lease',()=>{
  const f=fixture(),callers=createCallerContext(()=>{}),seen=[];f.options.captureCaller=owner=>callers.bind(owner,()=>{});
  Object.assign(f.options,{unowned:callers.unowned,bindProvider:callers.bindProvider,bindProviderResult:callers.bindProviderResult,invokeProviderResult:callers.invokeProviderResult,bindProviderCleanup:callers.bindProviderCleanup});
  const manager=createTextContentProviders(f.options);let subscription;
  const raw={provideTextDocumentContent:()=>'',onDidChange(){subscription={dispose(){seen.push(this===subscription);assert.throws(()=>manager.forOwner('P').registerTextDocumentContentProvider('renewed',{provideTextDocumentContent:()=>''}),/retired/);}};return subscription;}};
  const registration=callers.run('P',()=>manager.forOwner('P').registerTextDocumentContentProvider('preview-test',raw));callers.retire('P');callers.run('P',()=>{});
  callers.unowned(()=>registration.dispose());assert.deepEqual(seen,[true]);assert.equal(manager.snapshot().providers.length,0);
});
