'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const { createApi } = require('./api.cjs');
const { Uri } = require('./api-types.cjs');
const { createNativeRequests } = require('./native-requests.cjs');
function state(generation = 1, patch = {}) {
  return { generation, documents: [{id:7,uri:Uri.file('/tmp/original.txt').toString(),version:1,
    text:'α😀\r\nnext',languageId:'plaintext',isDirty:true,savedGeneration:0,...patch}], active:7,
    selections:[{anchor:{line:0,character:0},active:{line:0,character:0}}] };
}
function fixture(request = async()=>false) {
  const runtime = createApi(request, ()=>{}, {session:19}); runtime.sync(state());
  return {runtime, document:runtime.forExtension('test.first').workspace.textDocuments[0]};
}
test('shared save captures invoking caller/version immediately and mirror/didSave precede true', async()=>{
  let ack, request;
  const {runtime,document} = fixture((method,params)=>{request={method,params};return new Promise(resolve=>ack=resolve);});
  const events=[];
  runtime.forExtension('test.second').workspace.onDidSaveTextDocument(saved=>{
    assert.equal(saved,document);assert.equal(saved.isDirty,true);events.push('didSave');
  });
  const saving=runtime.runWithOwner('test.second',()=>document.save().then(result=>{events.push('resolved');return result;}));
  assert.equal(request.method,'nativeDocumentSave');assert.equal(request.params.owner,'test.second');
  assert.deepEqual(request.params.args,{document:7,version:1});
  runtime.sync(state(2,{version:2,text:'newer α😀\r\nnext',isDirty:true,savedGeneration:1}));
  ack(true);assert.equal(await saving,true);assert.deepEqual(events,['didSave','resolved']);
  assert.equal(document.getText(),'newer α😀\r\nnext');assert(document.isDirty);
});
test('successful acknowledgement without published saved mirror rejects; false changes nothing',async()=>{
  const {runtime,document}=fixture(async()=>true);
  await assert.rejects(runtime.runWithOwner('test.first',()=>document.save()),/mirror.*before/);
  const second=fixture(async()=>false);
  assert.equal(await second.runtime.runWithOwner('test.first',()=>second.document.save()),false);
  assert(second.document.isDirty);assert.equal(second.document.version,1);
});
test('unowned, closed, and retired caller descendants cannot acquire a save lease',async()=>{
  let calls=0;const {runtime,document}=fixture(async()=>{calls++;return false;});
  await assert.rejects(document.save(),/ownership/);
  let late;
  const timer=runtime.runWithOwner('test.first',()=>new Promise(resolve=>{
    late=()=>document.save().then(()=>resolve(null),error=>resolve(error));
  }));
  runtime.disposeOwner('test.first');runtime.forExtension('test.first');
  await runtime.runWithOwner('test.other',async()=>{ /* live unrelated owner */ });
  // The real scheduled descendant retains the old ALS lease, not the facade.
  const {AsyncResource}=require('node:async_hooks');
  let resource;
  runtime.runWithOwner('test.first',()=>resource=new AsyncResource('save-test'));
  runtime.disposeOwner('test.first');runtime.forExtension('test.first');
  resource.runInAsyncScope(late);assert.match(String(await timer),/retired/);resource.emitDestroy();
  runtime.sync({generation:2,documents:[],active:null,selections:[]});
  await assert.rejects(runtime.runWithOwner('test.other',()=>document.save()),/closed/);assert.equal(calls,0);
});
test('save helpers stay private and the existing shared eight-service occupancy survives timed out save',async()=>{
  const held=[];const {runtime,document}=fixture((method,params,lifecycle)=>{
    if(lifecycle)lifecycle.retained=true;
    return new Promise((resolve,reject)=>held.push({method,params,lifecycle,resolve,reject}));
  });
  const api=runtime.forExtension('test.first');assert(!Object.hasOwn(api,'currentCallerOwner'));
  assert(!Object.hasOwn(api,'saveDocument'));assert(!Object.hasOwn(api,'setDocumentSaveHook'));
  const saves=Array.from({length:8},()=>runtime.runWithOwner('test.first',()=>document.save()));
  const results=Promise.allSettled(saves);
  await assert.rejects(api.workspace.openTextDocument('/tmp/ninth'),/limit/);
  held[0].reject(Error('outcome is unknown'));await Promise.resolve();await Promise.resolve();
  await assert.rejects(runtime.runWithOwner('test.first',()=>document.save()),/limit/);
  held[0].lifecycle.onTerminal();
  const next=runtime.runWithOwner('test.first',()=>document.save());
  assert.equal(held.length,9);
  for(const entry of held.slice(1)){entry.lifecycle.onTerminal();entry.resolve(false);}
  assert.equal((await results).length,8);assert.equal(await next,false);
});

