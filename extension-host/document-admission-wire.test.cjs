'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {createNativeRequests}=require('./native-requests.cjs');
const {createDocumentServices}=require('./document-services.cjs');
for(const method of ['nativeDocumentLoadBegin','nativeDocumentLoadCancel'])test(`${method} retains capacity until its original typed terminal reply`,async()=>{
  let next=0;const timers=new Map(),frames=[];
  const clock={setTimeout(fn){timers.set(++next,fn);return next;},clearTimeout(id){timers.delete(id);}};
  const queue=createNativeRequests(frame=>frames.push(frame),()=>({id:9,owner:'origin',active:true}),clock);
  const services=createDocumentServices(queue.request,{session:7},{generation:()=>0,customerRequest:queue.request});
  const pending=Array.from({length:8},()=>services.call('owner',method,{load:'original',kind:'virtual',uri:'preview:/a'}).catch(error=>error));
  const fire=timers.get(1);fire();await pending[0];
  await assert.rejects(services.call('owner',method,{}),/queue limit/);
  for(const result of [null,true,{}, {load:''}, {load:7}, {canceled:false}, {load:'x',extra:true}])assert.equal(queue.settle({id:frames[0].id,result}),false);
  assert.equal(queue.settle({id:frames[0].id,result:method==='nativeDocumentLoadBegin'?{load:'original'}:{canceled:true}}),true);
  assert.equal(queue.settle({id:frames[0].id,result:{canceled:true}}),false);
  queue.shutdown();await Promise.all(pending);
});
