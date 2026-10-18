"use strict";
const {test}=require('node:test'),assert=require('node:assert/strict');
const {createApi}=require('./api.cjs'),{createNativeRequests}=require('./native-requests.cjs');
function fixture(handler,enabled=true) {
 const calls=[];const runtime=createApi((method,params)=>{calls.push({method,params});return handler(method,params);},()=>{},{session:17,clipboard:enabled});
 runtime.sync({generation:1,documents:[],active:null});return {runtime,api:runtime.forExtension('fixture.clipboard'),calls};
}
test('clipboard is frozen and owner-scoped; native Unicode CRLF reads and exact void writes do not mutate documents',async()=>{
 const text='猫🙂\r\nsecond\r\n';const f=fixture(method=>Promise.resolve(method==='nativeClipboardRead'?text:null));
 assert(Object.isFrozen(f.api.env.clipboard));assert.equal(f.api.env.clipboard,f.api.env.clipboard);
 assert.equal(await f.api.env.clipboard.readText(),text);assert.equal(await f.api.env.clipboard.writeText(text),undefined);
 assert.deepEqual(f.calls,[{method:'nativeClipboardRead',params:{session:17,owner:'fixture.clipboard',generation:1,args:null}},{method:'nativeClipboardWrite',params:{session:17,owner:'fixture.clipboard',generation:1,args:{text}}}]);
 assert.equal(f.api.workspace.textDocuments.length,0);assert.equal(f.api.env.appName,'VSCLI');
});
test('retained clipboard handles keep original caller leases across replacement and asynchronous replies',async()=>{
 let resolve;const f=fixture(()=>new Promise(done=>{resolve=done;}));const old=f.api.env.clipboard;
 const pending=old.readText();f.runtime.disposeOwner('fixture.clipboard');const newer=f.runtime.forExtension('fixture.clipboard').env.clipboard;
 resolve('late');await assert.rejects(pending,/retired/);await assert.rejects(old.writeText('forbidden'),/retired/);assert.equal(f.calls.length,1);
 assert.notEqual(old,newer);const read=newer.readText();resolve('new');assert.equal(await read,'new');
});
test('clipboard bounds and unavailable capability refuse before native work; malformed and unavailable replies reject',async()=>{
 const f=fixture(()=>Promise.resolve(null));for(const text of [undefined,null,3,{},'\ud800','猫'.repeat(349526)])await assert.rejects(f.api.env.clipboard.writeText(text));
 assert.equal(f.calls.length,0);const disabled=fixture(()=>Promise.resolve(null),false);await assert.rejects(disabled.api.env.clipboard.readText(),/not implemented/);assert.equal(disabled.calls.length,0);
 const badUtf16=fixture(()=>Promise.resolve('\ud800'));await assert.rejects(badUtf16.api.env.clipboard.readText(),/receipt/);
 const badRead=fixture(()=>Promise.resolve(false));await assert.rejects(badRead.api.env.clipboard.readText(),/receipt/);
 const hugeRead=fixture(()=>Promise.resolve('x'.repeat(1048577)));await assert.rejects(hugeRead.api.env.clipboard.readText(),/receipt/);
 const badWrite=fixture(()=>Promise.resolve(true));await assert.rejects(badWrite.api.env.clipboard.writeText('x'),/receipt/);
 const unavailable=fixture(()=>Promise.reject(new Error('System clipboard unavailable')));await assert.rejects(unavailable.api.env.clipboard.writeText('x'),/unavailable/);
});
test('clipboard requests share eight retained service slots and do not free them on a logical timeout',async()=>{
 const releases=[],f=fixture(()=>new Promise(done=>releases.push(done)));const requests=Array.from({length:8},()=>f.api.env.clipboard.readText());
 await assert.rejects(f.api.env.clipboard.writeText('ninth'),/queue limit/);assert.equal(f.calls.length,8);
 releases.forEach(done=>done('shared'));assert.deepEqual(await Promise.all(requests),Array(8).fill('shared'));
});
test('clipboard framing validates exact bounded string or void receipts and retains actual timeout capacity',async()=>{
 for(const method of ['nativeClipboardRead','nativeClipboardWrite']) {
  let timeout,delay,released=0;const frames=[];
  const requests=createNativeRequests(frame=>frames.push(frame),()=>({active:true,id:3,owner:'fixture.clipboard'}),{setTimeout(callback,ms){timeout=callback;delay=ms;return 1;},clearTimeout(){}});
  const lifecycle={onTerminal(){released++;}};const pending=requests.request(method,{session:17,owner:'fixture.clipboard',args:method.endsWith('Read')?null:{text:'cat'}},lifecycle);
  assert.equal(delay,5000);assert.equal(lifecycle.retained,true);assert.equal(frames[0].params.command,3);
  const timed=assert.rejects(pending,/outcome is unknown/);timeout();await timed;assert.equal(released,0);
  for(const bad of [{result:false},{},...(method.endsWith('Read')?[{result:'\ud800'}]:[]),{result:method.endsWith('Read')?'x'.repeat(1048577):'wrong'},{result:null,error:{message:'both'}},{error:{message:'x'.repeat(4097)}}]) {
   assert.equal(requests.settle({id:frames[0].id,...bad}),false);assert.equal(released,0);
  }
  assert.equal(requests.settle({id:frames[0].id,result:method.endsWith('Read')?'猫🙂\r\n':null}),true);assert.equal(released,1);
 }
});