test('held terminal save rejects retired original caller and owner-reacquisition ABA without losing published mirror',async()=>{
  for(const renew of [false,true]){
    let held,terminal=0;
    const {runtime,document}=fixture((_,params,lifecycle)=>{
      lifecycle.retained=true;
      return new Promise(resolve=>held={params,resolve,lifecycle});
    });
    const saving=runtime.runWithOwner('test.first',()=>document.save());
    const refusal=assert.rejects(saving,/ownership.*retired|ownership is missing or retired/);
    runtime.disposeOwner('test.first');if(renew)runtime.forExtension('test.first');
    runtime.sync(state(2,{savedGeneration:1,isDirty:true,text:'newer α😀\r\nnext',version:2}));
    const release=held.lifecycle.onTerminal;
    held.lifecycle.onTerminal=()=>{terminal++;release();};
    held.lifecycle.onTerminal();held.resolve(true);
    await refusal;assert.equal(terminal,1);assert.equal(document._snapshot.savedGeneration,1);
    assert.equal(document.getText(),'newer α😀\r\nnext');assert(document.isDirty);
    // A genuinely new owner can still admit another save; the old actual
    // request terminal released the shared service slot without revival.
    const next=runtime.runWithOwner('test.other',()=>document.save());
    held.lifecycle.onTerminal();held.resolve(false);assert.equal(await next,false);
  }
});
test('captured admission identity/version cannot alternate into another native save target',async()=>{
  let requests=[];const {runtime,document}=fixture(async(method,params)=>{requests.push(params.args);return false;});
  const snapshot=document._snapshot;let ids=0,versions=0;
  Object.defineProperty(snapshot,'id',{configurable:true,get(){return ++ids===1?7:99;}});
  Object.defineProperty(snapshot,'version',{configurable:true,get(){return ++versions===1?1:88;}});
  assert.equal(await runtime.runWithOwner('test.first',()=>document.save()),false);
  assert.equal(ids,0);assert.equal(versions,0);assert.deepEqual(requests,[{document:7,version:1}]);
  assert(requests.every(request=>request.document!==99),'changing ID must never send target B');
});
test('extension-owned metadata getters are never used for save admission or publication',async()=>{
  let calls=0,reads=0;const first=fixture(async()=>{calls++;return false;});
  Object.defineProperty(first.document._snapshot,'version',{configurable:true,get(){
    reads++;first.runtime.disposeOwner('test.first');first.runtime.forExtension('test.first');return 1;
  }});
  assert.equal(await first.runtime.runWithOwner('test.first',()=>first.document.save()),false);
  assert.equal(calls,1);assert.equal(reads,0);
  let ack;const second=fixture(()=>new Promise(resolve=>ack=resolve));
  const saving=second.runtime.runWithOwner('test.first',()=>second.document.save());
  second.runtime.sync(state(2,{savedGeneration:1,isDirty:false}));
  Object.defineProperty(second.document._snapshot,'savedGeneration',{configurable:true,get(){
    reads++;second.runtime.disposeOwner('test.first');second.runtime.forExtension('test.first');return 1;
  }});
  ack(true);assert.equal(await saving,true);assert.equal(reads,0);
  assert.equal(second.document.getText(),'α😀\r\nnext');assert.equal(second.document.isDirty,false);
});

function clock() { const timers=[];return {timers,setTimeout(call,delay){const timer={call,delay,cleared:false};timers.push(timer);return timer;},clearTimeout(timer){timer.cleared=true;}}; }
test('save timeout rejects unknown but wrong/malformed replies cannot release occupied capacity',async()=>{
  const sent=[],time=clock();let terminals=0;
  const queue=createNativeRequests(frame=>sent.push(frame),()=>({id:91,owner:'origin.a',active:true}),time);
  const lifecycle={onTerminal:()=>terminals++};
  const promise=queue.request('nativeDocumentSave',{owner:'callee.b',session:1,args:{document:7,version:1}},lifecycle);
  const failure=assert.rejects(promise,/outcome is unknown/);assert(lifecycle.retained);
  assert.equal(sent[0].params.owner,'callee.b');assert.equal(sent[0].params.commandOwner,'origin.a');
  assert.equal(time.timers[0].delay,360000);time.timers[0].call();await failure;
  assert.equal(queue.settle({id:'other',result:true}),false);
  assert.equal(queue.settle({id:sent[0].id,result:null}),false);
  assert.equal(queue.settle({id:sent[0].id,result:true,error:{message:'bad'}}),false);
  assert.equal(terminals,0);
  assert.equal(queue.settle({id:sent[0].id,result:true}),true);assert.equal(terminals,1);
  assert.equal(queue.settle({id:sent[0].id,result:true}),false);assert.equal(terminals,1);
});
test('completed command timers keep extension ownership without borrowing command origin',async()=>{
  const frames=[],time=clock(),origin={id:9,owner:'a',active:false};
  const queue=createNativeRequests(frame=>frames.push(frame),()=>origin,time);
  const result=queue.request('nativeDocumentSave',{owner:'a'},{});
  assert(!Object.hasOwn(frames[0].params,'command'));
  queue.settle({id:frames[0].id,result:false});assert.equal(await result,false);
});
test('bounded retained requests refuse admission until actual terminal and shutdown releases exactly once',async()=>{
  const sent=[],time=clock(),queue=createNativeRequests(frame=>sent.push(frame),()=>undefined,time);
  let releases=0;const requests=Array.from({length:64},()=>queue.request('nativeDocumentSave',{}, {onTerminal:()=>releases++}));
  const settled=Promise.allSettled(requests);
  for(const timer of time.timers)timer.call();
  await settled;await assert.rejects(queue.request('nativeDocumentSave',{}),/limit/);
  assert.equal(sent.length,64);assert.equal(releases,0);queue.shutdown();assert.equal(releases,64);
  queue.shutdown();assert.equal(releases,64);
});

function framed(message) {
  const body=Buffer.from(JSON.stringify(message));return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`),body]);
}
test('real host preserves cross-owner command origin, timer/event/provider save owners and didSave ordering', {timeout:20000},async t=>{
  const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
  const {spawn}=require('node:child_process'),{once}=require('node:events');
  const folder=fs.mkdtempSync(path.join(os.tmpdir(),'vscli-save-host-'));
  const extensions=[];
  for(const name of ['a','b']){
    const dir=path.join(folder,name);fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir,'package.json'),JSON.stringify({publisher:'save',name,version:'1',main:'extension.cjs'}));
    fs.writeFileSync(path.join(dir,'extension.cjs'),name==='a'?`
      const v=require('vscode');exports.activate=c=>c.subscriptions.push(v.commands.registerCommand('save.bridge',()=>v.commands.executeCommand('save.shared')));
    `:`
      const v=require('vscode');let events=0,armed=false;const doc=()=>v.workspace.textDocuments[0];
      exports.activate=c=>{
        c.subscriptions.push(v.workspace.onDidSaveTextDocument(d=>{if(d!==doc())throw Error('identity');events++;}));
        c.subscriptions.push(v.commands.registerCommand('save.shared',async()=>{const d=doc();const saved=await d.save();return {saved,events,dirty:d.isDirty,text:d.getText()};}));
        c.subscriptions.push(v.commands.registerCommand('save.timer',()=>{setTimeout(()=>doc().save().catch(console.error),0);}));
        c.subscriptions.push(v.commands.registerCommand('save.armEvent',()=>{armed=true;}));
        c.subscriptions.push(v.workspace.onDidChangeTextDocument(e=>{if(armed){armed=false;e.document.save().catch(console.error);}}));
        c.subscriptions.push(v.languages.registerDocumentFormattingEditProvider('plaintext',{
          marker:17,async provideDocumentFormattingEdits(d){if(this.marker!==17||d!==doc())throw Error('provider receiver/identity');await d.save();return [];}
        }));
      };
    `);
    extensions.push({id:`save.${name}`,version:'1',path:dir});
  }
  const child=spawn(process.execPath,[path.join(__dirname,'host.cjs')],{stdio:['pipe','pipe','pipe']});
  let buffer=Buffer.alloc(0),stderr='',failure;const messages=[],waiters=new Set();
  const wake=()=>{for(const fn of [...waiters])fn();};
  child.stderr.on('data',chunk=>{stderr=(stderr+chunk).slice(-32768);});
  child.on('error',error=>{failure=error;wake();});child.on('exit',code=>{failure=Error(`host exit ${code}: ${stderr}`);wake();});
  child.stdout.on('data',chunk=>{
    buffer=Buffer.concat([buffer,chunk]);if(buffer.length>2*1024*1024){failure=Error('test frame budget');child.kill();wake();return;}
    for(;;){const end=buffer.indexOf('\r\n\r\n');if(end<0)break;
      const length=Number(buffer.subarray(0,end).toString().split(':')[1]);if(buffer.length<end+4+length)break;
      messages.push(JSON.parse(buffer.subarray(end+4,end+4+length)));buffer=buffer.subarray(end+4+length);
      if(messages.length>128){failure=Error('test messages budget');child.kill();break;}
    }wake();
  });
  t.after(async()=>{if(child.exitCode===null&&child.signalCode===null){const ended=once(child,'exit');child.kill();await ended;}fs.rmSync(folder,{recursive:true,force:true});});
  function receive(predicate){return new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{waiters.delete(check);reject(Error(`no frame: ${stderr}`));},5000);
    const check=()=>{const i=messages.findIndex(predicate);if(i>=0||failure){clearTimeout(timer);waiters.delete(check);i>=0?resolve(messages.splice(i,1)[0]):reject(failure);}};
    waiters.add(check);check();
  });}
  const send=message=>child.stdin.write(framed(message));
  let current=state();current.documents[0].uri=Uri.file(path.join(folder,'original.txt')).toString();
  send({id:1,method:'initialize',params:{protocol:4,session:19,extensions,root:folder,state:current,configuration:[{},{}],languageProviders:true}});
  const initialized=await receive(m=>m.id===1);assert(!initialized.error,JSON.stringify(initialized));
  const provider=initialized.result.languageProviders.find(p=>p.owner==='save.b'&&p.type==='formatting');assert(provider);
  send({id:9,method:'execute',params:{session:19,owner:'save.a',command:'save.bridge',args:[]}});
  let save=await receive(m=>m.method==='nativeDocumentSave');assert.equal(save.params.owner,'save.b');
  assert.equal(save.params.command,9);assert.equal(save.params.commandOwner,'save.a');assert.deepEqual(save.params.args,{document:7,version:1});
  current={...current,generation:2,documents:[{...current.documents[0],version:2,text:'newer α😀\r\nnext',isDirty:true,savedGeneration:1}]};
  send({method:'state',params:current});send({id:save.id,result:true});
  const reply=await receive(m=>m.id===9);assert(!reply.error,JSON.stringify(reply));
  assert.deepEqual(reply.result,{saved:true,events:1,dirty:true,text:'newer α😀\r\nnext'});
  send({id:10,method:'execute',params:{session:19,owner:'save.b',command:'save.timer',args:[]}});
  await receive(m=>m.id===10);save=await receive(m=>m.method==='nativeDocumentSave');
  assert.equal(save.params.owner,'save.b');assert(!Object.hasOwn(save.params,'command'));
  current={...current,generation:3,documents:[{...current.documents[0],savedGeneration:2}]};
  send({method:'state',params:current});send({id:save.id,result:true});
  send({id:20,method:'provideLanguage',params:{session:19,owner:'save.b',provider:provider.id,document:7,version:2,position:{line:0,character:0},options:{tabSize:4,insertSpaces:true}}});
  save=await receive(m=>m.method==='nativeDocumentSave');assert.equal(save.params.owner,'save.b');assert.equal(save.params.command,20);
  current={...current,generation:4,documents:[{...current.documents[0],savedGeneration:3}]};
  send({method:'state',params:current});send({id:save.id,result:true});
  const formatted=await receive(m=>m.id===20);assert(!formatted.error,JSON.stringify(formatted));assert.deepEqual(formatted.result,[]);
  send({id:30,method:'execute',params:{session:19,owner:'save.b',command:'save.armEvent',args:[]}});await receive(m=>m.id===30);
  current={...current,generation:5,documents:[{...current.documents[0],version:3,text:'event α😀\r\nnext'}]};send({method:'state',params:current});
  save=await receive(m=>m.method==='nativeDocumentSave');assert.equal(save.params.owner,'save.b');assert(!Object.hasOwn(save.params,'command'));
  assert.equal(save.params.args.version,3);send({id:save.id,result:false});
  // Metadata-only clean state; unchanged document/version/source and actual
  // host requests/terminal frames qualify repeated didSave-before-true.
  current={...current,generation:6,documents:[{...current.documents[0],isDirty:false}]};
  send({method:'state',params:current});
  const cleanText=current.documents[0].text;
  for(const [command,publication,eventCount] of [[40,4,4],[41,5,5]]){
    send({id:command,method:'execute',params:{session:19,owner:'save.b',command:'save.shared',args:[]}});
    save=await receive(m=>m.method==='nativeDocumentSave');
    assert.equal(save.params.owner,'save.b');assert.equal(save.params.command,command);
    assert.deepEqual(save.params.args,{document:7,version:3});
    assert.equal(current.documents[0].savedGeneration,publication-1);
    current={...current,generation:current.generation+1,documents:[{...current.documents[0],savedGeneration:publication}]};
    send({method:'state',params:current});send({id:save.id,result:true});
    const clean=await receive(m=>m.id===command);assert(!clean.error,JSON.stringify(clean));
    assert.deepEqual(clean.result,{saved:true,events:eventCount,dirty:false,text:cleanText});
    assert.equal(current.documents[0].version,3);assert.equal(current.documents[0].isDirty,false);
  }
});
